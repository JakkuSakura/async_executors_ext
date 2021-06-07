use crate::YieldNow;
use async_executors::{JoinHandle, LocalSpawnHandle, SpawnHandle};
use crossbeam::deque::Injector;
use crossbeam::deque::Stealer;
use crossbeam::deque::Worker;
use futures_executor::block_on;
use futures_task::{FutureObj, LocalFutureObj, LocalSpawn, Spawn, SpawnError};
use futures_util::future::RemoteHandle;
use futures_util::FutureExt;
use glommio_crate::LocalExecutorBuilder;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::ops::Range;
use std::sync::{Arc, Mutex};

/// A simple glommio runtime builder
#[derive(Debug)]
pub struct GlommioTpBuilder {
    threads: usize,
    name: String,
    pin_to_cpu: Option<Range<usize>>,
}

impl GlommioTpBuilder {
    /// Create a new builder
    pub fn new(threads: usize) -> Self {
        Self {
            threads,
            name: "unnamed".to_string(),
            pin_to_cpu: None,
        }
    }

    /// block on the given future
    pub fn build(&self) -> Result<GlommioTp, std::io::Error> {
        let range = self.pin_to_cpu.clone().unwrap_or(0..self.threads);
        let mut thread_pool = vec![];
        let mut workers = vec![];
        let global_injector = Arc::new(Injector::new());
        let mut dedicated_tx = vec![];
        for (thread, cpu_id) in (0..self.threads).zip(range) {
            let mut builder =
                LocalExecutorBuilder::new().name(&format!("{}-{}", self.name, thread));
            if self.pin_to_cpu.is_some() {
                builder = builder.pin_to_cpu(cpu_id);
            }
            thread_pool.push(builder);
            let (tx, rx) = crossbeam::channel::unbounded();
            dedicated_tx.push(tx);
            workers.push(ManagedExecutor {
                dedicated: rx,
                local: Worker::new_fifo(),
                global: Arc::clone(&global_injector),
                stealers: vec![],
            });
        }
        for e1 in 0..workers.len() {
            for e2 in 0..workers.len() {
                if e1 != e2 {
                    let stealer = workers[e2].local.stealer();
                    workers[e1].stealers.push(stealer);
                }
            }
        }
        let mut join_handles = vec![];
        for (id, (t, e)) in thread_pool.into_iter().zip(workers.into_iter()).enumerate() {
            let name = format!("{}-{}", self.name, id);

            let join_handle = std::thread::Builder::new()
                .name(name)
                .spawn(move || {
                    t.make().unwrap().run(async move { e.run().await });
                })
                .unwrap();
            join_handles.push(join_handle);
        }
        Ok(GlommioTp::new(join_handles, global_injector, dedicated_tx))
    }
}

/// A custom task
#[derive(Debug)]
pub struct CustomTask<T> {
    future: FutureObj<'static, T>,
    executor_id: Option<usize>,
}

impl<Fut, T> From<Fut> for CustomTask<T>
where
    Fut: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    fn from(f: Fut) -> Self {
        Self {
            future: FutureObj::new(f.boxed()),
            executor_id: None,
        }
    }
}

#[derive(Debug)]
struct ManagedExecutor {
    dedicated: crossbeam::channel::Receiver<CustomTask<()>>,
    local: Worker<CustomTask<()>>,
    global: Arc<Injector<CustomTask<()>>>,
    stealers: Vec<Stealer<CustomTask<()>>>,
}

impl ManagedExecutor {
    fn find_task(&self) -> Option<CustomTask<()>> {
        loop {
            match self.dedicated.try_recv() {
                Ok(obj) => {
                    return Some(obj);
                }
                Err(crossbeam::channel::TryRecvError::Empty) => {
                    // Pop a task from the local queue, if not empty.
                    let result = self.local.pop().or_else(|| {
                        // Otherwise, we need to look for a task elsewhere.
                        std::iter::repeat_with(|| {
                            // Try stealing a batch of tasks from the global queue.
                            self.global
                                .steal_batch_and_pop(&self.local)
                                // Or try stealing a task from one of the other threads.
                                .or_else(|| self.stealers.iter().map(|s| s.steal()).collect())
                        })
                        .find(|x| !x.is_retry())
                        .and_then(|x| x.success())
                    });
                    if result.is_some() {
                        return result;
                    }
                }
                Err(crossbeam::channel::TryRecvError::Disconnected) => {
                    return None;
                }
            }
        }
    }
    async fn run(&self) {
        while let Some(task) = self.find_task() {
            task.future.await;
        }
    }
}

