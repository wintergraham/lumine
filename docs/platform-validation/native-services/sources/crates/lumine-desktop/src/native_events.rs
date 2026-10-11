//! Owned operating-system application activation and open-URL callbacks.
use std::{fmt, sync::Arc};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationEvent {
    OpenUrls(Vec<String>),
    Reopen,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationEventError(pub String);
impl fmt::Display for ApplicationEventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ApplicationEventError {}
pub(crate) type Handler = Arc<dyn Fn(ApplicationEvent) + Send + Sync>;
pub(crate) fn validate_id(id: &str) -> Result<(), ApplicationEventError> {
    let pieces: Vec<_> = id.split('.').collect();
    if id.len() > 255
        || pieces.len() < 2
        || pieces.iter().any(|piece| {
            piece.is_empty()
                || !piece.as_bytes()[0].is_ascii_alphabetic()
                || !piece
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        })
    {
        Err(ApplicationEventError(
            "application ID must be a reverse-DNS identifier".into(),
        ))
    } else {
        Ok(())
    }
}
#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use dbus::{
        arg::PropMap,
        blocking::{Connection, stdintf::org_freedesktop_dbus::RequestNameReply},
        channel::MatchingReceiver,
        message::MatchRule,
    };
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };
    pub(crate) struct NativeApplicationEvents {
        stop: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }
    impl NativeApplicationEvents {
        pub(crate) fn new(id: &str, handler: Handler) -> Result<Self, ApplicationEventError> {
            validate_id(id)?;
            let id = id.to_owned();
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let (ready, started) = mpsc::sync_channel(1);
            let thread=thread::Builder::new().name("lumine-application-events".into()).spawn(move || {
                let setup=(||->Result<_,ApplicationEventError>{
                    let connection=Connection::new_session().map_err(|e|ApplicationEventError(e.to_string()))?;
                    let reply=connection.request_name(&id,false,false,true).map_err(|e|ApplicationEventError(e.to_string()))?;
                    if reply!=RequestNameReply::PrimaryOwner {return Err(ApplicationEventError(format!("application ID {id} is already owned")))}
                    let mut routes=dbus_crossroads::Crossroads::new();
                    let interface=routes.register("org.freedesktop.Application",|builder| {
                        builder.method("Activate",("platform_data",),(),|_:&mut dbus_crossroads::Context,handler:&mut Handler,(_,): (PropMap,)| {
                            handler(ApplicationEvent::Reopen);Ok(())
                        });
                        builder.method("Open",("uris","platform_data"),(),|_:&mut dbus_crossroads::Context,handler:&mut Handler,(urls,_):(Vec<String>,PropMap)| {
                            if urls.len()>256 || urls.iter().any(|url|url.len()>65536 || !crate::url::valid_url(url)) {
                                return Err(dbus::MethodErr::invalid_arg("invalid URL batch"));
                            }
                            handler(ApplicationEvent::OpenUrls(urls));Ok(())
                        });
                    });
                    let path=format!("/{}",id.replace('.',"/").replace('-',"_"));
                    routes.insert(path,&[interface],handler);
                    connection.start_receive(MatchRule::new_method_call(),Box::new(move |message,connection| {
                        let _=routes.handle_message(message,connection);true
                    }));
                    Ok(connection)
                })();
                match setup {
                    Ok(connection)=>{
                        let _=ready.send(Ok(()));
                        while !stopped.load(Ordering::Acquire) {
                            if connection.process(Duration::from_millis(100)).is_err(){break}
                        }
                    }
                    Err(error)=>{let _=ready.send(Err(error));}
                }
            }).map_err(|e|ApplicationEventError(e.to_string()))?;
            match started.recv_timeout(Duration::from_secs(3)) {
                Ok(Ok(())) => Ok(Self {
                    stop,
                    thread: Some(thread),
                }),
                result => {
                    stop.store(true, Ordering::Release);
                    Err(match result {
                        Ok(Err(error)) => error,
                        _ => ApplicationEventError(
                            "application event service startup timed out".into(),
                        ),
                    })
                }
            }
        }
    }
    impl Drop for NativeApplicationEvents {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}
#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use objc2::{
        DeclaredClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
        runtime::ProtocolObject,
    };
    use objc2_app_kit::{NSApplication, NSApplicationDelegate};
    use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSURL};
    struct State {
        handler: Handler,
    }
    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = State]
        struct Delegate;
        unsafe impl NSObjectProtocol for Delegate {}
        unsafe impl NSApplicationDelegate for Delegate {
            #[unsafe(method(application:openURLs:))]
            fn open_urls(&self, _: &NSApplication, urls: &NSArray<NSURL>) {
                let urls = urls
                    .iter()
                    .filter_map(|url| url.absoluteString().map(|s| s.to_string()))
                    .collect();
                (self.ivars().handler)(ApplicationEvent::OpenUrls(urls));
            }
            #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
            fn reopen(&self, _: &NSApplication, _: bool) -> bool {
                (self.ivars().handler)(ApplicationEvent::Reopen);
                true
            }
        }
    );
    pub(crate) struct NativeApplicationEvents {
        app: Retained<NSApplication>,
        _delegate: Retained<Delegate>,
    }
    impl NativeApplicationEvents {
        pub(crate) fn new(id: &str, handler: Handler) -> Result<Self, ApplicationEventError> {
            validate_id(id)?;
            let mtm = MainThreadMarker::new().ok_or_else(|| {
                ApplicationEventError("application delegate requires the main thread".into())
            })?;
            let app = NSApplication::sharedApplication(mtm);
            if app.delegate().is_some() {
                return Err(ApplicationEventError(
                    "an NSApplication delegate is already registered".into(),
                ));
            }
            let delegate = Delegate::alloc(mtm).set_ivars(State { handler });
            let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
            app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            Ok(Self {
                app,
                _delegate: delegate,
            })
        }
    }
    impl Drop for NativeApplicationEvents {
        fn drop(&mut self) {
            self.app.setDelegate(None);
        }
    }
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) use platform::NativeApplicationEvents;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_ids_are_valid_bus_names_and_paths() {
        assert!(validate_id("org.example.Editor").is_ok());
        for id in ["single", "org..app", "org.9app", "org.app/bad", "org.app\0"] {
            assert!(validate_id(id).is_err());
        }
    }
}
