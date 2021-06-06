use std::future::Future;
use crate::{SpawnStatic, LocalSpawnStatic, SpawnHandleStatic, JoinHandle, LocalSpawnHandleStatic, SpawnBlockingStatic};
use futures_task::SpawnError;

#[derive(Debug, Copy, Clone)]
pub struct Tokio;

impl SpawnStatic for Tokio {
    fn spawn<Output, Fut>(future: Fut) -> Result<(), SpawnError>
    where
        Fut: Future<Output = Output> + Send + 'static,
        Output: Send + 'static,
    {
        let _ = tokio_crate::task::spawn(future);
        Ok(())
    }
}

impl LocalSpawnStatic for Tokio {
    fn spawn_local<Output, Fut>(future: Fut) -> Result<(), SpawnError>
    where
        Fut: Future<Output = Output> + 'static,
        Output: 'static,
    {
        let _ = tokio_crate::task::spawn_local(future);
        Ok(())
    }
}

impl SpawnHandleStatic for Tokio {
    fn spawn_handle<Output, Fut>(future: Fut) -> Result<JoinHandle<Output>, SpawnError>
    where
        Fut: Future<Output = Output> + Send + 'static,
        Output: 'static + Send,
    {
        Ok(JoinHandle::tokio(tokio_crate::task::spawn(future)))
    }
}

impl LocalSpawnHandleStatic for Tokio {
    fn spawn_handle_local<Output, Fut>(future: Fut) -> Result<JoinHandle<Output>, SpawnError>
    where
        Fut: Future<Output = Output> + 'static,
        Output: 'static,
    {
        Ok(JoinHandle::tokio(tokio_crate::task::spawn_local(future)))
    }
}

impl SpawnBlockingStatic for Tokio {
    fn spawn_blocking<T: Send + 'static>(
        func: impl FnOnce() -> T + Send + 'static,
    ) -> Result<JoinHandle<T>, SpawnError> {
        let handle = tokio_crate::task::spawn_blocking(func);
        Ok(JoinHandle::tokio(handle))
    }
}
