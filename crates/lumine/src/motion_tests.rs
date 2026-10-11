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
    policy: MotionPolicy,
    values: Vec<MotionValue>,
    presence: Presence,
    start: Instant,
}

impl Harness {
    fn new(count: usize) -> Self {
        Self::build(count, false)
    }

    fn build(count: usize, isolated_second: bool) -> Self {
        let executor = Rc::new(RefCell::new(LocalExecutor::new()));
        let frames = FrameClock::new();
        let mut ui = Ui::new(100.0, 100.0);
        let policy = MotionPolicy {
            active: ui.signal(true),
            reduced: ui.signal(false),
        };
        let output = Rc::new(RefCell::new(None));
        let capture = output.clone();
        let root = ui.mount(provide(
            TaskRunner::from_executor(executor.clone()),
            provide(
                frames.clone(),
                provide(
                    policy.clone(),
                    component(move |cx| {
                        let values = (0..count)
                            .map(|index| {
                                if isolated_second && index == 1 {
                                    MotionValue::with_policy(cx, 0.0, None)
                                } else {
                                    cx.motion_value(0.0)
                                }
                            })
                            .collect::<Vec<_>>();
                        let presence = Presence::new(cx, false);
                        *capture.borrow_mut() = Some((values, presence));
                        div()
                    }),
                ),
            ),
        ));
        let (values, presence) = output.borrow_mut().take().unwrap();
        Self {
            _ui: ui,
            root,
            executor,
            frames,
            policy,
            values,
            presence,
            start: Instant::now(),
        }
    }

    fn tick(&self) -> usize {
        self.executor.borrow_mut().tick()
    }

    fn frame_at(&self, index: u64, time: Instant) -> usize {
        let delivered = self.frames.deliver(Frame {
            index,
            time,
            interval: Duration::from_millis(10),
        });
        self.tick();
        delivered
    }

    fn frame(&self, index: u64, millis: u64) -> usize {
        self.frame_at(index, self.start + Duration::from_millis(millis))
    }
}

fn tween(millis: u64) -> Transition {
    Transition::tween(Duration::from_millis(millis), Easing::Linear)
}

#[test]
fn opposite_finite_extreme_endpoints_remain_finite_for_tweens_and_springs() {
    for transition in [tween(1000), Transition::spring(Spring::default())] {
        let harness = Harness::new(1);
        let motion = &harness.values[0];
        motion.set(f32::MAX);
        let animation = motion.animate_to(-f32::MAX, transition);
        harness.tick();
        harness.frame(1, 100);
        assert!(motion.get().is_finite());
        assert!(motion.velocity().is_finite());
        assert!(motion.get() < f32::MAX, "the animation must advance");
        harness.frame(2, 10_000);
        assert_eq!(motion.get(), -f32::MAX);
        assert_eq!(animation.result.get(), Some(Completion::Finished));
    }
}

fn completion(animation: Animation) -> Poll<Completion> {
    let mut future = std::pin::pin!(animation.finished());
    future
        .as_mut()
        .poll(&mut PollContext::from_waker(Waker::noop()))
}

#[test]
fn simultaneous_values_share_one_task_and_batch_observers() {
    let h = Harness::new(2);
    assert!(Rc::ptr_eq(&h.values[0].scheduler, &h.values[1].scheduler));
    let signals = h.values.iter().map(MotionValue::signal).collect::<Vec<_>>();
    let runs = Rc::new(Cell::new(0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _effect = h.values[0].scheduler.runtime.effect({
        let runs = runs.clone();
        let seen = seen.clone();
        move || {
            runs.set(runs.get() + 1);
            seen.borrow_mut().push((signals[0].get(), signals[1].get()));
        }
    });
    for value in &h.values {
        value.animate_to(1.0, tween(100));
    }
    assert_eq!(
        h.tick(),
        1,
        "one scheduler task, independent of value count"
    );
    assert_eq!(h.frame(1, 50), 1, "one registered frame waiter");
    assert_eq!(runs.get(), 2, "the combined consumer runs once per frame");
    let sample = *seen.borrow().last().unwrap();
    assert!((sample.0 - sample.1).abs() < 0.01);
    assert_eq!(h.frame(2, 150), 1);
    assert!(!h.frames.wants_frame());
    assert!(h.values.iter().all(|v| v.get() == 1.0));
    assert_eq!(h.tick(), 0, "settled driver parks");
}

#[test]
fn component_owns_animation_when_the_view_retains_only_its_signal() {
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
                let value = cx.motion_value(0.);
                let run = value.animate_to(1., tween(1_000));
                let signal = value.signal();
                *capture.borrow_mut() =
                    Some((signal.clone(), run, Rc::downgrade(&value.scheduler)));
                div().reactive_style(move || crate::style::Styles::new().opacity(signal.get()))
            }),
        ),
    ));
    let (signal, run, scheduler) = output.borrow_mut().take().unwrap();
    assert!(
        scheduler.upgrade().is_some(),
        "the component lease retains the scheduler"
    );
    executor.borrow_mut().tick();
    assert!(frames.wants_frame());
    frames.deliver(Frame {
        index: 1,
        time: Instant::now() + Duration::from_millis(100),
        interval: Duration::from_millis(10),
    });
    executor.borrow_mut().tick();
    assert!(signal.get() > 0. && signal.get() < 1.);
    root.unmount();
    executor.borrow_mut().tick();
    assert_eq!(completion(run), Poll::Ready(Completion::Cancelled));
    assert!(scheduler.upgrade().is_none());
    assert!(!frames.wants_frame());
}

