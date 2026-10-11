use std::{cell::Cell, rc::Rc};
use lumine::{
    compose::{ViewHandle, prelude::*},
    widgets::Ui,
};

struct Lease(Rc<Cell<usize>>);
impl Lease {
    fn new(live: Rc<Cell<usize>>) -> Self {
        live.set(live.get() + 1);
        Self(live)
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}
struct Service(Rc<()>);

#[test]
fn retained_handles_do_not_keep_keyed_rows_slots_services_or_effects_alive() {
    let mut ui = Ui::new(400., 300.);
    let baseline = ui.runtime.effect_count();
    let live = Rc::new(Cell::new(0));
    let mut old_handles = Vec::new();
    for _ in 0..20 {
        let token = Rc::new(());
        let weak = Rc::downgrade(&token);
        let keys = ui.signal(vec![1, 2, 3]);
        let read_keys = keys.clone();
        let tick = ui.signal(0);
        let read_tick = tick.clone();
        let live_rows = live.clone();
        let mounted = ui.mount(provide(
            Service(token),
            component(move |cx| {
                let body = cx.slot(move |cx| {
                    let service = cx.service::<Service>();
                    text_signal(move || {
                        format!("{}:{}", Rc::strong_count(&service.0), read_tick.get())
                    })
                    .id("slot")
                });
                column().child(body).child(keyed(
                    move || read_keys.get(),
                    move |key, cx| {
                        cx.retain(Lease::new(live_rows.clone()));
                        text(format!("row {key}")).id(format!("row-{key}"))
                    },
                ))
            }),
        ));
        ui.prepare_frame();
        let retained = mounted.find("row-2");
        assert_eq!(live.get(), 3);
        keys.set(vec![3, 2, 1]);
        ui.prepare_frame();
        assert_eq!(mounted.find("row-2"), retained);
        assert_eq!(live.get(), 3);
        keys.set(vec![2, 4]);
        ui.prepare_frame();
        assert_eq!(live.get(), 2);
        assert_eq!(mounted.find("row-2"), retained);
        mounted.unmount();
        ui.prepare_frame();
        assert_eq!(live.get(), 0);
        assert!(
            weak.upgrade().is_none(),
            "slot/provider ownership must end with subtree"
        );
        assert_eq!(ui.runtime.effect_count(), baseline);
        keys.set(vec![5]);
        tick.set(1);
        assert!(!mounted.is_mounted());
        assert!(mounted.find("slot").is_none());
        old_handles.push(mounted);
    }
    assert_eq!(ui.scene.borrow().children(ui.root()).len(), 0);
    assert_eq!(live.get(), 0);
    for handle in old_handles {
        handle.unmount();
    }
}

struct RemoveOnDrop(ViewHandle, Rc<Cell<usize>>);
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        self.1.set(self.1.get() + 1);
        self.0.unmount();
    }
}

#[test]
fn component_resource_destructor_can_remove_sibling_with_retained_handles() {
    let mut ui = Ui::new(400., 300.);
    let baseline = ui.runtime.effect_count();
    let live = Rc::new(Cell::new(0));
    let child_live = live.clone();
    let sibling = ui.mount(component(move |cx| {
        cx.retain(Lease::new(child_live));
        text("Sibling").id("sibling")
    }));
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let remove = sibling.clone();
    let owner = ui.mount(component(move |cx| {
        cx.retain(RemoveOnDrop(remove, count));
        text("Owner")
    }));
    owner.unmount();
    ui.prepare_frame();
    assert_eq!(calls.get(), 1);
    assert_eq!(live.get(), 0);
    assert!(!owner.is_mounted());
    assert!(!sibling.is_mounted());
    assert!(sibling.find("sibling").is_none());
    assert_eq!(ui.runtime.effect_count(), baseline);
    owner.unmount();
    sibling.unmount();
    assert_eq!(calls.get(), 1);
}

#[test]
fn completed_task_guard_destructor_can_spawn_through_retained_tasks_capability() {
    use std::cell::RefCell;
    use lumine::compose::{TaskRunner, TaskToken, Tasks};
    struct Reenter {
        tasks: Rc<RefCell<Option<Tasks>>>,
        fired: Rc<Cell<bool>>,
    }
    impl Drop for Reenter {
        fn drop(&mut self) {
            if !self.fired.replace(true) {
                self.tasks.borrow().as_ref().unwrap().spawn(async {});
            }
        }
    }
    let tasks = Rc::new(RefCell::new(None::<Tasks>));
    let output = tasks.clone();
    let runner_tasks = tasks.clone();
    let fired = Rc::new(Cell::new(false));
    let observed = fired.clone();
    let runner = TaskRunner::new(move |_future| {
        TaskToken::new(
            Reenter {
                tasks: runner_tasks.clone(),
                fired: observed.clone(),
            },
            |_| true,
        )
    });
    let mut ui = Ui::new(400., 300.);
    let mounted = ui.mount(provide(
        runner,
        component(move |cx| {
            *output.borrow_mut() = Some(cx.tasks());
            text("Tasks")
        }),
    ));
    let capability = tasks.borrow().as_ref().unwrap().clone();
    capability.spawn(async {});
    capability.spawn(async {});
    assert!(fired.get());
    mounted.unmount();
    tasks.borrow_mut().take();
    capability.spawn(async {});
}

#[test]
fn task_completion_check_can_spawn_and_new_guard_stays_owned_until_unmount() {
    use std::cell::RefCell;
    use lumine::compose::{TaskRunner, TaskToken, Tasks};
    let mut ui = Ui::new(400., 300.);
    let tasks = Rc::new(RefCell::new(None::<Tasks>));
    let output = tasks.clone();
    let runner_tasks = tasks.clone();
    let checked = Rc::new(Cell::new(false));
    let live = Rc::new(Cell::new(0));
    let live_guards = live.clone();
    let observed = checked.clone();
    let runner = TaskRunner::new(move |_future| {
        let tasks = runner_tasks.clone();
        let checked = observed.clone();
        TaskToken::new(Lease::new(live_guards.clone()), move |_| {
            if !checked.replace(true) {
                tasks.borrow().as_ref().unwrap().spawn(async {});
            }
            false
        })
    });
    let mounted = ui.mount(provide(
        runner,
        component(move |cx| {
            *output.borrow_mut() = Some(cx.tasks());
            text("Tasks")
        }),
    ));
    let capability = tasks.borrow().as_ref().unwrap().clone();
    capability.spawn(async {});
    assert_eq!(live.get(), 1);
    capability.spawn(async {});
    assert!(checked.get());
    assert_eq!(
        live.get(),
        3,
        "original, reentrant and outer tasks remain owned"
    );
    mounted.unmount();
    assert_eq!(live.get(), 0, "unmount releases every live guard");
    tasks.borrow_mut().take();
    capability.spawn(async {});
    assert_eq!(
        live.get(),
        0,
        "retained capability cannot recreate a dead owner"
    );
}
