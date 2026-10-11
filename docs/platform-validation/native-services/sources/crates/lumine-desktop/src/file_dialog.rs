//! Window-owned native file selection. Selecting a path never reads or writes it.
use std::{
    cell::{Cell, RefCell},
    fmt,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Weak},
};
use std::{
    future::Future,
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll, Waker},
};
use winit::window::Window;

/// An extension filter, without a leading dot (for example `png` or `rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFilter {
    pub name: String,
    pub extensions: Vec<String>,
}
impl FileFilter {
    pub fn new(
        name: impl Into<String>,
        extensions: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            name: name.into(),
            extensions: extensions.into_iter().map(Into::into).collect(),
        }
    }
}
/// Common settings for opening files or folders and choosing a save destination.
/// Filters affect files only; extension/overwrite behavior follows the native dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDialogOptions {
    pub title: String,
    pub directory: Option<PathBuf>,
    pub file_name: Option<String>,
    pub filters: Vec<FileFilter>,
}
impl FileDialogOptions {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }
    pub fn directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directory = Some(directory.into());
        self
    }
    pub fn file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }
    pub fn filter(mut self, filter: FileFilter) -> Self {
        self.filters.push(filter);
        self
    }
    fn validate(&self) -> Result<(), FileDialogError> {
        if self.title.contains('\0')
            || self.file_name.as_ref().is_some_and(|s| s.contains('\0'))
            || self.filters.iter().any(|f| {
                f.name.contains('\0')
                    || f.extensions.is_empty()
                    || f.extensions
                        .iter()
                        .any(|s| s.contains(['\0', '/', '\\']) || s.starts_with('.'))
            })
        {
            return Err(FileDialogError::InvalidOptions);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogError {
    /// The owning window has closed.
    WindowClosed,
    /// The native window is not created yet, or is currently suspended.
    WindowUnavailable,
    InvalidOptions,
    /// The native worker could not be started or terminated unexpectedly.
    BackendUnavailable,
}
impl fmt::Display for FileDialogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WindowClosed => "the dialog's window is closed",
            Self::WindowUnavailable => "the dialog's native window is unavailable",
            Self::InvalidOptions => "invalid native file dialog options",
            Self::BackendUnavailable => "the native file dialog backend is unavailable",
        })
    }
}
impl std::error::Error for FileDialogError {}
/// `None` means no selection. Linux reports portal failures as errors; the macOS
/// backend may also return no selection when a native picker is unavailable.
pub type FileDialogResult<T> = Result<Option<T>, FileDialogError>;
#[derive(Clone, Copy)]
pub(crate) enum Kind {
    OpenFile,
    OpenFiles,
    Folder,
    Folders,
    Save,
}

/// Cloneable service for one window. Invoke from its UI executor after native creation.
/// Linux uses an owned XDG Desktop Portal request; macOS uses native sheets.
///
/// On Linux, dropping a file-picker future or closing its owner requests portal
/// dismissal and releases the native parent. Prompts and macOS sheets cancel result
/// delivery; physical dismissal follows their native backend.
#[derive(Clone)]
pub struct FileDialogs {
    owner: crate::WindowHandle,
    native: Rc<RefCell<Option<Weak<Window>>>>,
    generation: Rc<Cell<u64>>,
    cancellations: Rc<RefCell<Vec<Weak<std::sync::atomic::AtomicBool>>>>,
}
impl FileDialogs {
    pub(crate) fn new(owner: crate::WindowHandle) -> Self {
        Self {
            owner,
            native: Rc::new(RefCell::new(None)),
            generation: Rc::new(Cell::new(0)),
            cancellations: Rc::new(RefCell::new(Vec::new())),
        }
    }
    pub(crate) fn attach(&self, window: Option<&Arc<Window>>) {
        for token in self
            .cancellations
            .borrow_mut()
            .drain(..)
            .filter_map(|w| w.upgrade())
        {
            token.store(true, std::sync::atomic::Ordering::Release);
        }
        self.generation.set(self.generation.get().wrapping_add(1));
        *self.native.borrow_mut() = window.map(Arc::downgrade);
    }
    async fn select(
        &self,
        kind: Kind,
        options: FileDialogOptions,
    ) -> FileDialogResult<Vec<PathBuf>> {
        options.validate()?;
        if self.owner.is_closed() {
            return Err(FileDialogError::WindowClosed);
        }
        let parent = self
            .native
            .borrow()
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or(FileDialogError::WindowUnavailable)?;
        let generation = self.generation.get();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.cancellations
            .borrow_mut()
            .retain(|w| w.strong_count() > 0);
        self.cancellations
            .borrow_mut()
            .push(Arc::downgrade(&cancel));
        let _cancel_on_drop = CancelOnDrop(cancel.clone());
        let result = native_select(kind, options, parent, cancel).await?;
        if self.owner.is_closed() {
            return Err(FileDialogError::WindowClosed);
        }
        if generation != self.generation.get() {
            return Err(FileDialogError::WindowUnavailable);
        }
        Ok(result)
    }
    /// Show a native prompt; cancellation and owner-generation rules match file selection.
    pub async fn prompt(
        &self,
        options: crate::PromptOptions,
    ) -> Result<crate::PromptResponse, FileDialogError> {
        options.validate()?;
        if self.owner.is_closed() {
            return Err(FileDialogError::WindowClosed);
        }
        let parent = self
            .native
            .borrow()
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or(FileDialogError::WindowUnavailable)?;
        let generation = self.generation.get();
        let response = crate::native_prompt::show(parent, options).await?;
        if self.owner.is_closed() {
            return Err(FileDialogError::WindowClosed);
        }
        if generation != self.generation.get() {
            return Err(FileDialogError::WindowUnavailable);
        }
        Ok(response)
    }
    pub async fn open_file(&self, options: FileDialogOptions) -> FileDialogResult<PathBuf> {
        Ok(self
            .select(Kind::OpenFile, options)
            .await?
            .and_then(|p| p.into_iter().next()))
    }
    pub async fn open_files(&self, options: FileDialogOptions) -> FileDialogResult<Vec<PathBuf>> {
        self.select(Kind::OpenFiles, options).await
    }
    pub async fn open_folder(&self, options: FileDialogOptions) -> FileDialogResult<PathBuf> {
        Ok(self
            .select(Kind::Folder, options)
            .await?
            .and_then(|p| p.into_iter().next()))
    }
    pub async fn open_folders(&self, options: FileDialogOptions) -> FileDialogResult<Vec<PathBuf>> {
        self.select(Kind::Folders, options).await
    }
    /// Choose a destination; this does not create, truncate, or write a file.
    pub async fn save_file(&self, options: FileDialogOptions) -> FileDialogResult<PathBuf> {
        Ok(self
            .select(Kind::Save, options)
            .await?
            .and_then(|p| p.into_iter().next()))
    }
}

