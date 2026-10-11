//! Standalone diagnostic copy; build and runtime evidence is archived separately.
//! Direct winit IME diagnostic, independent of lumine editors and rendering.
//! `--fixed` keeps candidate geometry fixed; default/`--moving` follows preedit
//! character count. `--seconds N` sets the lifetime (default 15 seconds).
//! `--cursor-delay-ms N` coalesces updates behind a 0–1000 ms deadline (default 0).
//! `--preedit-block-ms 0|40|80` blocks the callback thread (default 0).
//! `--preedit-block-position before-cursor|after-cursor` selects ordering.
//! `--present-on-ime` optionally presents the white surface during preedit.
//! This isolates event-loop stalls and request order, not GPU rendering cost.
use std::{
    num::NonZeroU32,
    rc::Rc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize},
    event::{Ime, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

struct Probe {
    fixed: bool,
    duration: Duration,
    deadline: Option<Instant>,
    started: Instant,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    context: Option<softbuffer::Context<Rc<Window>>>,
    committed: String,
    preedit: String,
    focused: bool,
    rectangle: Option<i32>,
    cursor_delay: Duration,
    preedit_block: Duration,
    block_before_cursor: bool,
    present_on_ime: bool,
    pending_rectangle: Option<(Instant, i32)>,
}
impl Probe {
    fn cursor(&mut self) {
        let x = if self.fixed {
            40
        } else {
            40 + self.preedit.chars().count().min(40) as i32 * 14
        };
        if self.rectangle == Some(x) {
            self.pending_rectangle = None;
            return;
        }
        println!(
            "RECT_REQUEST x={x} delay_ms={} elapsed={:.6}",
            self.cursor_delay.as_millis(),
            self.started.elapsed().as_secs_f64()
        );
        if self.rectangle.is_none() || self.cursor_delay.is_zero() {
            self.pending_rectangle = None;
            self.apply_cursor(x);
        } else {
            let deadline = self
                .pending_rectangle
                .map_or_else(|| Instant::now() + self.cursor_delay, |pending| pending.0);
            self.pending_rectangle = Some((deadline, x));
        }
    }
    fn apply_cursor(&mut self, x: i32) {
        let Some(window) = &self.window else {
            return;
        };
        if self.rectangle == Some(x) {
            return;
        }
        window.set_ime_cursor_area(LogicalPosition::new(x, 80), LogicalSize::new(1, 24));
        self.rectangle = Some(x);
        println!(
            "RECT x={x} y=80 width=1 height=24 elapsed={:.6}",
            self.started.elapsed().as_secs_f64()
        );
    }
    fn block_preedit(&self, position: &str) {
        if self.preedit_block.is_zero() {
            return;
        }
        let began = Instant::now();
        println!(
            "BLOCK_BEGIN position={position} requested_ms={} preedit={:?} elapsed={:.6}",
            self.preedit_block.as_millis(),
            self.preedit,
            self.started.elapsed().as_secs_f64()
        );
        // Deliberately synchronous: unlike cursor_delay, this prevents winit
        // from dispatching queued native events or returning to its flush loop.
        std::thread::sleep(self.preedit_block);
        println!(
            "BLOCK_END position={position} observed_ms={:.3} elapsed={:.6}",
            began.elapsed().as_secs_f64() * 1000.,
            self.started.elapsed().as_secs_f64()
        );
    }
    fn present_white(&mut self, stage: &str) {
        let size = self.window.as_ref().unwrap().inner_size();
        if let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        {
            let began = Instant::now();
            println!(
                "PRESENT_BEGIN stage={stage} elapsed={:.6}",
                self.started.elapsed().as_secs_f64()
            );
            let surface = self.surface.as_mut().unwrap();
            surface
                .resize(width, height)
                .expect("resize software surface");
            let mut buffer = surface.buffer_mut().expect("map software surface");
            buffer.fill(0x00ffffff);
            buffer.present().expect("present software surface");
            println!(
                "PRESENT_END stage={stage} observed_ms={:.3} elapsed={:.6}",
                began.elapsed().as_secs_f64() * 1000.,
                self.started.elapsed().as_secs_f64()
            );
        }
    }
    fn finish(&self) {
        println!(
            "FINAL fixed={} committed={:?} preedit={:?}",
            self.fixed, self.committed, self.preedit
        );
    }
}
impl ApplicationHandler for Probe {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Rc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("lumine direct winit IME probe")
                        .with_inner_size(LogicalSize::new(640., 240.)),
                )
                .expect("create diagnostic window"),
        );
        let context = softbuffer::Context::new(window.clone()).expect("create software context");
        let surface =
            softbuffer::Surface::new(&context, window.clone()).expect("create software surface");
        self.context = Some(context);
        self.surface = Some(surface);
        self.window = Some(window.clone());
        self.deadline = Some(Instant::now() + self.duration);
        println!("READY mode={}", if self.fixed { "fixed" } else { "moving" });
        println!(
            "CONFIG preedit_block_ms={} preedit_block_position={} present_on_ime={}",
            self.preedit_block.as_millis(),
            if self.block_before_cursor {
                "before-cursor"
            } else {
                "after-cursor"
            },
            self.present_on_ime
        );
        window.request_redraw();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.finish();
                event_loop.exit();
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.pending_rectangle = None;
                println!("FOCUS {focused}");
                self.window.as_ref().unwrap().set_ime_allowed(focused);
                if focused {
                    self.rectangle = None;
                    self.cursor();
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state.is_pressed() && event.logical_key == Key::Named(NamedKey::F8) =>
            {
                self.committed.clear();
                self.preedit.clear();
                println!("RESET");
                if self.focused {
                    self.cursor();
                }
            }
            WindowEvent::Ime(event) => {
                let is_preedit = matches!(&event, Ime::Preedit(..));
                println!(
                    "IME {event:?} elapsed={:.6}",
                    self.started.elapsed().as_secs_f64()
                );
                match event {
                    Ime::Preedit(text, _) => self.preedit = text,
                    Ime::Commit(text) => {
                        self.committed.push_str(&text);
                        self.preedit.clear();
                    }
                    _ => {}
                }
                if self.focused {
                    if is_preedit && self.block_before_cursor {
                        self.block_preedit("before-cursor");
                        if self.present_on_ime {
                            self.present_white("ime-before-cursor");
                        }
                    }
                    self.cursor();
                    if is_preedit && !self.block_before_cursor {
                        self.block_preedit("after-cursor");
                        if self.present_on_ime {
                            self.present_white("ime-after-cursor");
                        }
                    }
                }
            }
            WindowEvent::Resized(_) => self.window.as_ref().unwrap().request_redraw(),
            WindowEvent::RedrawRequested => self.present_white("redraw"),
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if let Some((deadline, x)) = self.pending_rectangle
            && now >= deadline
        {
            self.pending_rectangle = None;
            if self.focused {
                self.apply_cursor(x);
            }
        }
        if let Some(deadline) = self.deadline {
            if Instant::now() >= deadline {
                self.finish();
                event_loop.exit();
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(
                    self.pending_rectangle
                        .map_or(deadline, |pending| deadline.min(pending.0)),
                ));
            }
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixed = false;
    let mut seconds = 15.;
    let mut cursor_delay_ms = 0_u64;
    let mut preedit_block_ms = 0_u64;
    let mut block_before_cursor = true;
    let mut present_on_ime = false;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--fixed" => fixed = true,
            "--moving" => fixed = false,
            "--seconds" => seconds = arguments.next().ok_or("missing seconds")?.parse::<f64>()?,
            "--preedit-block-ms" => {
                preedit_block_ms = arguments
                    .next()
                    .ok_or("missing preedit block duration")?
                    .parse::<u64>()?;
            }
            "--preedit-block-position" => {
                block_before_cursor = match arguments
                    .next()
                    .ok_or("missing preedit block position")?
                    .as_str()
                {
                    "before-cursor" => true,
                    "after-cursor" => false,
                    _ => {
                        return Err(
                            "preedit block position must be before-cursor or after-cursor".into(),
                        );
                    }
                };
            }
            "--present-on-ime" => present_on_ime = true,
            "--cursor-delay-ms" => {
                cursor_delay_ms = arguments
                    .next()
                    .ok_or("missing cursor delay")?
                    .parse::<u64>()?
            }
            _ => return Err(format!("unknown argument: {argument}").into()),
        }
    }
    if !seconds.is_finite() || !(0.1..=300.).contains(&seconds) {
        return Err("seconds must be finite and within 0.1..=300".into());
    }
    if cursor_delay_ms > 1000 {
        return Err("cursor delay must be within 0..=1000 ms".into());
    }
    if ![0, 40, 80].contains(&preedit_block_ms) {
        return Err("preedit block duration must be 0, 40 or 80 ms".into());
    }
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut Probe {
        fixed,
        duration: Duration::from_secs_f64(seconds),
        deadline: None,
        started: Instant::now(),
        window: None,
        surface: None,
        context: None,
        committed: String::new(),
        preedit: String::new(),
        focused: false,
        rectangle: None,
        cursor_delay: Duration::from_millis(cursor_delay_ms),
        preedit_block: Duration::from_millis(preedit_block_ms),
        block_before_cursor,
        present_on_ime,
        pending_rectangle: None,
    })?;
    Ok(())
}
