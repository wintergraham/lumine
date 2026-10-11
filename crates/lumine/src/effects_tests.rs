use super::*;
use crate::{
    compose::{ViewHandle, component, div, provide},
    style::Styled,
    task::LocalExecutor,
    widgets::Ui,
};

struct Harness {
    _ui: Ui,
    root: ViewHandle,
    executor: Rc<RefCell<LocalExecutor>>,
    frames: FrameClock,
    clocks: Vec<EffectClock>,
    policy: MotionPolicy,
    start: Instant,
}
impl Harness {
    fn new(options: &[EffectOptions]) -> Self {
        let executor = Rc::new(RefCell::new(LocalExecutor::new()));
        let frames = FrameClock::new();
        let mut ui = Ui::new(100.0, 100.0);
        let policy = MotionPolicy {
            active: ui.signal(true),
            reduced: ui.signal(false),
        };
        let output = Rc::new(RefCell::new(None));
        let capture = output.clone();
        let options = options.to_vec();
        let root = ui.mount(provide(
            TaskRunner::from_executor(executor.clone()),
            provide(
                frames.clone(),
                provide(
                    policy.clone(),
                    component(move |cx| {
                        let scheduler = EffectScheduler::new(cx);
                        let clocks = options
                            .iter()
                            .map(|options| scheduler.clock(cx, *options))
                            .collect::<Vec<_>>();
                        *capture.borrow_mut() = Some(clocks);
                        div().size(100.0, 100.0)
                    }),
                ),
            ),
        ));
        let clocks = output.borrow_mut().take().unwrap();
        Self {
            _ui: ui,
            root,
            executor,
            frames,
            clocks,
            policy,
            start: Instant::now(),
        }
    }
    fn tick(&self) -> usize {
        self.executor.borrow_mut().tick()
    }
    fn frame(&self, index: u64) -> usize {
        let delivered = self.frames.deliver(Frame {
            index,
            time: self.start + Duration::from_secs_f64(index as f64 / 120.0),
            interval: Duration::from_secs_f64(1.0 / 120.0),
        });
        self.tick();
        delivered
    }
}
fn options(hz: f64, priority: u8) -> EffectOptions {
    EffectOptions { hz, priority }
}

#[test]
fn clocks_share_one_task_and_batch_consumers_at_independent_cadences() {
    let h = Harness::new(&[options(60.0, 0), options(30.0, 0)]);
    assert!(Rc::ptr_eq(&h.clocks[0].scheduler, &h.clocks[1].scheduler));
    let runs = Rc::new(Cell::new(0));
    let frames = h.clocks.iter().map(EffectClock::frame).collect::<Vec<_>>();
    let _observer = h.clocks[0].scheduler.runtime.effect({
        let runs = runs.clone();
        move || {
            for frame in &frames {
                frame.get();
            }
            runs.set(runs.get() + 1);
        }
    });
    assert_eq!(h.tick(), 1);
    for index in 0..=12 {
        assert!(h.frame(index) <= 1);
    }
    assert_eq!(h.clocks[0].frame().get().tick, 7);
    assert_eq!(h.clocks[1].frame().get().tick, 4);
    assert_eq!(
        runs.get(),
        8,
        "one combined observer execution per selected refresh"
    );
    assert!(h.frames.wants_frame());
}

#[test]
fn component_owns_bound_clock_when_the_view_retains_only_its_frame_signal() {
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    let frames = FrameClock::new();
    let mut ui = Ui::new(100., 100.);
    let output = Rc::new(RefCell::new(None));
    let capture = output.clone();
    let root = ui.mount(provide(
        TaskRunner::from_executor(executor.clone()),
        provide(
            frames.clone(),
            component(move |cx| {
                let clock = cx.effect_clock(options(30., 0));
                let signal = clock.frame();
                *capture.borrow_mut() = Some((signal.clone(), Rc::downgrade(&clock.scheduler)));
                clock.bind(div().size(20., 20.).reactive_style(move || {
                    crate::style::Styles::new()
                        .opacity((0.1 + signal.get().tick as f32 / 10.).min(1.))
                }))
            }),
        ),
    ));
    let (signal, scheduler) = output.borrow_mut().take().unwrap();
    ui.prepare_frame();
    executor.borrow_mut().tick();
    assert!(
        scheduler.upgrade().is_some(),
        "the component lease retains the scheduler"
    );
    assert!(frames.wants_frame());
    frames.deliver(Frame {
        index: 1,
        time: Instant::now(),
        interval: Duration::from_secs_f64(1. / 60.),
    });
    executor.borrow_mut().tick();
    assert_eq!(signal.get().tick, 1);
    ui.set_presented(false);
    executor.borrow_mut().tick();
    assert!(!frames.wants_frame());
    root.unmount();
    executor.borrow_mut().tick();
    assert!(scheduler.upgrade().is_none());
    assert!(!frames.wants_frame());
}

