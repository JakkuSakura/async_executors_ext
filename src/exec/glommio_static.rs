use crate::{
    JoinHandle, LocalSpawnHandleStatic, LocalSpawnStatic, SpawnBlockingStatic, SpawnHandleStatic,
    SpawnStatic, YieldNow,
};
use futures_task::SpawnError;
use futures_util::FutureExt;
use glommio_crate::Task;
use nix::sched::CpuSet;
use std::future::Future;

/// A simple glommio runtime builder
#[derive(Debug, Clone, Copy, Default)]
pub struct Glommio;

impl LocalSpawnStatic for Glommio {
    fn spawn_local<Output, Fut>(future: Fut) -> Result<(), SpawnError>
    where
        Fut: Future<Output = Output> + 'static,
        Output: 'static,
    {
        glommio_crate::Task::local(future).detach();
        Ok(())
    }
}

impl LocalSpawnHandleStatic for Glommio {
    fn spawn_handle_local<Output, Fut>(future: Fut) -> Result<JoinHandle<Output>, SpawnError>
    where
        Fut: Future<Output = Output> + 'static,
        Output: 'static,
    {
        let (remote, handle) = future.remote_handle();
        Task::local(remote).detach();
        Ok(JoinHandle::remote_handle(handle))
    }
}

impl SpawnStatic for Glommio {
    fn spawn<Output, Fut>(future: Fut) -> Result<(), SpawnError>
    where
        Fut: Future<Output = Output> + Send + 'static,
        Output: Send + 'static,
    {
        glommio_crate::Task::local(future).detach();
        Ok(())
    }
}

impl SpawnHandleStatic for Glommio {
    fn spawn_handle<Output, Fut>(future: Fut) -> Result<JoinHandle<Output>, SpawnError>
    where
        Fut: Future<Output = Output> + Send + 'static,
        Output: 'static + Send,
    {
        let (remote, handle) = future.remote_handle();
        glommio_crate::Task::local(remote).detach();
        Ok(JoinHandle::remote_handle(handle))
    }
}

impl SpawnBlockingStatic for Glommio {
    fn spawn_blocking<T: Send + 'static>(
        func: impl FnOnce() -> T + Send + 'static,
    ) -> Result<JoinHandle<T>, SpawnError> {
        let (remote, handle) = async { func() }.remote_handle();
        std::thread::spawn(move || {
            bind_to_cpu_set(to_cpu_set(None.into_iter())).unwrap();
            futures_executor::block_on(remote)
        });
        Ok(JoinHandle::remote_handle(handle))
    }
}
impl YieldNow for Glommio {}
macro_rules! to_io_error {
    ($error:expr) => {{
        match $error {
            Ok(x) => Ok(x),
            Err(nix::Error::Sys(_)) => Err(std::io::Error::last_os_error()),
            Err(nix::Error::InvalidUtf8) => {
                Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))
            }
            Err(nix::Error::InvalidPath) => {
                Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))
            }
            Err(nix::Error::UnsupportedOperation) => {
                Err(std::io::Error::from(std::io::ErrorKind::Other))
            }
        }
    }};
}
fn bind_to_cpu_set(cpuset: CpuSet) -> std::io::Result<()> {
    let pid = nix::unistd::Pid::this();
    to_io_error!(nix::sched::sched_setaffinity(pid, &cpuset))
}
fn to_cpu_set(cores: impl Iterator<Item = i32>) -> CpuSet {
    let mut set = CpuSet::new();
    let mut is_set = false;
    for i in cores {
        set.set(i as _).unwrap();
        is_set = true;
    }
    if !is_set {
        for i in 0..CpuSet::count() {
            set.set(i as _).unwrap();
        }
    }
    set
}
