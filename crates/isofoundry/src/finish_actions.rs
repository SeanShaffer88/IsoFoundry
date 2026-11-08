//! Completion actions after a job (M6): the shared entry point the
//! command line uses for `--shutdown-when-done`. The window side
//! (sound / notification / shutdown for task-window jobs) lives in
//! `win::finish` and reads the same settings.

/// Shut the computer down shortly (announced, cancellable with
/// `shutdown /a`). Windows-only; elsewhere this does nothing.
pub fn shutdown_after_finish(message: &str) -> bool {
    #[cfg(windows)]
    {
        crate::win::finish::shutdown_after_finish(message)
    }
    #[cfg(not(windows))]
    {
        let _ = message;
        false
    }
}
