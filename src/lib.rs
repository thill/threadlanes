//! Real-time executors with deterministic task routing and guaranteed ordering.
//!
//! # Example
//!
//! ```rust
//! use threadlanes::{ThreadLanes, LaneExecutor};
//!
//! struct MyExecutor {
//!     id: usize,
//! }
//! impl LaneExecutor<usize> for MyExecutor {
//!     fn execute(&mut self, task: usize) {
//!        println!("{} received {}", self.id, task);
//!     }
//! }
//!
//! fn main() {
//!     let lanes = ThreadLanes::new(vec![
//!         MyExecutor{id: 0},
//!         MyExecutor{id: 1},
//!         MyExecutor{id: 2},
//!     ]);
//!     
//!     lanes.send(0, 11); // send task=11 to thread lane 0
//!     lanes.send(1, 12); // send task=12 to thread lane 1
//!     lanes.send(1, 13); // send task=13 to thread lane 1
//!     lanes.send(2, 14); // send task=14 to thread lane 2
//!     lanes.send(2, 15); // send task=15 to thread lane 2
//!     lanes.send(2, 16); // send task=16 to thread lane 2
//!    
//!     lanes.flush();
//! }
//! ```
//!
//! # Why threadlanes
//!
//! ThreadLanes are useful when you need deterministic task ordering through stateful Executors.
//! This is in contrast to a thread pool, where the thread that executes a task is not deterministic.

extern crate countdown_latch;

mod tests;

use countdown_latch::CountDownLatch;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

/// Trait for user to define an Executor to be used by ThreadLanes
pub trait LaneExecutor<T> {
    fn execute(&mut self, task: T);
}

/// A ThreadLanes instance manages underlying executors, their threads, and their queues.
///
/// Submit tasks to be executed via the `send(&self, lane: usize, task: T)` method.
///
/// On destruction, underlying threads will be stopped and joined.
pub struct ThreadLanes<T: Send + 'static> {
    senders: Vec<LaneSender<T>>,
    keep_running: Arc<AtomicBool>,
    thread_handles: Vec<thread::JoinHandle<()>>,
    stopped_latch: Arc<CountDownLatch>,
}
impl<T: Send + 'static> ThreadLanes<T> {
    /// Create a new threadlanes instance using the given executors and lane capacity.
    /// If lane capacity is None, there will be no capacity limits. 
    pub fn new<E: LaneExecutor<T> + Send + 'static>(executors: Vec<E>, lane_capacity: Option<usize>) -> Self {
        let keep_running = Arc::new(AtomicBool::new(true));
        let stopped_latch = Arc::new(CountDownLatch::new(executors.len()));
        let mut thread_handles: Vec<thread::JoinHandle<()>> = Vec::new();
        let mut senders: Vec<LaneSender<T>> = Vec::new();
        for executor in executors {
            let (sender, receiver) = create_lane(lane_capacity, keep_running.clone(), Arc::clone(&stopped_latch));
            thread_handles.push(thread::spawn(move || receiver.receive_loop(executor)));
            senders.push(sender);
        }
        Self {
            senders: senders,
            keep_running,
            stopped_latch,
            thread_handles: thread_handles,
        }
    }

    /// Send a task for the given lane, which is a 0-based executor index.
    ///
    /// # Panics
    /// This will result in a panic if the underlying executor has crashed.
    pub fn send(&self, lane: usize, task: T) {
        match self.senders[lane].send_task(task) {
            Ok(_) => (),
            Err(_) => {
                if self.keep_running.load(Ordering::Acquire) {
                    panic!("LaneExecutor crashed on lane {}", lane);
                }
            }
        }
    }

    /// Block until all tasks that have already been submitted have been executed on all lanes.
    ///
    /// # Panics
    /// This will result in a panic if any underlying executor has crashed.
    pub fn flush(&self) {
        let latch = Arc::new(CountDownLatch::new(self.senders.len()));
        for lane in 0..self.senders.len() {
            match self.senders[lane].send_flush(Arc::clone(&latch)) {
                Ok(_) => (),
                Err(_) => {
                    panic!("LaneExecutor crashed on lane {}", lane);
                }
            }
        }
        latch.await();
    }

    /// Block until all tasks that have already been submitted have been executed for a specific lane.
    ///
    /// # Panics
    /// This will result in a panic if the underlying executor has crashed.
    pub fn flush_lane(&self, lane: usize) {
        let latch = Arc::new(CountDownLatch::new(1));
        match self.senders[lane].send_flush(Arc::clone(&latch)) {
            Ok(_) => (),
            Err(_) => {
                panic!("LaneExecutor crashed on lane {}", lane);
            }
        }
        latch.await();
    }

    /// Send the stop signal to underlying executor threads and return immediately.
    pub fn stop(&self) {
        self.keep_running.store(false, Ordering::Release);
        for sender in &self.senders {
            sender.try_send_stop();
        }
    }

    /// Join until all underlying executor threads have been stopped.
    pub fn join(&self) {
        self.stopped_latch.await();
    }
}
impl<T: Send + 'static> Drop for ThreadLanes<T> {
    fn drop(&mut self) {
        self.stop();
        for handle in self.thread_handles.drain(..) {
            handle.join().unwrap();
        }
    }
}

