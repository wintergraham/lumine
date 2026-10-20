//! A UI-thread foreign pump around winit's external event-loop API. Native
//! events, redraws and lifecycle callbacks remain synchronous inside winit.
use std::time::{Duration, Instant};
use winit::{
    application::ApplicationHandler,
    error::EventLoopError,
    event_loop::EventLoop,
    platform::pump_events::{EventLoopExtPumpEvents, PumpStatus},
};

const INTERVAL: Duration = Duration::from_millis(10);

pub(crate) fn run<T: 'static, A: ApplicationHandler<T>>(
    mut event_loop: EventLoop<T>,
    app: &mut A,
    mut pump: impl FnMut(),
) -> Result<(), EventLoopError> {
    let mut due = Instant::now();
    loop {
        // Winit takes the minimum of this bound and its own ControlFlow
        // deadline. In particular, input/animation work is never delayed to
        // wait for the foreign pump, and ControlFlow::Poll remains available.
        let wait = due.saturating_duration_since(Instant::now()).min(INTERVAL);
        match event_loop.pump_app_events(Some(wait), app) {
            PumpStatus::Exit(0) => return Ok(()),
            PumpStatus::Exit(code) => return Err(EventLoopError::ExitFailure(code)),
            PumpStatus::Continue => {}
        }
        // No ApplicationHandler callback is active here. Exit is checked first
        // so CEF cannot call back into a window manager that is shutting down.
        if Instant::now() >= due {
            pump();
            // Skip missed ticks instead of a catch-up spin after slow foreign
            // work. Busy winit input/render dispatch does not multiply pumps.
            due = Instant::now() + INTERVAL;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};
    use winit::{
        event::{StartCause, WindowEvent},
        event_loop::{ActiveEventLoop, ControlFlow},
        platform::x11::EventLoopBuilderExtX11,
        window::WindowId,
    };

    #[test]
    #[ignore = "requires an X11 display; run alone with --test-threads=1"]
    fn foreign_pump_runs_on_owner_thread_between_callbacks_and_stops_on_exit() {
        struct Probe {
            active: Rc<Cell<bool>>,
            pumps: Rc<Cell<usize>>,
            iterations: usize,
            exited: Rc<Cell<bool>>,
        }
        impl ApplicationHandler<()> for Probe {
            fn new_events(&mut self, _: &ActiveEventLoop, _: StartCause) {
                self.active.set(true);
                self.iterations += 1;
                self.active.set(false);
            }
            fn resumed(&mut self, _: &ActiveEventLoop) {}
            fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
            fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
                self.active.set(true);
                if self.pumps.get() == 3 {
                    event_loop.exit();
                } else {
                    // An application deadline shorter than the CEF interval
                    // must keep waking normally inside the external loop.
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        Instant::now() + Duration::from_millis(1),
                    ));
                }
                self.active.set(false);
            }
            fn exiting(&mut self, _: &ActiveEventLoop) {
                self.exited.set(true);
            }
        }
        let mut builder = EventLoop::<()>::with_user_event();
        builder.with_x11().with_any_thread(true);
        let event_loop = builder.build().unwrap();
        let owner = std::thread::current().id();
        let active = Rc::new(Cell::new(false));
        let pumps = Rc::new(Cell::new(0));
        let exited = Rc::new(Cell::new(false));
        let mut app = Probe {
            active: active.clone(),
            pumps: pumps.clone(),
            iterations: 0,
            exited: exited.clone(),
        };
        run(event_loop, &mut app, || {
            assert_eq!(std::thread::current().id(), owner);
            assert!(!active.get());
            assert!(!exited.get());
            pumps.set(pumps.get() + 1);
        })
        .unwrap();
        assert!(exited.get());
        assert_eq!(pumps.get(), 3);
        assert!(app.iterations > pumps.get());
    }
}
