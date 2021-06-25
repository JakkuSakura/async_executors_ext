use crate::{bind_to_cpu_set, to_cpu_set, CommonRt, NonblockingFuture, NonblockingFutureExt};
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::ops::Range;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::Poll;

#[derive(Debug)]
pub struct NonblockingTpBuilder {
    threads: i32,
    name: String,
    pin_to_cpu: Option<Range<i32>>,
}

impl NonblockingTpBuilder {
    /// Create a new builder
    pub fn new(threads: i32) -> Self {
        Self {
            threads,
            name: "NonblockingTp".to_string(),
            pin_to_cpu: None,
        }
    }
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// block on the given future
    pub fn build(&self) -> Result<NonblockingTp, std::io::Error> {
        let range = self.pin_to_cpu.clone().unwrap_or(0..self.threads);
        let mut controllers = vec![];
        for (id, cpu_id) in range.enumerate() {
            let (tx, rx) = crossbeam::channel::unbounded();
            let running = Arc::new(AtomicBool::new(true));
            controllers.push(ManagedExecutorController {
                running: Arc::clone(&running),
                tx,
            });
            let mut executor: ManagedExecutor<()> = ManagedExecutor {
                cpu: Some(cpu_id),
                tasks: vec![],
                rx,
                running,
                cnt: 0,
            };
            CommonRt::spawn_blocking_with_name(format!("{}-{}", self.name, id), move || {
                bind_to_cpu_set(to_cpu_set(executor.cpu)).unwrap();
                loop {
                    match executor.poll_nb_unpin() {
                        Poll::Ready(_) => break,
                        Poll::Pending => {}
                    }
                }
            })
            .unwrap()
            .detach();
        }

        Ok(NonblockingTp {
            controllers,
            last: Default::default(),
        })
    }
}

/// A custom task
pub struct NonblockingTask<T> {
    future: Pin<Box<dyn NonblockingFuture<Output = T> + Send>>,
}
impl<T> NonblockingTask<T> {
    pub fn new(x: impl NonblockingFuture<Output = T> + Send + 'static) -> Self {
        Self {
            future: Box::pin(x),
        }
    }
    pub fn new_boxed(x: Pin<Box<dyn NonblockingFuture<Output = T> + Send>>) -> Self {
        Self { future: x }
    }
}
impl<T> NonblockingFuture for NonblockingTask<T> {
    type Output = T;

    fn poll_nb(mut self: Pin<&mut Self>) -> Poll<Self::Output> {
        self.future.poll_nb_unpin()
    }
}

struct ManagedExecutor<T> {
    cpu: Option<i32>,
    tasks: Vec<NonblockingTask<T>>,
    rx: crossbeam::channel::Receiver<NonblockingTask<T>>,
    running: Arc<AtomicBool>,
    cnt: usize,
}

impl<T> ManagedExecutor<T> {
    fn find_task(&self) -> Option<NonblockingTask<T>> {
        match self.rx.try_recv() {
            Ok(obj) => {
                return Some(obj);
            }
            Err(crossbeam::channel::TryRecvError::Empty) => {
                return None;
            }
            Err(crossbeam::channel::TryRecvError::Disconnected) => {
                return None;
            }
        }
    }
}
impl<T> Drop for ManagedExecutor<T> {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release)
    }
}

impl<T> NonblockingFuture for ManagedExecutor<T> {
    type Output = ();

    fn poll_nb(mut self: Pin<&mut Self>) -> Poll<Self::Output> {
        if !self.running.load(Ordering::Relaxed) {
            return Poll::Ready(());
        }
        self.cnt = self.cnt.wrapping_add(1);
        if self.cnt == 10 {
            self.cnt = 0;
            self.find_task().map(|x| self.tasks.push(x));
        }
        let mut i = 0;
        while i < self.tasks.len() {
            match self.tasks[i].poll_nb_unpin() {
                Poll::Ready(_) => {
                    self.tasks.swap_remove(i);
                }
                Poll::Pending => {
                    i += 1;
                }
            }
        }
        Poll::Pending
    }
}
struct ManagedExecutorController {
    running: Arc<AtomicBool>,
    tx: crossbeam::channel::Sender<NonblockingTask<()>>,
}
impl ManagedExecutorController {
    pub fn shutdown(&self) {
        self.running.store(false, Ordering::Release)
    }
    pub fn send(&self, task: NonblockingTask<()>) -> Result<(), NonblockingError<()>> {
        if self.running.load(Ordering::Acquire) {
            Ok(self.tx.try_send(task)?)
        } else {
            Err(NonblockingError::ExecutorNotRunning(task))
        }
    }
}

pub enum NonblockingError<T> {
    /// executor dropped
    ExecutorDropped(NonblockingTask<T>),
    ExecutorNotRunning(NonblockingTask<T>),
}
impl<T> NonblockingError<T> {
    pub fn into_inner(self) -> NonblockingTask<T> {
        match self {
            NonblockingError::ExecutorDropped(x) => x,
            NonblockingError::ExecutorNotRunning(x) => x,
        }
    }
}
impl<T> Debug for NonblockingError<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                NonblockingError::ExecutorDropped(_) => {
                    "ExecutorDropped"
                }
                NonblockingError::ExecutorNotRunning(_) => {
                    "ExecutorNotRunning"
                }
            }
        )
    }
}
impl<T> Display for NonblockingError<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                NonblockingError::ExecutorDropped(_) => {
                    "ExecutorDropped"
                }
                NonblockingError::ExecutorNotRunning(_) => {
                    "ExecutorNotRunning"
                }
            }
        )
    }
}
impl<T> Error for NonblockingError<T> {}

impl<T> From<crossbeam::channel::TrySendError<NonblockingTask<T>>> for NonblockingError<T> {
    fn from(x: crossbeam::channel::TrySendError<NonblockingTask<T>>) -> Self {
        Self::ExecutorDropped(x.into_inner())
    }
}

/// A ThreadPooled Glommio Runtime with work stealing algorithm
pub struct NonblockingTp {
    controllers: Vec<ManagedExecutorController>,
    last: AtomicUsize,
}

impl NonblockingTp {
    pub fn spawn_custom_task_obj(
        &self,
        task: NonblockingTask<()>,
    ) -> Result<(), NonblockingError<()>> {
        let mut task = Some(task);
        let mut cnt = 0;
        while let Some(t) = task.take() {
            let id = self.last.fetch_add(1, Ordering::AcqRel) % self.controllers.len();
            match self.controllers.get(id).unwrap().send(t) {
                Ok(_) => {}
                Err(e) => {
                    cnt += 1;
                    if cnt >= self.controllers.len() {
                        return Err(e);
                    }
                    task = Some(e.into_inner());
                }
            }
        }
        Ok(())
    }
    pub fn shutdown_all(&self) {
        self.controllers.iter().for_each(|x| x.shutdown());
    }
}
