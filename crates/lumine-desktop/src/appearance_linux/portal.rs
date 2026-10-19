use super::{Event, PortalPreference, Wake};
use dbus::{
    arg::{RefArg, Variant},
    blocking::Connection,
    channel::{BusType, Channel},
    message::MatchRule,
};
use std::{
    io::Write,
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
const SERVICE: &str = "org.freedesktop.portal.Desktop";
const INTERFACE: &str = "org.freedesktop.portal.Settings";
const GROUP: &str = "org.freedesktop.appearance";
pub(super) struct Portal {
    stop: UnixStream,
    thread: Option<thread::JoinHandle<()>>,
}
fn preference(value: &dyn RefArg) -> PortalPreference {
    let value = value.as_u64().or_else(|| value.as_iter()?.next()?.as_u64());
    match value {
        Some(0) => PortalPreference::NoPreference,
        Some(1) => PortalPreference::Dark,
        Some(2) => PortalPreference::Light,
        _ => PortalPreference::Unavailable,
    }
}
fn read(connection: &Connection) -> PortalPreference {
    let proxy = connection.with_proxy(
        SERVICE,
        "/org/freedesktop/portal/desktop",
        Duration::from_millis(750),
    );
    let result: Result<(Variant<Box<dyn RefArg>>,), _> =
        proxy.method_call(INTERFACE, "ReadOne", (GROUP, "color-scheme"));
    match result {
        Ok((value,)) => preference(value.0.as_ref()),
        Err(error) if error.name() == Some("org.freedesktop.DBus.Error.UnknownMethod") => {
            let result: Result<(Variant<Box<dyn RefArg>>,), _> =
                proxy.method_call(INTERFACE, "Read", (GROUP, "color-scheme"));
            result
                .map(|(value,)| preference(value.0.as_ref()))
                .unwrap_or(PortalPreference::Unavailable)
        }
        Err(_) => PortalPreference::Unavailable,
    }
}
impl Portal {
    pub(super) fn new(notify: Wake) -> std::io::Result<Self> {
        let (receiver, stop) = UnixStream::pair()?;
        let thread = thread::Builder::new()
            .name("lumine-appearance-portal".into())
            .spawn(move || {
                let run = || -> Result<(), Box<dyn std::error::Error>> {
                    let mut channel = Channel::get_private(BusType::Session)?;
                    channel.set_watch_enabled(true);
                    let connection = Connection::from(channel);
                    let callback = notify.clone();
                    let mut rule = MatchRule::new_signal(INTERFACE, "SettingChanged");
                    rule.sender = Some(SERVICE.into());
                    rule.path = Some("/org/freedesktop/portal/desktop".into());
                    connection.add_match(
                        rule,
                        move |(group, key, value): (String, String, Variant<Box<dyn RefArg>>),
                              _,
                              _| {
                            if group == GROUP && key == "color-scheme" {
                                callback(Event::Portal(preference(value.0.as_ref())));
                            }
                            true
                        },
                    )?;
                    let reread = Arc::new(AtomicBool::new(false));
                    let dirty = reread.clone();
                    connection.add_match(
                        MatchRule::new_signal("org.freedesktop.DBus", "NameOwnerChanged"),
                        move |(name, _, _): (String, String, String), _, _| {
                            if name == SERVICE {
                                dirty.store(true, Ordering::Release);
                            }
                            true
                        },
                    )?;
                    // Subscribe before the initial read, so a fast preference change cannot be lost.
                    notify(Event::Portal(read(&connection)));
                    loop {
                        while connection.process(Duration::ZERO)? {}
                        if reread.swap(false, Ordering::AcqRel) {
                            notify(Event::Portal(read(&connection)));
                        }
                        let watch = connection.channel().watch();
                        let mut fds = [
                            libc::pollfd {
                                fd: watch.fd,
                                events: (if watch.read { libc::POLLIN } else { 0 })
                                    | (if watch.write { libc::POLLOUT } else { 0 }),
                                revents: 0,
                            },
                            libc::pollfd {
                                fd: receiver.as_raw_fd(),
                                events: libc::POLLIN,
                                revents: 0,
                            },
                        ];
                        let status = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
                        if fds[1].revents != 0 {
                            return Ok(());
                        }
                        if status < 0
                            && std::io::Error::last_os_error().kind()
                                != std::io::ErrorKind::Interrupted
                        {
                            return Err(std::io::Error::last_os_error().into());
                        }
                    }
                };
                if run().is_err() {
                    notify(Event::Portal(PortalPreference::Unavailable));
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Portal {
    fn drop(&mut self) {
        let _ = self.stop.write_all(&[1]);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