struct CancelOnDrop(Arc<std::sync::atomic::AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}
#[cfg(target_os = "linux")]
async fn native_select(
    kind: Kind,
    options: FileDialogOptions,
    parent: Arc<Window>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> FileDialogResult<Vec<PathBuf>> {
    background(move || crate::portal_dialog::select(kind, options, parent, cancel))?.await?
}
#[cfg(target_os = "macos")]
async fn native_select(
    kind: Kind,
    options: FileDialogOptions,
    parent: Arc<Window>,
    _cancel: Arc<std::sync::atomic::AtomicBool>,
) -> FileDialogResult<Vec<PathBuf>> {
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_parent(parent.as_ref())
        .set_title(options.title);
    if let Some(directory) = options.directory {
        dialog = dialog.set_directory(directory);
    }
    if let Some(name) = options.file_name {
        dialog = dialog.set_file_name(name);
    }
    for filter in options.filters {
        dialog = dialog.add_filter(filter.name, &filter.extensions);
    }
    let result = match kind {
        Kind::OpenFile => dialog.pick_file().await.map(|p| vec![p]),
        Kind::OpenFiles => dialog.pick_files().await,
        Kind::Folder => dialog.pick_folder().await.map(|p| vec![p]),
        Kind::Folders => dialog.pick_folders().await,
        Kind::Save => dialog.save_file().await.map(|p| vec![p]),
    };
    Ok(result.map(|files| files.into_iter().map(|f| f.path().to_owned()).collect()))
}

struct Completion<T> {
    result: Option<Result<T, FileDialogError>>,
    waker: Option<Waker>,
}
pub(crate) struct Background<T>(Arc<Mutex<Completion<T>>>);
pub(crate) fn background<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<Background<T>, FileDialogError> {
    let state = Arc::new(Mutex::new(Completion {
        result: None,
        waker: None,
    }));
    let sender = state.clone();
    std::thread::Builder::new()
        .name("lumine-file-dialog".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .map_err(|_| FileDialogError::BackendUnavailable);
            let waker = {
                let mut state = sender.lock().unwrap();
                state.result = Some(result);
                state.waker.take()
            };
            if let Some(waker) = waker {
                waker.wake();
            }
        })
        .map_err(|_| FileDialogError::BackendUnavailable)?;
    Ok(Background(state))
}
impl<T> Future for Background<T> {
    type Output = Result<T, FileDialogError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.0.lock().unwrap();
        if let Some(result) = state.result.take() {
            Poll::Ready(result)
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_filter_does_not_reach_native_cstring_conversion() {
        for filter in [
            FileFilter::new("bad\0name", ["rs"]),
            FileFilter::new("Rust", ["r\0s"]),
            FileFilter::new("Rust", [".rs"]),
        ] {
            assert_eq!(
                FileDialogOptions::default().filter(filter).validate(),
                Err(FileDialogError::InvalidOptions)
            );
        }
        assert!(
            FileDialogOptions::new("Open")
                .filter(FileFilter::new("Rust", ["rs", "toml"]))
                .validate()
                .is_ok()
        );
    }
    #[test]
    fn cancelled_receiver_releases_worker_owned_resources_after_completion() {
        let retained = Arc::new(());
        let worker = retained.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let future = background(move || {
            let _keep = worker;
            rx.recv().unwrap();
        })
        .unwrap();
        drop(future);
        assert_eq!(Arc::strong_count(&retained), 2);
        tx.send(()).unwrap();
        for _ in 0..100 {
            if Arc::strong_count(&retained) == 1 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("native worker retained its parent after completion");
    }
}
