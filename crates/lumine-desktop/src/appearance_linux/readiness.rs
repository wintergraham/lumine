//! A descriptor waiter, not a GTK thread or a periodic appearance poll.
use super::{Event, Wake};
use std::{
    io::{Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::mpsc,
    thread,
};
pub(super) struct Watch {
    pub fds: Vec<libc::pollfd>,
    pub timeout: i32,
}
pub(super) struct Readiness {
    commands: mpsc::Sender<Watch>,
    wake: UnixStream,
    thread: Option<thread::JoinHandle<()>>,
}
impl Readiness {
    pub(super) fn new(notify: Wake) -> std::io::Result<Self> {
        let (mut receiver, wake) = UnixStream::pair()?;
        receiver.set_nonblocking(true)?;
        let (commands, requests) = mpsc::channel::<Watch>();
        let thread = thread::Builder::new()
            .name("lumine-gtk-ready".into())
            .spawn(move || {
                let mut current: Option<Watch> = None;
                loop {
                    let mut fds = current.as_ref().map(|w| w.fds.clone()).unwrap_or_default();
                    fds.push(libc::pollfd {
                        fd: receiver.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    });
                    let timeout = current.as_ref().map(|w| w.timeout).unwrap_or(-1);
                    let status = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
                    if status < 0 {
                        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                        {
                            continue;
                        }
                        break;
                    }
                    let control = fds.last().unwrap().revents;
                    if control != 0 {
                        let mut bytes = [0; 64];
                        match receiver.read(&mut bytes) {
                            Ok(0) => break,
                            _ => {}
                        }
                        loop {
                            match requests.try_recv() {
                                Ok(watch) => current = Some(watch),
                                Err(mpsc::TryRecvError::Empty) => break,
                                Err(mpsc::TryRecvError::Disconnected) => return,
                            }
                        }
                        continue;
                    }
                    if current.take().is_some() {
                        notify(Event::DispatchGtk);
                    }
                }
            })?;
        Ok(Self {
            commands,
            wake,
            thread: Some(thread),
        })
    }
    pub(super) fn watch(&mut self, watch: Watch) {
        if self.commands.send(watch).is_ok() {
            let _ = self.wake.write_all(&[1]);
        }
    }
}
impl Drop for Readiness {
    fn drop(&mut self) {
        let _ = self.wake.shutdown(std::net::Shutdown::Both);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