#[test]
fn mixed_twelve_and_eight_hertz_do_not_undersample_the_slower_clock() {
    let h = Harness::new(&[options(12.0, 0), options(8.0, 0)]);
    h.tick();
    for index in 0..=60 {
        h.frame(index);
    }
    assert_eq!(h.clocks[0].frame().get().tick, 7);
    assert_eq!(h.clocks[1].frame().get().tick, 5);
    let a = h.clocks[0].frame().get();
    let b = h.clocks[1].frame().get();
    assert!((a.delta.as_secs_f64() - 1.0 / 12.0).abs() < 0.000001);
    assert!((b.delta.as_secs_f64() - 1.0 / 8.0).abs() < 0.000001);
}

#[test]
fn mixed_low_cadences_wake_only_for_the_next_effect_deadline() {
    let h = Harness::new(&[options(12.0, 0), options(8.0, 0)]);
    h.tick();
    let mut delivered = 0;
    for index in 0..=120 {
        delivered += h.frames.deliver(Frame {
            index,
            time: h.start + Duration::from_secs_f64(index as f64 / 60.0),
            interval: Duration::from_secs_f64(1.0 / 60.0),
        });
        h.tick();
    }
    assert_eq!(h.clocks[0].frame().get().tick, 25);
    assert_eq!(
        h.clocks[1].frame().get().tick,
        16,
        "8Hz rounds to7.5Hz on60Hz display"
    );
    assert_eq!(
        delivered, 37,
        "coincident deadlines batch; intermediate refreshes don't wake the driver"
    );
}

#[test]
fn global_frame_cap_keeps_each_effect_on_its_rounded_cadence() {
    let h = Harness::new(&[options(60.0, 0), options(8.0, 0)]);
    h.frames.set_max_rate(Some(20.0));
    h.tick();
    let mut delivered = 0;
    for index in 0..=60 {
        delivered += h.frame(index);
    }
    assert_eq!(h.clocks[0].frame().get().tick, 11);
    assert_eq!(
        h.clocks[1].frame().get().tick,
        4,
        "15-refresh request rounds up to18 under6-refresh cap"
    );
    assert_eq!(delivered, 11);
}

#[test]
fn fallback_missing_executor_cannot_poison_a_later_backed_component() {
    let frames = FrameClock::new();
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    let runner = TaskRunner::from_executor(executor.clone());
    let mut ui = Ui::new(100.0, 100.0);
    let output = Rc::new(RefCell::new(Vec::new()));
    let capture = output.clone();
    let root = ui.mount(provide(
        frames.clone(),
        component(move |cx| {
            capture.borrow_mut().push(cx.effect_clock(options(30.0, 0)));
            let capture = capture.clone();
            provide(
                runner.clone(),
                component(move |cx| {
                    capture.borrow_mut().push(cx.effect_clock(options(30.0, 0)));
                    div()
                }),
            )
        }),
    ));
    let clocks = output.borrow();
    assert!(!clocks[0].selected().get());
    assert!(clocks[1].selected().get());
    assert!(!Rc::ptr_eq(&clocks[0].scheduler, &clocks[1].scheduler));
    assert_eq!(executor.borrow_mut().tick(), 1);
    assert_eq!(
        frames.deliver(Frame {
            index: 0,
            time: Instant::now(),
            interval: Duration::from_secs_f64(1.0 / 60.0)
        }),
        1
    );
    executor.borrow_mut().tick();
    assert_eq!(clocks[0].frame().get().tick, 0);
    assert_eq!(clocks[1].frame().get().tick, 1);
    root.unmount();
    executor.borrow_mut().tick();
    assert!(!frames.wants_frame());
}

#[test]
fn sharing_a_frame_clock_across_ui_runtimes_keeps_budgets_isolated() {
    let frames = FrameClock::new();
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    let mut mounted = Vec::new();
    let mut clocks = Vec::new();
    for _ in 0..2 {
        let mut ui = Ui::new(100.0, 100.0);
        let output = Rc::new(RefCell::new(None));
        let capture = output.clone();
        let root = ui.mount(provide(
            TaskRunner::from_executor(executor.clone()),
            provide(
                frames.clone(),
                component(move |cx| {
                    *capture.borrow_mut() = Some(cx.effect_clock(options(30.0, 0)));
                    div()
                }),
            ),
        ));
        clocks.push(output.borrow_mut().take().unwrap());
        mounted.push((ui, root));
    }
    assert!(!Rc::ptr_eq(&clocks[0].scheduler, &clocks[1].scheduler));
    EffectScheduler(clocks[0].scheduler.clone()).set_budget(0);
    assert!(!clocks[0].selected().get());
    assert!(clocks[1].selected().get());
    assert_eq!(executor.borrow_mut().tick(), 1);
    assert!(frames.wants_frame());
    mounted[1].1.unmount();
    executor.borrow_mut().tick();
    assert!(!frames.wants_frame());
}

