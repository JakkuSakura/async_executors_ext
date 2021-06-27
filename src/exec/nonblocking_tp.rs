use crate::{
    try_bind_available_cpu, try_unbind_from_cpu, CommonRt, NonblockingFuture, NonblockingFutureExt,
};
use dashmap::DashMap;
use futures_task::ArcWake;
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::task::{Poll, Waker};

#[derive(Debug)]
pub struct NonblockingTpBuilder {
    threads: i32,
    name: String,
}

impl NonblockingTpBuilder {
    /// Create a new builder
    pub fn new(threads: i32) -> Self {
        Self {
            threads,
            name: "NonblockingTp".to_string(),
        }
    }
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// block on the given future
    pub fn build(&self) -> Result<NonblockingTp, std::io::Error> {
        let mut controllers = vec![];
        for id in 0..self.threads {
            let (tx, rx) = crossbeam::channel::unbounded();
            let running = Arc::new(AtomicBool::new(true));
            controllers.push(ManagedExecutorController {
                running: Arc::clone(&running),
                tx,
            });
            let mut executor: ManagedExecutor<()> = ManagedExecutor {
                bind_cpu: true,
                tasks: vec![],
                rx,
                running,
                woken: Arc::new(DashMap::new()),
                cnt: 0,
            };
            CommonRt::spawn_blocking_with_name(format!("{}-{}", self.name, id), move || {
                if executor.bind_cpu {
                    try_bind_available_cpu().unwrap();
                } else {
                    let _ = try_unbind_from_cpu();
                }
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
pub struct TaskSetWaker {
    task_id: AtomicI32,
    queue_set: RwLock<Option<Arc<DashMap<i32, ()>>>>,
}
impl TaskSetWaker {
    pub fn new() -> Self {
        Self {
            task_id: Default::default(),
            queue_set: RwLock::new(None),
        }
    }
    pub fn wake(&self) {
        if let Some(set) = self.queue_set.read().unwrap().as_ref() {
            set.insert(self.task_id.load(Ordering::Relaxed), ());
        }
    }
}
impl ArcWake for TaskSetWaker {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        (**arc_self).wake()
    }
}
/// A custom task
pub struct NonblockingTask<T> {
    future: Pin<Box<dyn NonblockingFuture<Output = T> + Send>>,
    waker: Option<Arc<TaskSetWaker>>,
}
impl<T> NonblockingTask<T> {
    pub fn new(x: impl NonblockingFuture<Output = T> + Send + 'static) -> Self {
        Self {
            future: Box::pin(x),
            waker: None,
        }
    }
    pub fn new_boxed(x: Pin<Box<dyn NonblockingFuture<Output = T> + Send>>) -> Self {
        Self {
            future: x,
            waker: None,
        }
    }
    pub fn get_waker(&mut self) -> Waker {
        match &self.waker {
            None => {
                let waker = Arc::new(TaskSetWaker::new());
                let waker2 = futures_task::waker_ref(&waker).clone();
                self.waker = Some(waker);
                waker2
            }
            Some(waker) => futures_task::waker_ref(waker).clone(),
        }
    }
}
impl<T> NonblockingFuture for NonblockingTask<T> {
    type Output = T;

    fn poll_nb(mut self: Pin<&mut Self>) -> Poll<Self::Output> {
        self.future.poll_nb_unpin()
    }
}

struct ManagedExecutor<T> {
    bind_cpu: bool,
    tasks: Vec<Option<NonblockingTask<T>>>,
    rx: crossbeam::channel::Receiver<NonblockingTask<T>>,
    running: Arc<AtomicBool>,
    woken: Arc<DashMap<i32, ()>>,
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
    fn find_hole(&self) -> Option<usize> {
        self.tasks
            .iter()
            .enumerate()
            .find(|x| x.1.is_none())
            .map(|x| x.0)
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
            if let Some(task) = self.find_task() {
                let task_id = if let Some(hole) = self.find_hole() {
                    hole
                } else {
                    let len = self.tasks.len();
                    self.tasks.push(None);
                    len
                };
                if let Some(waker) = task.waker.as_ref() {
                    waker.task_id.store(task_id as _, Ordering::Release);
                    *waker.queue_set.write().unwrap() = Some(Arc::clone(&self.woken));
                }
                self.tasks[task_id] = Some(task)
            }
        }
        let mut tasks = Vec::new();
        for task_ids in self.woken.iter() {
            tasks.push(*task_ids.key());
        }
        for task_id in tasks {
            self.woken.remove(&task_id);
            let task = &mut self.tasks[task_id as usize];
            if let Some(task2) = task.as_mut() {
                match task2.poll_nb_unpin() {
                    Poll::Ready(_) => {
                        *task = None;
                    }
                    Poll::Pending => {}
                }
            }
        }
        for task in self.tasks.iter_mut() {
            if let Some(task2) = task.as_mut() {
                match task2.poll_nb_unpin() {
                    Poll::Ready(_) => {
                        *task = None;
                    }
                    Poll::Pending => {}
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
