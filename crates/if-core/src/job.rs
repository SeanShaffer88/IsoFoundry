//! Job primitives shared by every task: cooperative cancellation and progress.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Cloneable cancellation flag. Long loops call [`CancelToken::check`] at
/// safe points; nothing is ever killed mid-write.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Returns `Err(Cancelled)` once cancellation was requested.
    pub fn check(&self) -> crate::error::Result<()> {
        if self.is_cancelled() {
            Err(crate::error::Error::new(crate::error::Exit::Cancelled, "cancelled"))
        } else {
            Ok(())
        }
    }
}

/// Byte progress observed by a running job. Values are only ever reported
/// from real work; no ETA is invented.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

impl Progress {
    pub fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }

    pub fn add(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
    }

    /// `(done, total)`; total 0 means unknown.
    pub fn snapshot(&self) -> (u64, u64) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }

    /// Fraction 0..=1 when the total is known.
    pub fn fraction(&self) -> Option<f32> {
        let (d, t) = self.snapshot();
        (t > 0).then(|| (d.min(t) as f64 / t as f64) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_is_shared_between_clones() {
        let a = CancelToken::new();
        let b = a.clone();
        assert!(b.check().is_ok());
        a.cancel();
        assert_eq!(b.check().unwrap_err().exit, crate::error::Exit::Cancelled);
    }

    #[test]
    fn progress_fraction() {
        let p = Progress::default();
        assert_eq!(p.fraction(), None);
        p.set_total(200);
        p.add(50);
        assert_eq!(p.fraction(), Some(0.25));
        p.add(500);
        assert_eq!(p.fraction(), Some(1.0));
    }
}