enum Event<T> {
    Task(T),
    Stop(),
    Flush(Arc<CountDownLatch>),
}

enum TaskSender<T: Send> {
    Async(mpsc::Sender<Event<T>>),
    Sync(mpsc::SyncSender<Event<T>>),
}

fn create_lane<T: Send>(
    lane_capacity: Option<usize>,
    keep_running: Arc<AtomicBool>,
    stopped_latch: Arc<CountDownLatch>,
) -> (LaneSender<T>, LaneReceiver<T>) {
    match lane_capacity {
        None => {
            let (tx, rx) = mpsc::channel();
            return (
                LaneSender {
                    sender: TaskSender::Async(tx),
                },
                LaneReceiver {
                    receiver: rx,
                    keep_running: keep_running.clone(),
                    stopped_latch,
                },
            );
        }
        Some(size) => {
            let (tx, rx) = mpsc::sync_channel(size);
            return (
                LaneSender {
                    sender: TaskSender::Sync(tx),
                },
                LaneReceiver {
                    receiver: rx,
                    keep_running: keep_running.clone(),
                    stopped_latch,
                },
            );
        }
    }
}

struct LaneSender<T: Send> {
    sender: TaskSender<T>,
}
impl<T: Send> LaneSender<T> {
    fn try_send_stop(&self) {
        match self.send(Event::Stop()) {
            Ok(_) => (),
            Err(_) => (),
        }
    }
    fn send_flush(&self, latch: Arc<CountDownLatch>) -> Result<(), mpsc::SendError<Event<T>>> {
        self.send(Event::Flush(latch))
    }
    fn send_task(&self, task: T) -> Result<(), mpsc::SendError<Event<T>>> {
        self.send(Event::Task(task))
    }
    fn send(&self, event: Event<T>) -> Result<(), mpsc::SendError<Event<T>>> {
        match &self.sender {
            TaskSender::Async(sender) => sender.send(event),
            TaskSender::Sync(sender) => sender.send(event),
        }
    }
}

struct LaneReceiver<T: Send> {
    receiver: mpsc::Receiver<Event<T>>,
    keep_running: Arc<AtomicBool>,
    stopped_latch: Arc<CountDownLatch>,
}
impl<T: Send> LaneReceiver<T> {
    fn receive_loop<E: LaneExecutor<T>>(&self, mut executor: E) {
        while self.keep_running.load(Ordering::Relaxed) {
            match self.receiver.recv().unwrap() {
                Event::Flush(latch) => latch.count_down(),
                Event::Stop() => break,
                Event::Task(task) => {
                    executor.execute(task);
                }
            }
        }
        self.stopped_latch.count_down();
    }
}
