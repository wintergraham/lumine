//! Shared deadline scheduler. Sleeping tasks do not poll or allocate a thread per timer.
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
struct Driver {
    waiters: Mutex<BTreeMap<(Instant, u64), Waker>>,
    changed: Condvar,
    next: AtomicU64,
}
fn driver() -> &'static Arc<Driver> {
    static DRIVER: OnceLock<Arc<Driver>> = OnceLock::new();
    DRIVER.get_or_init(|| {
        let driver = Arc::new(Driver {
            waiters: Mutex::new(BTreeMap::new()),
            changed: Condvar::new(),
            next: AtomicU64::new(1),
        });
        let shared = driver.clone();
        std::thread::Builder::new()
            .name("lumine-timers".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                loop {
                    let mut waiters = shared.waiters.lock().unwrap();
                    loop {
                        let Some((&(deadline, _), _)) = waiters.first_key_value() else {
                            waiters = shared.changed.wait(waiters).unwrap();
                            continue;
                        };
                        let now = Instant::now();
                        if deadline > now {
                            waiters = shared
                                .changed
                                .wait_timeout(waiters, deadline - now)
                                .unwrap()
                                .0;
                            continue;
                        }
                        let mut ready = Vec::new();
                        while waiters
                            .first_key_value()
                            .is_some_and(|((deadline, _), _)| *deadline <= now)
                        {
                            ready.push(waiters.pop_first().unwrap().1);
                        }
                        drop(waiters);
                        for waker in ready {
                            waker.wake();
                        }
                        break;
                    }
                }
            })
            .expect("create timer scheduler");
        driver
    })
}
#[must_use = "Futures do nothing unless awaited or polled"]
pub struct Sleep {
    deadline: Instant,
    registration: Option<(Arc<Driver>, u64)>,
}
pub fn sleep(duration: Duration) -> Sleep {
    sleep_until(Instant::now() + duration)
}
pub fn sleep_until(deadline: Instant) -> Sleep {
    Sleep {
        deadline,
        registration: None,
    }
}
impl Future for Sleep {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if Instant::now() >= self.deadline {
            if let Some((d, id)) = self.registration.take() {
                let removed = d.waiters.lock().unwrap().remove(&(self.deadline, id));
                drop(removed);
            }
            return Poll::Ready(());
        }
        if self.registration.is_none() {
            let d = driver().clone();
            let id = d.next.fetch_add(1, Ordering::Relaxed);
            self.registration = Some((d, id));
        }
        let (d, id) = self.registration.as_ref().unwrap();
        // Waker clone/drop may execute user code. Keep both outside the
        // scheduler lock, including replacing a task's previous waker.
        let next = cx.waker().clone();
        let removed = {
            let mut waiters = d.waiters.lock().unwrap();
            match waiters.get(&(self.deadline, *id)) {
                Some(w) if w.will_wake(&next) => Some(next),
                _ => waiters.insert((self.deadline, *id), next),
            }
        };
        drop(removed);
        d.changed.notify_one();
        Poll::Pending
    }
}
impl Drop for Sleep {
    fn drop(&mut self) {
        if let Some((d, id)) = self.registration.take() {
            let removed = d.waiters.lock().unwrap().remove(&(self.deadline, id));
            drop(removed);
            d.changed.notify_one();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;
    struct Count(AtomicUsize);
    impl Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn cancellation_unregisters_and_deadline_wakes_once() {
        let c = Arc::new(Count(AtomicUsize::new(0)));
        let w = Waker::from(c.clone());
        let mut cx = Context::from_waker(&w);
        let mut cancelled = sleep(Duration::from_secs(10));
        assert!(Pin::new(&mut cancelled).poll(&mut cx).is_pending());
        let (d, id) = cancelled.registration.as_ref().unwrap();
        let d = d.clone();
        let id = *id;
        let deadline = cancelled.deadline;
        drop(cancelled);
        assert!(!d.waiters.lock().unwrap().contains_key(&(deadline, id)));
        let mut future = sleep(Duration::from_millis(10));
        assert!(Pin::new(&mut future).poll(&mut cx).is_pending());
        let end = Instant::now() + Duration::from_secs(2);
        while c.0.load(Ordering::SeqCst) == 0 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(c.0.load(Ordering::SeqCst), 1);
        assert!(Pin::new(&mut future).poll(&mut cx).is_ready());
    }
}

#[cfg(test)]
mod waker_cleanup_tests {
    use super::*;
    use std::{
        sync::{Weak, atomic::AtomicBool},
        task::Wake,
    };
    struct CheckDrop {
        driver: Weak<Driver>,
        locked: Arc<AtomicBool>,
        dropped: Arc<AtomicBool>,
    }
    #[allow(
        clippy::manual_noop_waker,
        reason = "custom Drop is the behavior under test"
    )]
    impl Wake for CheckDrop {
        fn wake(self: Arc<Self>) {}
    }
    impl Drop for CheckDrop {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
            if let Some(driver) = self.driver.upgrade() {
                self.locked
                    .store(driver.waiters.try_lock().is_err(), Ordering::SeqCst);
            }
        }
    }
    #[test]
    fn replacement_and_cancellation_drop_user_wakers_outside_timer_lock() {
        for mode in ["cancel", "replace", "ready"] {
            let driver = Arc::new(Driver {
                waiters: Mutex::new(BTreeMap::new()),
                changed: Condvar::new(),
                next: AtomicU64::new(2),
            });
            let locked = Arc::new(AtomicBool::new(false));
            let dropped = Arc::new(AtomicBool::new(false));
            let deadline = if mode == "ready" {
                Instant::now()
            } else {
                Instant::now() + Duration::from_secs(60)
            };
            driver.waiters.lock().unwrap().insert(
                (deadline, 1),
                Waker::from(Arc::new(CheckDrop {
                    driver: Arc::downgrade(&driver),
                    locked: locked.clone(),
                    dropped: dropped.clone(),
                })),
            );
            let mut future = Sleep {
                deadline,
                registration: Some((driver.clone(), 1)),
            };
            if mode == "ready" {
                assert!(
                    Pin::new(&mut future)
                        .poll(&mut Context::from_waker(Waker::noop()))
                        .is_ready()
                );
            } else if mode == "replace" {
                assert!(
                    Pin::new(&mut future)
                        .poll(&mut Context::from_waker(Waker::noop()))
                        .is_pending()
                );
            } else {
                drop(future);
            }
            assert!(dropped.load(Ordering::SeqCst), "user waker was not dropped");
            assert!(
                !locked.load(Ordering::SeqCst),
                "user waker destructor ran under timer lock (mode={mode})"
            );
        }
    }
}