#[test]
fn priority_budget_is_stable_and_zero_budget_has_no_task() {
    let h = Harness::new(&[
        options(30.0, 2),
        options(30.0, 0),
        options(30.0, 0),
        options(30.0, 1),
    ]);
    let scheduler = EffectScheduler(h.clocks[0].scheduler.clone());
    scheduler.set_budget(2);
    let selected = || {
        h.clocks
            .iter()
            .map(|clock| clock.selected().get())
            .collect::<Vec<_>>()
    };
    assert_eq!(selected(), [false, true, true, false]);
    h.clocks[1].enabled().set(false);
    assert_eq!(selected(), [false, false, true, true]);
    h.clocks[0].set_priority(0);
    assert_eq!(selected(), [true, false, true, false]);
    h.tick();
    scheduler.set_budget(0);
    assert_eq!(selected(), [false, false, false, false]);
    assert!(scheduler.0.task.borrow().is_none());
    h.tick();
    assert!(!h.frames.wants_frame());
    assert_eq!(h.tick(), 0);
    scheduler.set_budget(1);
    assert_eq!(h.tick(), 1);
    assert!(h.frames.wants_frame());
}

#[test]
fn policy_hidden_and_disabled_freeze_phase_and_resume_without_wall_time_jump() {
    let h = Harness::new(&[options(30.0, 0)]);
    let clock = &h.clocks[0];
    h.tick();
    h.frame(0);
    let before = clock.frame().get();
    h.policy.active.set(false);
    for index in 1..120 {
        h.frame(index);
    }
    assert_eq!(clock.frame().get(), before);
    h.policy.active.set(true);
    h.tick();
    h.frame(120);
    let after = clock.frame().get();
    assert_eq!(after.tick, before.tick + 1);
    assert!(after.elapsed - before.elapsed < Duration::from_millis(50));
    for (toggle, inverse) in [
        (h.policy.reduced.clone(), true),
        (clock.visible(), false),
        (clock.enabled(), false),
    ] {
        let before = clock.frame().get();
        toggle.set(inverse);
        h.tick();
        h.frame(240);
        assert!(!clock.selected().get());
        assert_eq!(clock.frame().get(), before);
        toggle.set(!inverse);
        h.tick();
        h.frame(360);
        assert!(clock.selected().get());
    }
}

#[test]
fn disposal_cancels_driver_and_external_clones_cannot_resume() {
    let h = Harness::new(&[options(30.0, 0)]);
    h.tick();
    h.frame(0);
    h.root.unmount();
    assert!(!h.clocks[0].selected().get());
    h.clocks[0].enabled().set(true);
    h.clocks[0].visible().set(true);
    h.clocks[0].set_rate(120.0);
    h.tick();
    assert!(!h.frames.wants_frame());
    assert_eq!(h.tick(), 0);
    assert!(!h.clocks[0].track.alive.get());
}

#[test]
fn resetting_phase_inside_a_consumer_is_reentrant() {
    let h = Harness::new(&[options(30.0, 0)]);
    let clock = h.clocks[0].clone();
    let signal = clock.frame();
    let reset = Rc::new(Cell::new(false));
    let _observer = clock.scheduler.runtime.effect({
        let reset = reset.clone();
        let clock = clock.clone();
        move || {
            if signal.get().tick == 1 && !reset.replace(true) {
                clock.reset();
            }
        }
    });
    h.tick();
    h.frame(0);
    assert_eq!(clock.frame().get(), EffectFrame::default());
    h.frame(4);
    assert_eq!(clock.frame().get().tick, 1);
}

#[test]
fn visibility_binding_follows_retained_geometry_and_native_presentation() {
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    let frames = FrameClock::new();
    let mut ui = Ui::new(100.0, 100.0);
    let output = Rc::new(RefCell::new(None));
    let capture = output.clone();
    let root = ui.mount(provide(
        TaskRunner::from_executor(executor.clone()),
        provide(
            frames.clone(),
            component(move |cx| {
                let clock = cx.effect_clock(options(30.0, 0));
                let view = clock.bind(div().id("effect").size(20.0, 20.0));
                *capture.borrow_mut() = Some(clock);
                view
            }),
        ),
    ));
    let clock = output.borrow_mut().take().unwrap();
    ui.prepare_frame();
    assert!(clock.selected().get());
    ui.set_presented(false);
    assert!(!clock.selected().get());
    ui.set_presented(true);
    assert!(clock.selected().get());
    let node = root.find("effect").unwrap();
    ui.scene
        .borrow_mut()
        .set_transform(node, crate::scene::Transform { x: 200.0, y: 0.0 });
    ui.prepare_frame();
    assert!(!clock.visible().get());
    assert!(!clock.selected().get());
    executor.borrow_mut().tick();
    assert!(!frames.wants_frame());
}
