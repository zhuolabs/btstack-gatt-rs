use futures_channel::oneshot;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ServerError {
    #[error("{details}")]
    Failed { details: String },
}

fn failed(error: impl std::fmt::Display) -> ServerError {
    ServerError::Failed {
        details: error.to_string(),
    }
}

type Completion = oneshot::Receiver<Result<(), ServerError>>;

/// One USB server session. Kotlin must keep its UsbDeviceConnection open until
/// stop() returns. This object owns only a duplicate of the borrowed descriptor.
#[derive(uniffi::Object)]
pub struct NativeServer {
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
    completion: Mutex<Option<Completion>>,
}

#[uniffi::export]
impl NativeServer {
    #[uniffi::constructor]
    pub fn new(fd: i32) -> Result<Arc<Self>, ServerError> {
        #[cfg(target_os = "android")]
        {
            android::redirect_output().map_err(failed)?;
            if fd < 0 {
                return Err(failed("USB connection is closed"));
            }
            // fcntl rejects invalid integers without constructing a BorrowedFd.
            // Kotlin keeps the connection open and never performs competing I/O.
            let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
            if duplicate < 0 {
                return Err(failed(std::io::Error::last_os_error()));
            }
            use std::os::fd::FromRawFd;
            let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(duplicate) };
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let (tx, rx) = oneshot::channel();
            let worker = std::thread::Builder::new()
                .name("gatt-android".into())
                .spawn(move || {
                    let result = android::serve(owned, &stopped).map_err(failed);
                    if let Err(error) = &result {
                        eprintln!("GATT server failed: {error}");
                    }
                    let _ = tx.send(result);
                })
                .map_err(failed)?;
            Ok(Arc::new(Self {
                stop,
                worker: Mutex::new(Some(worker)),
                completion: Mutex::new(Some(rx)),
            }))
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = fd;
            Err(failed("USB FD server is Android-only"))
        }
    }

    /// Suspends until the server fails or stops. Dropping this future (including
    /// UniFFI coroutine cancellation) stops and joins the native worker.
    pub async fn run(self: Arc<Self>) -> Result<(), ServerError> {
        let completion = self
            .completion
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| failed("run() may only be called once per session"))?;
        let _guard = StopOnDrop(self);
        completion.await.map_err(failed)?
    }

    /// Idempotent barrier: returns only after BTstack and USB workers stop and
    /// the duplicated descriptor is released. Call on Dispatchers.IO.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        // Keep the lock across join so concurrent callers also wait for cleanup.
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
    }
}

struct StopOnDrop(Arc<NativeServer>);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}
impl Drop for NativeServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "android")]
mod android;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
        time::Duration,
    };

    fn session() -> (Arc<NativeServer>, Arc<AtomicBool>) {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let released = Arc::new(AtomicBool::new(false));
        let worker_released = released.clone();
        let (tx, rx) = oneshot::channel();
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            worker_released.store(true, Ordering::Release);
            let _ = tx.send(Ok(()));
        });
        (
            Arc::new(NativeServer {
                stop,
                worker: Mutex::new(Some(worker)),
                completion: Mutex::new(Some(rx)),
            }),
            released,
        )
    }

    #[test]
    fn cancelling_future_joins_worker_before_returning() {
        let (server, released) = session();
        let mut future = Box::pin(server.clone().run());
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(future);
        assert!(released.load(Ordering::Acquire));
        server.stop(); // Repeated cleanup is harmless.
    }

    #[test]
    fn stop_before_first_poll_releases_worker_and_completes() {
        let (server, released) = session();
        server.stop();
        assert!(released.load(Ordering::Acquire));
        let mut future = Box::pin(server.clone().run());
        assert!(matches!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
        let mut duplicate = Box::pin(server.run());
        assert!(matches!(
            duplicate
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(_))
        ));
    }
}
