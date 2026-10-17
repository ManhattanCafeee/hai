//! A spawned task held to the scope that owns it.

use tokio::task::JoinHandle;

/// One supervised task: dropping the guard aborts it, so a generation that forgets to stop a task
/// cannot leave it running into the next one.
pub(crate) struct TaskGuard(JoinHandle<()>);

impl TaskGuard {
    pub(crate) fn spawn<F>(future: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Self(tokio::spawn(future))
    }

    /// Takes over a task someone else spawned.
    pub(crate) fn wrap(task: JoinHandle<()>) -> Self {
        Self(task)
    }

    /// The task itself, for the `select!` that also watches it end.
    pub(crate) fn handle(&mut self) -> &mut JoinHandle<()> {
        &mut self.0
    }

    /// Waits for the task to end on its own.
    pub(crate) async fn join(mut self) {
        let _ = (&mut self.0).await;
    }
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}