/// A glommio error
#[derive(Debug, Copy, Clone)]
pub enum GlommioError {
    /// no such executor
    NoSuchExecutor(usize),
    /// executor dropped
    ExecutorDropped(Option<usize>),
}

impl Display for GlommioError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            GlommioError::NoSuchExecutor(id) => {
                f.write_fmt(format_args!("No such executor: id={}", id))
            }
            GlommioError::ExecutorDropped(id) => {
                f.write_fmt(format_args!("Executor dropped: id={:?}", id))
            }
        }
    }
}

impl Error for GlommioError {}

impl<T> From<crossbeam::channel::SendError<T>> for GlommioError {
    fn from(_: crossbeam::channel::SendError<T>) -> Self {
        Self::ExecutorDropped(None)
    }
}

/// A ThreadPooled Glommio Runtime with work stealing algorithm
#[derive(Debug)]
pub struct GlommioTp {
    join_handles: Mutex<Vec<std::thread::JoinHandle<()>>>,
    global: Arc<Injector<CustomTask<()>>>,
    dedicated: Vec<crossbeam::channel::Sender<CustomTask<()>>>,
}

impl GlommioTp {
    fn new(
        join_handles: Vec<std::thread::JoinHandle<()>>,
        global: Arc<Injector<CustomTask<()>>>,
        dedicated: Vec<crossbeam::channel::Sender<CustomTask<()>>>,
    ) -> Self {
        Self {
            join_handles: Mutex::new(join_handles),
            global,
            dedicated,
        }
    }

    /// spawn a custom task
    pub fn spawn_custom_task<T: Send + 'static>(
        &self,
        task: impl Into<CustomTask<T>>,
    ) -> Result<RemoteHandle<T>, GlommioError> {
        let task = task.into();
        let (remote, handle) = task.future.remote_handle();

        let task = CustomTask {
            future: FutureObj::new(remote.boxed()),
            executor_id: task.executor_id,
        };
        if let Some(id) = task.executor_id {
            match self.dedicated.get(id) {
                Some(sender) => {
                    sender.send(task)?;
                    return Ok(handle);
                }
                None => {
                    return Err(GlommioError::NoSuchExecutor(id));
                }
            }
        } else {
            self.global.push(task);
            Ok(handle)
        }
    }
    /// spawn a custom task
    pub fn spawn_custom_task_obj(&self, task: CustomTask<()>) -> Result<(), GlommioError> {
        if let Some(id) = task.executor_id {
            match self.dedicated.get(id) {
                Some(sender) => {
                    sender.send(task)?;
                    return Ok(());
                }
                None => {
                    return Err(GlommioError::NoSuchExecutor(id));
                }
            }
        } else {
            self.global.push(task);
            Ok(())
        }
    }
    /// spawn a task and block on it
    pub fn block_on<Fut>(&self, future: Fut) -> Fut::Output
    where
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        let (remote, handle) = future.remote_handle();
        self.global.push(CustomTask {
            future: FutureObj::new(remote.boxed()),
            executor_id: None,
        });
        block_on(handle)
    }
}

impl Spawn for GlommioTp {
    fn spawn_obj(&self, future: FutureObj<'static, ()>) -> Result<(), SpawnError> {
        self.global.push(CustomTask {
            future,
            executor_id: None,
        });
        Ok(())
    }
}

impl<Out: Send + 'static> SpawnHandle<Out> for GlommioTp {
    fn spawn_handle_obj(
        &self,
        future: FutureObj<'static, Out>,
    ) -> Result<JoinHandle<Out>, SpawnError> {
        let (remote, handle) = future.remote_handle();
        self.global.push(CustomTask {
            future: FutureObj::new(remote.boxed()),
            executor_id: None,
        });
        Ok(JoinHandle::remote_handle(handle))
    }
}

impl LocalSpawn for GlommioTp {
    fn spawn_local_obj(&self, future: LocalFutureObj<'static, ()>) -> Result<(), SpawnError> {
        glommio_crate::Task::local(future).detach();
        Ok(())
    }
}

impl<Out: Send + 'static> LocalSpawnHandle<Out> for GlommioTp {
    fn spawn_handle_local_obj(
        &self,
        future: LocalFutureObj<'static, Out>,
    ) -> Result<JoinHandle<Out>, SpawnError> {
        let (remote, handle) = future.remote_handle();
        glommio_crate::Task::local(remote).detach();
        Ok(JoinHandle::remote_handle(handle))
    }
}
impl YieldNow for GlommioTp {}
