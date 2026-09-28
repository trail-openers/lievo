//! In-process refresh guard shared by the two background refresh paths
//! (`ensure_fresh` callers) — see `refresh::REFRESH_IN_PROGRESS`.

use std::sync::atomic::Ordering;

/// RAII guard for the `REFRESH_IN_PROGRESS` flag.
/// Sets the flag on construction (via `try_acquire`), clears it on drop.
/// Drop runs unconditionally, even during unwind.
pub struct RefreshGuard;

impl RefreshGuard {
    /// Attempt to acquire the refresh lock.
    /// Returns `Some(guard)` if the lock was free; `None` if already held.
    pub fn try_acquire() -> Option<Self> {
        super::REFRESH_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| RefreshGuard)
    }
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        super::REFRESH_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Mutex to serialize tests that touch the global REFRESH_IN_PROGRESS flag.
    // Without this, parallel test execution interferes with guard state.
    use super::super::REFRESH_IN_PROGRESS;
    use std::sync::atomic::Ordering;

    static GUARD_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_refresh_guard_clears_flag_on_drop() {
        let _lock = GUARD_TEST_LOCK.lock().unwrap();

        // Reset flag to ensure clean state
        REFRESH_IN_PROGRESS.store(false, Ordering::SeqCst);

        // Initially flag should be false
        assert!(
            !REFRESH_IN_PROGRESS.load(Ordering::SeqCst),
            "flag should start false"
        );

        // Acquire the guard
        let guard = RefreshGuard::try_acquire();
        assert!(guard.is_some(), "should acquire guard when flag is false");
        assert!(
            REFRESH_IN_PROGRESS.load(Ordering::SeqCst),
            "flag should be true while guard is held"
        );

        // Drop the guard
        drop(guard);
        assert!(
            !REFRESH_IN_PROGRESS.load(Ordering::SeqCst),
            "flag should be false after guard drops"
        );
    }

    #[test]
    fn test_refresh_guard_prevents_concurrent_acquisition() {
        let _lock = GUARD_TEST_LOCK.lock().unwrap();

        // Reset flag to ensure clean state
        REFRESH_IN_PROGRESS.store(false, Ordering::SeqCst);

        // Acquire first guard
        let guard1 = RefreshGuard::try_acquire();
        assert!(guard1.is_some(), "first guard should acquire");

        // Attempt to acquire second guard while first is held
        let guard2 = RefreshGuard::try_acquire();
        assert!(
            guard2.is_none(),
            "second guard should fail while first is held"
        );

        // Drop first guard
        drop(guard1);

        // Now we should be able to acquire again
        let guard3 = RefreshGuard::try_acquire();
        assert!(guard3.is_some(), "should acquire after first guard dropped");

        drop(guard3);
    }

    #[test]
    fn test_refresh_guard_clears_on_panic() {
        let _lock = GUARD_TEST_LOCK.lock().unwrap();

        // This test verifies that the guard clears the flag even during panic.
        // We use std::panic::catch_unwind to swallow the panic.

        // Reset flag to false for this test
        REFRESH_IN_PROGRESS.store(false, Ordering::SeqCst);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = RefreshGuard::try_acquire();
            assert!(
                REFRESH_IN_PROGRESS.load(Ordering::SeqCst),
                "flag should be true while guard is held"
            );

            // Panic inside the guard scope
            panic!("simulated panic to test guard drops on unwind");
        }));

        // The panic should have been caught
        assert!(
            result.is_err(),
            "panic should have been caught by catch_unwind"
        );

        // After the panic (and guard drop), flag should be false
        assert!(
            !REFRESH_IN_PROGRESS.load(Ordering::SeqCst),
            "flag should be cleared even after panic inside guard scope"
        );
    }

    #[test]
    fn test_background_thread_panic_clears_flag() {
        let _lock = GUARD_TEST_LOCK.lock().unwrap();

        // Reset flag to false
        REFRESH_IN_PROGRESS.store(false, Ordering::SeqCst);

        // Spawn a thread that panics while holding the guard
        let handle = std::thread::spawn(|| {
            let _guard =
                RefreshGuard::try_acquire().expect("guard should be available in spawned thread");
            panic!("intentional panic in background thread");
        });

        // Join the thread and let it panic
        let _result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = handle.join();
        }));

        // Regardless of whether we caught the panic, the flag should be clear
        // because the guard dropped during unwind
        assert!(
            !super::super::REFRESH_IN_PROGRESS.load(Ordering::SeqCst),
            "flag should be cleared after background thread panics"
        );
    }
}