#[test]
fn a_headless_fallback_cannot_poison_a_later_executor_backed_subtree() {
    let frames = FrameClock::new();
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    let runner = TaskRunner::from_executor(executor.clone());
    let mut ui = Ui::new(100.0, 100.0);
    let output = Rc::new(RefCell::new(None));
    let capture = output.clone();
    let _root = ui.mount(provide(
        frames.clone(),
        component(move |cx| {
            let fallback = cx.motion_value(0.0);
            provide(
                runner,
                component(move |cx| {
                    *capture.borrow_mut() = Some((fallback, cx.motion_value(0.0)));
                    div()
                }),
            )
        }),
    ));
    let (fallback, animated) = output.borrow_mut().take().unwrap();
    fallback.animate_to(1.0, tween(100));
    assert_eq!(fallback.get(), 1.0);
    animated.animate_to(1.0, tween(100));
    assert_eq!(
        animated.get(),
        0.0,
        "executor-backed subtree should animate"
    );
    executor.borrow_mut().tick();
    assert!(frames.wants_frame());
}

#[test]
fn independent_reactive_roots_use_independent_schedulers_on_a_shared_clock() {
    let frames = FrameClock::new();
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    let mut roots = Vec::new();
    let mut uis = Vec::new();
    let mut values = Vec::new();
    for _ in 0..2 {
        let mut ui = Ui::new(100.0, 100.0);
        let output = Rc::new(RefCell::new(None));
        let capture = output.clone();
        let root = ui.mount(provide(
            frames.clone(),
            provide(
                TaskRunner::from_executor(executor.clone()),
                component(move |cx| {
                    *capture.borrow_mut() = Some(cx.motion_value(0.0));
                    div()
                }),
            ),
        ));
        values.push(output.borrow_mut().take().unwrap());
        roots.push(root);
        uis.push(ui);
    }
    assert!(!Rc::ptr_eq(&values[0].scheduler, &values[1].scheduler));
    for value in &values {
        value.animate_to(1.0, tween(100));
    }
    assert_eq!(executor.borrow_mut().tick(), 2);
    assert_eq!(
        frames.deliver(Frame {
            index: 1,
            time: Instant::now() + Duration::from_millis(150),
            interval: Duration::from_millis(10)
        }),
        2
    );
    executor.borrow_mut().tick();
    assert!(values.iter().all(|value| value.get() == 1.0));
    assert!(!frames.wants_frame());
}

#[test]
fn inactive_policy_unregisters_frames_and_resumes_without_hidden_time() {
    let h = Harness::new(1);
    let value = &h.values[0];
    value.animate_to(1.0, tween(100));
    h.tick();
    h.frame(1, 40);
    let before = value.get();
    h.policy.active.set(false);
    h.tick();
    assert!(!h.frames.wants_frame());
    assert_eq!(h.frame(2, 10_000), 0);
    assert_eq!(value.get(), before);
    h.policy.active.set(true);
    h.tick();
    h.frame_at(3, Instant::now() + Duration::from_millis(10));
    assert!(
        (value.get() - before - 0.1).abs() < 0.03,
        "resumed value: {} before: {before}",
        value.get()
    );
}

