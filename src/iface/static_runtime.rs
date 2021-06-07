use crate::JoinHandle;
use futures_task::SpawnError;
use std::fmt::Debug;
use std::future::Future;

pub trait StaticRuntime: Default + Debug + Send + Sync + Copy + Clone + Unpin + 'static {}
impl<T: Default + Debug + Send + Sync + Copy + Clone + Unpin + 'static> StaticRuntime for T {}

/// Let you spawn and get a [JoinHandle] to await the output of a future.
pub trait SpawnHandleStatic: StaticRuntime {
    /// Spawn a future and return a [JoinHandle] that can be awaited for the output of the future.
    fn spawn_handle<Output, Fut>(future: Fut) -> Result<JoinHandle<Output>, SpawnError>
    where
        Fut: Future<Output = Output> + Send + 'static,
        Output: 'static + Send;
}

/// Spawn a blocking task, maybe in a thread pool(tokio), or in current thread and spawns a new thread(std-async)
pub trait SpawnBlockingStatic: StaticRuntime {
    /// spawn a blocking function
    fn spawn_blocking<T: Send + 'static>(
        func: impl FnOnce() -> T + Send + 'static,
    ) -> Result<JoinHandle<T>, SpawnError>;
}

/// Let you spawn and get a [JoinHandle] to await the output of a future.
pub trait LocalSpawnHandleStatic: StaticRuntime {
    /// spawn and get a [JoinHandle] to await the output of a future.
    fn spawn_handle_local<Output, Fut>(future: Fut) -> Result<JoinHandle<Output>, SpawnError>
    where
        Fut: Future<Output = Output> + 'static,
        Output: 'static;
}

/// The `SpawnStatic` trait allows for pushing futures onto an executor that will
/// run them to completion. Except that this is used for ZST as type
pub trait SpawnStatic: StaticRuntime {
    /// Spawns a future that will be run to completion
    fn spawn<Output, Fut>(future: Fut) -> Result<(), SpawnError>
    where
        Fut: Future<Output = Output> + Send + 'static,
        Output: Send + 'static;
}

/// The `LocalSpawnStatic` is similar to [`SpawnStatic`], but allows spawning futures
/// that don't implement `Send`.
pub trait LocalSpawnStatic: StaticRuntime {
    /// Spawns a future that will be run to completion
    fn spawn_local<Output, Fut>(future: Fut) -> Result<(), SpawnError>
    where
        Fut: Future<Output = Output> + 'static,
        Output: 'static;
}
