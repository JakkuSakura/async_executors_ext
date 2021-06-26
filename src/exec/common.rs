use crate::{JoinHandle, SpawnBlockingStatic, YieldNow};
use futures_task::SpawnError;
use futures_util::FutureExt;
#[cfg(target_os = "linux")]
use nix::sched::CpuSet;
use std::io::ErrorKind;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, Default)]
pub struct CommonRt;
impl CommonRt {
    pub fn spawn_blocking_with_name<T: Send + 'static>(
        name: impl Into<String>,
        func: impl FnOnce() -> T + Send + 'static,
    ) -> Result<JoinHandle<T>, SpawnError> {
        let (remote, handle) = async { func() }.remote_handle();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                try_unbind_from_cpu().unwrap();
                minimal_executor::block_on(remote)
            })
            .unwrap();
        Ok(JoinHandle::remote_handle(handle))
    }
}
impl SpawnBlockingStatic for CommonRt {
    fn spawn_blocking<T: Send + 'static>(
        func: impl FnOnce() -> T + Send + 'static,
    ) -> Result<JoinHandle<T>, SpawnError> {
        let (remote, handle) = async { func() }.remote_handle();
        std::thread::spawn(move || {
            try_unbind_from_cpu().unwrap();
            minimal_executor::block_on(remote)
        });
        Ok(JoinHandle::remote_handle(handle))
    }
}
impl YieldNow for CommonRt {}
#[allow(unused_macros)]
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

static ALLOCATED: [AtomicBool; 256] = arr_macro::arr![AtomicBool::new(false); 256];
pub fn try_bind_to_cpu(core_id: i32) -> std::io::Result<()> {
    if ALLOCATED[core_id as usize]
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
    {
        bind_to_cpu_set(to_cpu_set(Some(core_id)))
    } else {
        Err(std::io::Error::new(
            ErrorKind::Other,
            format!("Cannot bind to core {}", core_id),
        ))
    }
}
pub fn try_bind_available_cpu() -> std::io::Result<()> {
    for i in 0..256 {
        if try_bind_to_cpu(i).is_ok() {
            return Ok(());
        }
    }
    Err(std::io::Error::new(
        ErrorKind::Other,
        format!("Cannot bind to core 0..256"),
    ))
}
pub fn try_unbind_from_cpu() -> std::io::Result<()> {
    bind_to_cpu_set(to_cpu_set(None))
}
#[cfg(any(target_os = "android", target_os = "linux"))]
pub(crate) fn bind_to_cpu_set(cpuset: CpuSet) -> std::io::Result<()> {
    let pid = nix::unistd::Pid::this();
    to_io_error!(nix::sched::sched_setaffinity(pid, &cpuset))
}

#[cfg(any(target_os = "android", target_os = "linux"))]
pub(crate) fn to_cpu_set(cores: impl IntoIterator<Item = i32>) -> CpuSet {
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
#[cfg(not(any(target_os = "android", target_os = "linux")))]
pub(crate) fn to_cpu_set(_: impl IntoIterator<Item = i32>) {}
#[cfg(not(any(target_os = "android", target_os = "linux")))]
pub(crate) fn bind_to_cpu_set<T>(_: T) -> std::io::Result<()> {
    Ok(())
}
