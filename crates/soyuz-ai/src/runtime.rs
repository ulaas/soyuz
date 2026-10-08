//! The crate's private async runtime and HTTP client. The app is not async: requests run
//! on a small dedicated tokio runtime ("soyuz-ai" threads) and results come back over
//! crossbeam channels, with a `wake` callback so the UI can repaint.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::Notify;

use crate::error::{AiError, AiResult};

/// Repaint / wake-up callback invoked after each delivered event.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// A no-op [`Wake`].
pub fn no_wake() -> Wake {
    Arc::new(|| {})
}

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("soyuz-ai")
            .enable_all()
            .build()
            .expect("cannot start the soyuz-ai runtime")
    })
}

/// Shared HTTP client: rustls with the `ring` provider and the OS certificate verifier.
pub(crate) fn client() -> AiResult<&'static reqwest::Client> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            // Ignore the error: another component may have installed a provider already.
            let _ = rustls::crypto::ring::default_provider().install_default();
            reqwest::Client::builder()
                .user_agent(concat!("soyuz/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(15))
                .pool_idle_timeout(Duration::from_secs(90))
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| AiError::Network(format!("cannot build HTTP client: {e}")))
}

/// Cooperative cancellation shared between the caller and the request task.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<CancelInner>);

#[derive(Default)]
struct CancelInner {
    flag: AtomicBool,
    notify: Notify,
}

impl std::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CancelToken({})", self.is_cancelled())
    }
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.flag.store(true, Ordering::SeqCst);
        self.0.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.flag.load(Ordering::SeqCst)
    }

    /// Resolves once [`cancel`](Self::cancel) was called.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.0.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}
