//! Owned Wayland data-device transport for external file drags.
use std::{
    collections::HashMap,
    io::{ErrorKind, Read},
    os::fd::AsFd,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    protocol::{
        wl_data_device,
        wl_data_device_manager::{self, DndAction},
        wl_data_offer, wl_registry, wl_seat,
    },
};
use winit::{
    event_loop::OwnedDisplayHandle,
    raw_window_handle::{HasDisplayHandle, RawDisplayHandle},
};
pub(crate) type Routes = Arc<Mutex<HashMap<u32, u64>>>;
#[derive(Debug)]
pub(crate) enum FileEvent {
    Hover {
        x: f32,
        y: f32,
        path: PathBuf,
    },
    Leave,
    Rejected {
        x: f32,
        y: f32,
        reason: lumine::input::FileDropError,
    },
    Drop {
        x: f32,
        y: f32,
        paths: Arc<[PathBuf]>,
        accepted: std::sync::mpsc::Sender<bool>,
    },
}
type Handler = Arc<dyn Fn(u64, FileEvent) + Send + Sync>;
pub(crate) struct NativeFiles {
    stop: calloop::channel::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
    // Worker joins before this owned foreign display is released.
    _display: OwnedDisplayHandle,
}
impl NativeFiles {
    pub fn new(
        display: OwnedDisplayHandle,
        routes: Routes,
        handler: Handler,
    ) -> Result<Option<Self>, String> {
        let RawDisplayHandle::Wayland(raw) = display
            .display_handle()
            .map_err(|e| e.to_string())?
            .as_raw()
        else {
            return Ok(None);
        };
        // SAFETY: the owned display remains alive until the worker has joined.
        let backend = unsafe {
            wayland_backend::sys::client::Backend::from_foreign_display(raw.display.as_ptr().cast())
        };
        let connection = Connection::from_backend(backend);
        let (stop, commands) = calloop::channel::channel();
        let worker = thread::Builder::new()
            .name("lumine-wayland-files".into())
            .spawn(move || {
                let queue = connection.new_event_queue::<State>();
                let qh = queue.handle();
                let registry = connection.display().get_registry(&qh, ());
                let mut state = State {
                    routes,
                    handler,
                    manager: None,
                    seats: Vec::new(),
                    offers: HashMap::new(),
                    drag: None,
                    completion: None,
                    exit: false,
                };
                let Ok(mut event_loop) = calloop::EventLoop::<State>::try_new() else {
                    return;
                };
                let handle = event_loop.handle();
                if handle
                    .insert_source(commands, |_, _, state| state.exit = true)
                    .is_err()
                {
                    return;
                }
                if calloop_wayland_source::WaylandSource::new(connection.clone(), queue)
                    .insert(handle)
                    .is_err()
                {
                    return;
                }
                while !state.exit {
                    let timeout = (state.drag.as_ref().is_some_and(|d| d.read.is_some())
                        || state.completion.is_some())
                    .then_some(Duration::from_millis(20));
                    if event_loop.dispatch(timeout, &mut state).is_err() {
                        break;
                    }
                    state.read_transfer();
                }
                state.cancel();
                for (_, offer) in state.offers.drain() {
                    offer.proxy.destroy();
                }
                for (_, seat, device) in state.seats.drain(..) {
                    if let Some(device) = device
                        && device.version() >= 2
                    {
                        device.release();
                    }
                    if seat.version() >= 5 {
                        seat.release();
                    }
                }
                if let Some(manager) = state.manager.take() {
                    drop(manager);
                }
                drop(registry);
                let _ = connection.flush();
            })
            .map_err(|e| e.to_string())?;
        Ok(Some(Self {
            stop,
            worker: Some(worker),
            _display: display,
        }))
    }
}
impl Drop for NativeFiles {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
struct Offer {
    proxy: wl_data_offer::WlDataOffer,
    uris: bool,
}
struct Transfer {
    stream: UnixStream,
    bytes: Vec<u8>,
    deadline: Instant,
}
struct Drag {
    offer: wl_data_offer::WlDataOffer,
    token: u64,
    x: f32,
    y: f32,
    read: Option<Transfer>,
    paths: Option<Arc<[PathBuf]>>,
    dropped: bool,
    copy: bool,
    left: bool,
}
struct Completion {
    offer: wl_data_offer::WlDataOffer,
    accepted: std::sync::mpsc::Receiver<bool>,
    deadline: Instant,
}
struct State {
    routes: Routes,
    handler: Handler,
    manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    seats: Vec<(u32, wl_seat::WlSeat, Option<wl_data_device::WlDataDevice>)>,
    offers: HashMap<u32, Offer>,
    drag: Option<Drag>,
    completion: Option<Completion>,
    exit: bool,
}
impl State {
    fn devices(&mut self, qh: &QueueHandle<Self>) {
        if let Some(manager) = &self.manager {
            for (_, seat, device) in &mut self.seats {
                if device.is_none() {
                    *device = Some(manager.get_data_device(seat, qh, ()));
                }
            }
        }
    }
    fn cancel(&mut self) {
        if let Some(done) = self.completion.take() {
            done.offer.destroy();
        }
        if let Some(drag) = self.drag.take() {
            if !drag.left {
                (self.handler)(drag.token, FileEvent::Leave);
            }
            drag.offer.destroy();
        }
    }
    fn reject(&mut self, reason: lumine::input::FileDropError) {
        if let Some(drag) = &self.drag {
            (self.handler)(
                drag.token,
                FileEvent::Rejected {
                    x: drag.x,
                    y: drag.y,
                    reason,
                },
            );
        }
        self.cancel();
    }
    fn read_transfer(&mut self) {
        if let Some(done) = &self.completion {
            let accepted = match done.accepted.try_recv() {
                Ok(value) => Some(value),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(false),
                Err(_) => (Instant::now() > done.deadline).then_some(false),
            };
            if let Some(accepted) = accepted {
                let done = self.completion.take().unwrap();
                if accepted && done.offer.version() >= 3 {
                    done.offer.finish();
                }
                done.offer.destroy();
            }
        }
        let Some(drag) = &mut self.drag else { return };
        if !self
            .routes
            .lock()
            .unwrap()
            .values()
            .any(|token| *token == drag.token)
        {
            self.cancel();
            return;
        }
        let Some(read) = &mut drag.read else { return };
        match read.poll() {
            Ok(Some(bytes)) => {
                drag.read = None;
                match decode_uris(&bytes) {
                    Ok(paths) => {
                        if !drag.left {
                            for path in &paths {
                                (self.handler)(
                                    drag.token,
                                    FileEvent::Hover {
                                        x: drag.x,
                                        y: drag.y,
                                        path: path.clone(),
                                    },
                                );
                            }
                        }
                        drag.paths = Some(paths.into());
                        self.finish();
                    }
                    Err(reason) => self.reject(reason),
                }
            }
            Err(reason) => self.reject(reason),
            Ok(None) => {}
        }
    }
    fn finish(&mut self) {
        if !self
            .drag
            .as_ref()
            .is_some_and(|d| d.dropped && d.paths.is_some())
        {
            return;
        }
        let drag = self.drag.take().unwrap();
        if drag.copy {
            let (accepted, reply) = std::sync::mpsc::channel();
            (self.handler)(
                drag.token,
                FileEvent::Drop {
                    x: drag.x,
                    y: drag.y,
                    paths: drag.paths.unwrap(),
                    accepted,
                },
            );
            // Only the UI's accepted drop completes the protocol transaction.
            // Closing the owner, rejecting the region, or losing the callback
            // destroys the offer without falsely reporting successful receipt.
            self.completion = Some(Completion {
                offer: drag.offer,
                accepted: reply,
                deadline: Instant::now() + Duration::from_secs(3),
            });
        } else {
            drag.offer.destroy();
        }
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                if interface == "wl_data_device_manager" {
                    state.manager = Some(registry.bind(name, version.min(3), qh, ()));
                }
                if interface == "wl_seat" {
                    state
                        .seats
                        .push((name, registry.bind(name, version.min(7), qh, ()), None));
                }
                state.devices(qh);
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(index) = state.seats.iter().position(|(id, _, _)| *id == name) {
                    state.cancel();
                    let (_, seat, device) = state.seats.swap_remove(index);
                    if let Some(device) = device
                        && device.version() >= 2
                    {
                        device.release();
                    }
                    if seat.version() >= 5 {
                        seat.release();
                    }
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_data_device_manager::WlDataDeviceManager,
        _: wl_data_device_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_data_offer::WlDataOffer, ()> for State {
    fn event(
        state: &mut Self,
        offer: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                if let Some(saved) = state.offers.get_mut(&offer.id().protocol_id()) {
                    saved.uris |= mime_type == "text/uri-list";
                }
            }
            wl_data_offer::Event::Action { dnd_action } => {
                if let Some(drag) = &mut state.drag
                    && drag.offer == *offer
                {
                    drag.copy = dnd_action.into_result().is_ok_and(|a| a == DndAction::Copy);
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    wayland_client::event_created_child!(State,wl_data_device::WlDataDevice,[0=>(wl_data_offer::WlDataOffer,())]);
    fn event(
        state: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        connection: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                if state.offers.len() >= 64 {
                    for (_, offer) in state.offers.drain() {
                        offer.proxy.destroy();
                    }
                }
                state.offers.insert(
                    id.id().protocol_id(),
                    Offer {
                        proxy: id,
                        uris: false,
                    },
                );
            }
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id,
            } => {
                state.cancel();
                let Some(offer) = id.and_then(|id| state.offers.remove(&id.id().protocol_id()))
                else {
                    return;
                };
                let token = state
                    .routes
                    .lock()
                    .unwrap()
                    .get(&surface.id().protocol_id())
                    .copied();
                let Some(token) = token.filter(|_| offer.uris) else {
                    offer.proxy.accept(serial, None);
                    offer.proxy.destroy();
                    return;
                };
                let Ok((read, write)) = UnixStream::pair() else {
                    offer.proxy.destroy();
                    return;
                };
                if read.set_nonblocking(true).is_err() {
                    offer.proxy.destroy();
                    return;
                }
                offer.proxy.accept(serial, Some("text/uri-list".into()));
                if offer.proxy.version() >= 3 {
                    offer.proxy.set_actions(DndAction::Copy, DndAction::Copy);
                }
                offer.proxy.receive("text/uri-list".into(), write.as_fd());
                drop(write);
                let copy = offer.proxy.version() < 3;
                state.drag = Some(Drag {
                    offer: offer.proxy,
                    token,
                    x: x as f32,
                    y: y as f32,
                    read: Some(Transfer {
                        stream: read,
                        bytes: Vec::new(),
                        deadline: Instant::now() + Duration::from_secs(3),
                    }),
                    paths: None,
                    dropped: false,
                    copy,
                    left: false,
                });
                let _ = connection.flush();
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if let Some(drag) = &mut state.drag {
                    drag.x = x as f32;
                    drag.y = y as f32;
                    if let Some(paths) = &drag.paths
                        && let Some(path) = paths.first()
                    {
                        (state.handler)(
                            drag.token,
                            FileEvent::Hover {
                                x: drag.x,
                                y: drag.y,
                                path: path.clone(),
                            },
                        );
                    }
                }
            }
            wl_data_device::Event::Drop => {
                if let Some(drag) = &mut state.drag {
                    drag.dropped = true;
                }
                state.finish();
            }
            wl_data_device::Event::Leave => {
                if let Some(drag) = &mut state.drag {
                    (state.handler)(drag.token, FileEvent::Leave);
                    drag.left = true;
                    if !drag.dropped {
                        state.cancel();
                    }
                }
            }
            wl_data_device::Event::Selection { id } => {
                if let Some(offer) = id.and_then(|id| state.offers.remove(&id.id().protocol_id())) {
                    offer.proxy.destroy();
                }
            }
            _ => {}
        }
    }
}
impl Transfer {
    fn poll(&mut self) -> Result<Option<Vec<u8>>, lumine::input::FileDropError> {
        use lumine::input::FileDropError;
        if Instant::now() > self.deadline {
            return Err(FileDropError::TimedOut);
        }
        let mut buffer = [0u8; 8192];
        loop {
            match self.stream.read(&mut buffer) {
                Ok(0) => return Ok(Some(std::mem::take(&mut self.bytes))),
                Ok(n) => {
                    if self.bytes.len() + n > 1024 * 1024 {
                        return Err(FileDropError::TooLarge);
                    }
                    self.bytes.extend_from_slice(&buffer[..n]);
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(None),
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return Err(FileDropError::InvalidData),
            }
        }
    }
}
fn decode_uris(bytes: &[u8]) -> Result<Vec<PathBuf>, lumine::input::FileDropError> {
    use lumine::input::FileDropError;
    let text = std::str::from_utf8(bytes).map_err(|_| FileDropError::InvalidData)?;
    let mut paths = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        if paths.len() == 1024 {
            return Err(FileDropError::TooManyFiles);
        }
        paths.push(crate::portal_dialog::file_uri(line).ok_or(FileDropError::InvalidData)?);
    }
    if paths.is_empty() {
        Err(FileDropError::InvalidData)
    } else {
        Ok(paths)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use lumine::input::FileDropError;
    #[test]
    fn uri_list_is_bounded_and_all_or_nothing() {
        assert_eq!(
            decode_uris(b"# files\r\nfile:///tmp/a\r\nfile:///tmp/second%20file.txt\r\n").unwrap(),
            [
                PathBuf::from("/tmp/a"),
                PathBuf::from("/tmp/second file.txt")
            ]
        );
        for bad in [
            &b"file:///good\nhttps://remote/file"[..],
            b"file:///bad%00path",
            b"# comment only",
            b"file:///bad%",
        ] {
            assert_eq!(decode_uris(bad), Err(FileDropError::InvalidData));
        }
        assert_eq!(
            decode_uris("file:///tmp/a\n".repeat(1025).as_bytes()),
            Err(FileDropError::TooManyFiles)
        );
    }
    #[test]
    fn stalled_oversized_and_disconnected_streams_do_not_block() {
        use std::io::Write;
        let (read, mut write) = UnixStream::pair().unwrap();
        read.set_nonblocking(true).unwrap();
        let mut transfer = Transfer {
            stream: read,
            bytes: Vec::new(),
            deadline: Instant::now() + Duration::from_secs(3),
        };
        assert!(transfer.poll().unwrap().is_none());
        transfer.deadline = Instant::now() - Duration::from_millis(1);
        assert_eq!(transfer.poll(), Err(FileDropError::TimedOut));
        transfer.deadline = Instant::now() + Duration::from_secs(3);
        transfer.bytes.resize(1024 * 1024, b'a');
        write.write_all(b"x").unwrap();
        assert_eq!(transfer.poll(), Err(FileDropError::TooLarge));
        transfer.bytes.clear();
        write.write_all(b"file:///truncated%").unwrap();
        drop(write);
        let bytes = transfer.poll().unwrap().unwrap();
        assert_eq!(decode_uris(&bytes), Err(FileDropError::InvalidData));
    }
}