#[test]
fn pausing_a_subtree_does_not_pause_other_tracks_on_the_same_clock() {
    let h = Harness::build(2, true);
    for value in &h.values {
        value.animate_to(1.0, tween(100));
    }
    h.tick();
    h.frame(1, 40);
    let paused = h.values[0].get();
    h.policy.active.set(false);
    h.tick();
    assert!(h.frames.wants_frame());
    h.frame(2, 80);
    assert_eq!(h.values[0].get(), paused);
    assert!(h.values[1].get() > 0.75);
    h.frame(3, 150);
    assert_eq!(h.values[1].get(), 1.0);
    assert!(!h.frames.wants_frame(), "only the paused track remains");
}

#[test]
fn delayed_tracks_sleep_and_pause_preserves_elapsed_active_delay() {
    let h = Harness::new(1);
    h.values[0].animate_to(1.0, tween(100).delay(Duration::from_secs(5)));
    h.tick();
    assert!(
        !h.frames.wants_frame(),
        "a delay-only driver uses one timer, no display refreshes"
    );
    // Inject two seconds of active delay, avoiding wall-clock sleeps in tests.
    h.values[0].track.run.borrow_mut().as_mut().unwrap().last =
        Some(Instant::now() - Duration::from_secs(2));
    h.policy.active.set(false);
    let elapsed = h.values[0].track.run.borrow().as_ref().unwrap().elapsed;
    assert!(
        elapsed >= Duration::from_millis(1_990) && elapsed < Duration::from_millis(2_100),
        "active delay lost on pause: {elapsed:?}"
    );
    h.tick();
    assert!(!h.frames.wants_frame());
    let elapsed = h.values[0].track.run.borrow().as_ref().unwrap().elapsed;
    h.policy.active.set(true);
    h.tick();
    assert_eq!(
        h.values[0].track.run.borrow().as_ref().unwrap().elapsed,
        elapsed
    );
    assert!(!h.frames.wants_frame());
}

#[test]
fn reduced_motion_finishes_running_and_new_transitions_without_frames() {
    let h = Harness::new(1);
    let animation = h.values[0].animate_to(1.0, Transition::spring(Spring::default()));
    h.tick();
    assert!(h.frames.wants_frame());
    h.policy.reduced.set(true);
    h.tick();
    assert_eq!(h.values[0].get(), 1.0);
    assert_eq!(completion(animation), Poll::Ready(Completion::Finished));
    assert!(!h.frames.wants_frame());
    let delayed = h.values[0].animate_to(0.0, tween(100).delay(Duration::from_secs(5)));
    assert_eq!(h.values[0].get(), 0.0);
    assert_eq!(completion(delayed), Poll::Ready(Completion::Finished));
    assert!(!h.frames.wants_frame());
}

#[test]
fn component_disposal_cancels_running_values_and_external_clones_cannot_restart() {
    let h = Harness::new(1);
    let external = h.values[0].clone();
    let animation = external.animate_to(1.0, tween(100));
    h.tick();
    h.root.unmount();
    h.tick();
    assert_eq!(completion(animation), Poll::Ready(Completion::Cancelled));
    assert!(!h.frames.wants_frame());
    external.animate_to(2.0, tween(100));
    external.set(3.0);
    h.tick();
    assert_eq!(external.get(), 0.0);
    assert!(!h.frames.wants_frame());
}

#[test]
fn retargets_cancel_previous_runs_and_preserve_spring_velocity() {
    let h = Harness::new(1);
    let value = &h.values[0];
    let first = value.animate_to(1.0, Transition::spring(Spring::default()));
    h.tick();
    h.frame(1, 40);
    let current = value.get();
    let velocity = value.velocity();
    assert!(velocity > 0.0);
    let second = value.animate_to(current, Transition::spring(Spring::default()));
    assert_eq!(completion(first), Poll::Ready(Completion::Cancelled));
    assert_eq!(value.velocity(), velocity);
    assert_eq!(
        completion(second.clone()),
        Poll::Pending,
        "same position with nonzero velocity is still moving"
    );
    h.tick();
    h.frame(2, 60);
    assert!(value.get() > current);
    h.frame(3, 5_000);
    assert_eq!(value.get(), current);
    assert_eq!(completion(second), Poll::Ready(Completion::Finished));
    assert!(!h.frames.wants_frame());
}

