//! Ownership of a queued clipboard completion. Dropping an accepted request
//! settles cancellation; rejecting an enqueue explicitly disarms the callback.
use std::fmt;

type Complete = Box<dyn FnOnce(Result<(), String>) + Send + 'static>;

pub(crate) struct Write {
    text: Option<String>,
    complete: Option<Complete>,
}
impl Write {
    pub(crate) fn new(
        text: String,
        complete: impl FnOnce(Result<(), String>) + Send + 'static,
    ) -> Self {
        Self {
            text: Some(text),
            complete: Some(Box::new(complete)),
        }
    }
    /// The operation runs on the UI thread. Its borrow/guards are released before
    /// invoking the completion, which may enqueue another native operation.
    pub(crate) fn execute(mut self, operation: impl FnOnce(String) -> Result<(), String>) {
        let result = operation(self.text.take().expect("clipboard request executes once"));
        self.settle(result);
    }
    pub(crate) fn reject(mut self, reason: &str) {
        self.settle(Err(reason.into()));
    }
    /// An enqueue error belongs to the caller of the API; it must not also
    /// arrive asynchronously through the completion callback.
    pub(crate) fn reject_enqueue(mut self) {
        self.complete.take();
    }
    fn settle(&mut self, result: Result<(), String>) {
        if let Some(complete) = self.complete.take() {
            complete(result);
        }
    }
}
impl Drop for Write {
    fn drop(&mut self) {
        self.settle(Err(
            "clipboard write cancelled: window or application closed".into(),
        ));
    }
}
impl fmt::Debug for Write {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClipboardWrite")
            .field("bytes", &self.text.as_ref().map(String::len))
            .field("pending", &self.complete.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    #[test]
    fn success_is_delivered_once_after_the_native_operation_returns() {
        let (sender, receiver) = mpsc::channel();
        let returned = Arc::new(AtomicBool::new(false));
        let observed = returned.clone();
        let request = Write::new("clipboard data".into(), move |result| {
            assert!(observed.load(Ordering::Acquire));
            sender.send(result).unwrap();
        });
        request.execute(|text| {
            assert_eq!(text, "clipboard data");
            assert!(receiver.try_recv().is_err());
            returned.store(true, Ordering::Release);
            Ok(())
        });
        assert_eq!(receiver.recv().unwrap(), Ok(()));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn native_error_is_propagated_once() {
        let (sender, receiver) = mpsc::channel();
        Write::new(String::new(), move |result| sender.send(result).unwrap())
            .execute(|_| Err("native selection ownership failed".into()));
        assert_eq!(
            receiver.recv().unwrap(),
            Err("native selection ownership failed".into())
        );
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn shutdown_or_discarded_queued_event_settles_an_error() {
        let (sender, receiver) = mpsc::channel();
        drop(Write::new(String::new(), move |result| {
            sender.send(result).unwrap()
        }));
        assert!(receiver.recv().unwrap().is_err());
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn closed_window_rejection_does_not_also_complete_on_drop() {
        let (sender, receiver) = mpsc::channel();
        Write::new(String::new(), move |result| sender.send(result).unwrap())
            .reject("window is closed");
        assert_eq!(receiver.recv().unwrap(), Err("window is closed".into()));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn initial_enqueue_failure_does_not_invoke_the_callback() {
        let (sender, receiver) = mpsc::channel();
        Write::new(String::new(), move |result| sender.send(result).unwrap()).reject_enqueue();
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }
}
