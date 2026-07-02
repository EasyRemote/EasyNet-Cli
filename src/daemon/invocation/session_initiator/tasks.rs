/// Aborts the wrapped task when dropped, tying a background loop's
/// lifetime to the owning session attempt.
pub(super) struct AbortOnDrop(pub(super) tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