#[test]
fn completion_waiter_wakes_on_supersession_and_stop() {
    let h = Harness::new(1);
    let animation = h.values[0].animate_to(1.0, tween(100));
    let mut finished = Box::pin(animation.finished());
    struct CountWake(std::sync::atomic::AtomicUsize);
    impl std::task::Wake for CountWake {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let wakes = std::sync::Arc::new(CountWake(std::sync::atomic::AtomicUsize::new(0)));
    let waker = Waker::from(wakes.clone());
    let mut cx = PollContext::from_waker(&waker);
    assert!(finished.as_mut().poll(&mut cx).is_pending());
    h.values[0].animate_to(2.0, tween(100));
    assert_eq!(wakes.0.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(
        finished.as_mut().poll(&mut cx),
        Poll::Ready(Completion::Cancelled)
    );
    let current = h.values[0].animation();
    h.values[0].stop();
    assert_eq!(completion(current), Poll::Ready(Completion::Cancelled));
    h.tick();
    assert!(!h.frames.wants_frame());
}

#[test]
fn completed_handles_remain_finished_after_reentrant_retargeting() {
    let h = Harness::new(1);
    let value = h.values[0].clone();
    let signal = value.signal();
    let _effect = value.scheduler.runtime.effect({
        let value = value.clone();
        move || {
            if signal.get() == 1.0 {
                value.animate_to(2.0, tween(100));
            }
        }
    });
    let first = value.animate_to(1.0, Transition::instant());
    assert_eq!(completion(first.clone()), Poll::Ready(Completion::Finished));
    assert_eq!(
        completion(value.animation()),
        Poll::Pending,
        "observer's reentrant run is distinct"
    );
    h.tick();
    h.frame(1, 150);
    assert_eq!(value.get(), 2.0);
    assert_eq!(completion(first), Poll::Ready(Completion::Finished));
}

#[test]
fn presence_retains_exit_and_reopen_invalidates_old_removal() {
    let h = Harness::new(0);
    let presence = &h.presence;
    presence.set_present(true, Transition::instant());
    assert!(presence.mounted().get());
    presence.set_present(false, tween(100));
    h.tick();
    h.frame(1, 40);
    assert!(presence.mounted().get());
    presence.set_present(true, tween(100));
    h.tick();
    h.frame(2, 150);
    h.tick();
    assert!(
        presence.mounted().get(),
        "cancelled exit cannot unmount a reopened view"
    );
    presence.set_present(false, tween(100));
    h.tick();
    h.frame(3, 300);
    h.tick();
    assert!(!presence.mounted().get());
    assert!(!h.frames.wants_frame());
}

#[test]
fn a_completed_presence_exit_cannot_remove_a_later_pending_exit() {
    let h = Harness::new(0);
    let presence = &h.presence;
    presence.set_present(true, Transition::instant());
    presence.set_present(false, Transition::instant());
    presence.set_present(true, Transition::instant());
    presence.set_present(false, tween(100));
    h.tick();
    assert!(
        presence.mounted().get(),
        "the earlier completed exit callback must not remove the newer exiting view"
    );
    h.frame(1, 150);
    h.tick();
    assert!(!presence.mounted().get());
}

#[test]
fn analytical_springs_agree_across_frame_rates_and_all_damping_regimes() {
    for damping in [10.0, 20.0, 40.0, 1_000_000.0] {
        let spring = Spring {
            stiffness: 100.0,
            damping,
            ..Spring::default()
        };
        let oscillator = Oscillator::new(spring, -1.0, 2.0);
        for frames in [60, 144] {
            let dt = 1.0 / frames as f64;
            let mut position = -1.0;
            let mut velocity = 2.0;
            for _ in 0..frames {
                (position, velocity) = Oscillator::new(spring, position, velocity).sample(dt);
            }
            let expected = oscillator.sample(1.0);
            assert!(
                (position - expected.0).abs() < 1e-4,
                "damping={damping}, frames={frames}"
            );
            assert!((velocity - expected.1).abs() < 1e-4);
            assert!(position.is_finite() && velocity.is_finite());
        }
    }
}

#[test]
fn bezier_inverts_flat_x_controls_and_preserves_overshoot() {
    // x(s)=s³; y(s)=3s-3s²+s³, including tiny x near a flat derivative.
    let easing = Easing::cubic_bezier(0.0, 1.0, 0.0, 1.0);
    for t in [1e-8_f32, 1e-6, 0.01, 0.25, 0.5, 0.99] {
        let s = (t as f64).cbrt();
        let expected = (3.0 * s - 3.0 * s * s + s * s * s) as f32;
        assert!(
            (easing.sample(t) - expected).abs() < 2e-5,
            "t={t}: {} vs {expected}",
            easing.sample(t)
        );
    }
    let overshoot = Easing::cubic_bezier(0.2, 2.0, 0.8, 2.0);
    assert!(overshoot.sample(0.5) > 1.0);
    assert_eq!(overshoot.sample(0.0), 0.0);
    assert_eq!(overshoot.sample(1.0), 1.0);
    let end_flat = Easing::cubic_bezier(1.0, 0.0, 1.0, 0.0);
    for t in [0.99_f32, 0.9999, 0.9999999] {
        let s = 1.0 - (1.0 - t as f64).cbrt();
        assert!((end_flat.sample(t) - (s * s * s) as f32).abs() < 2e-5);
    }
}

#[test]
fn retargeting_from_an_effect_does_not_subscribe_to_motion_value() {
    let h = Harness::new(1);
    let value = h.values[0].clone();
    let trigger = value.scheduler.runtime.signal(0);
    let runs = Rc::new(Cell::new(0));
    let _effect = value.scheduler.runtime.effect({
        let value = value.clone();
        let trigger = trigger.clone();
        let runs = runs.clone();
        move || {
            runs.set(runs.get() + 1);
            value.animate_to(trigger.get() as f32, tween(100));
        }
    });
    trigger.set(1);
    h.tick();
    h.frame(1, 40);
    assert_eq!(runs.get(), 2);
    trigger.set(2);
    h.tick();
    h.frame(2, 150);
    assert_eq!(value.get(), 2.0);
    assert_eq!(runs.get(), 3);
}

#[test]
#[ignore = "run in release mode with --ignored --nocapture for timing"]
fn motion_release_benchmark() {
    // A slow oscillator keeps the full active set alive for all 1,000 frames;
    // ordinary UI springs settle early and would measure an idle scheduler.
    let modes = [
        ("linear", tween(60_000)),
        (
            "bezier",
            Transition::tween(
                Duration::from_secs(60),
                Easing::cubic_bezier(0.2, 0.0, 0.0, 1.0),
            ),
        ),
        (
            "spring",
            Transition::spring(Spring {
                stiffness: 0.01,
                damping: 0.001,
                ..Spring::default()
            }),
        ),
    ];
    for (mode, transition) in modes {
        for count in [1, 100, 1_000] {
            for consumers in [false, true] {
                let h = Harness::new(count);
                let runtime = h.values[0].scheduler.runtime.clone();
                let effects = Rc::new(Cell::new(0_u64));
                let _consumers = if consumers {
                    h.values
                        .iter()
                        .map(|value| {
                            let signal = value.signal();
                            let effects = effects.clone();
                            runtime.effect(move || {
                                std::hint::black_box(signal.get());
                                effects.set(effects.get() + 1);
                            })
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let enqueue_started = Instant::now();
                for value in &h.values {
                    value.animate_to(1.0, transition);
                }
                let enqueue_elapsed = enqueue_started.elapsed();
                assert_eq!(h.tick(), 1);
                effects.set(0);
                let presentation_start = Instant::now();
                let started = Instant::now();
                let mut delivered = 0;
                let mut polls = 0;
                for index in 1..=1_000 {
                    delivered += h.frames.deliver(Frame {
                        index,
                        time: presentation_start + Duration::from_millis(index * 10),
                        interval: Duration::from_millis(10),
                    });
                    polls += h.tick();
                }
                let elapsed = started.elapsed();
                assert_eq!(
                    delivered, 1_000,
                    "exactly one driver is delivered per refresh"
                );
                assert_eq!(polls, 1_000, "exactly one driver poll per refresh");
                if consumers {
                    assert_eq!(effects.get(), 1_000 * count as u64);
                }
                let batched_started = Instant::now();
                runtime.batch(|| {
                    for value in &h.values {
                        value.animate_to(0.0, transition);
                    }
                });
                let batched_elapsed = batched_started.elapsed();
                println!(
                    "motion mode={mode}, values={count}, consumers={consumers}, frames=1000, enqueue={enqueue_elapsed:?}, batch_retarget={batched_elapsed:?}, elapsed={elapsed:?}, us/frame={:.3}, effect_runs={}, frame_requests={delivered}, task_polls={polls}",
                    elapsed.as_secs_f64() * 1_000.0,
                    effects.get()
                );
            }
        }
    }
}
