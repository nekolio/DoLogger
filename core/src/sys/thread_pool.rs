//! Background thread pools.
//!
//! # Pool Types
//!
//! | Pool | Purpose | Default threads | Priority |
//! |------|---------|-----------------|----------|
//! | cpu_pool | Filter, assembly, processing, formatting | num_cpus | Normal |
//! | io_pool | Async IO completion, file/network writes | num_cpus/2 | Normal |
//! | sysmon_pool | Sysmon flush, internal diagnostics | 1 | Low |

use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// A simple fixed-size thread pool.
pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: Option<crossbeam_channel::Sender<Job>>,
}

type Job = Box<dyn FnOnce() + Send + 'static>;

struct Worker {
    _id: usize,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    fn new(id: usize, prefix: &str, receiver: Arc<crossbeam_channel::Receiver<Job>>) -> Self {
        let handle = thread::Builder::new()
            .name(format!("dologger-{prefix}-{id}"))
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    job();
                }
            })
            .ok();

        Self { _id: id, handle }
    }
}

impl ThreadPool {
    /// Create a new thread pool with `size` threads and `name` prefix.
    pub fn new(size: usize, name: &str) -> Self {
        assert!(size > 0, "Thread pool size must be positive");

        let (sender, receiver) = crossbeam_channel::unbounded();
        let receiver = Arc::new(receiver);

        let mut workers = Vec::with_capacity(size);
        for id in 0..size {
            workers.push(Worker::new(id, name, Arc::clone(&receiver)));
        }

        Self {
            workers,
            sender: Some(sender),
        }
    }

    /// Create with a size derived from CPU count.
    pub fn cpu_pool() -> Self {
        let size = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self::new(size, "cpu")
    }

    /// Create an IO pool (half the CPU count).
    pub fn io_pool() -> Self {
        let size = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).max(1))
            .unwrap_or(2);
        Self::new(size, "io")
    }

    /// Submit a job to the pool. Non-blocking.
    pub fn execute<F>(&self, job: F)
    where
        F: FnOnce() + Send + 'static,
    {
        if let Some(ref sender) = self.sender {
            let _ = sender.send(Box::new(job));
        }
    }

    /// Get the number of worker threads.
    pub fn size(&self) -> usize {
        self.workers.len()
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        // Drop sender FIRST to close the channel, unblocking workers.
        // Workers will exit their recv() loop when the channel is closed.
        drop(self.sender.take());
        // Now join all workers — they'll finish quickly since the channel is closed.
        for worker in &mut self.workers {
            if let Some(handle) = worker.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

/// Convenience: create cpu_pool, io_pool, and shm_pool.
pub struct PoolSet {
    /// CPU-bound tasks (filter, assembly, processing, formatting)
    pub cpu_pool: ThreadPool,
    /// IO-bound tasks (file writes, network sends)
    pub io_pool: ThreadPool,
    /// SHM-bound tasks (sink_shm writes) — single thread, lowest priority
    pub shm_pool: ThreadPool,
}

impl PoolSet {
    /// Create the default pool set.
    pub fn new() -> Self {
        Self {
            cpu_pool: ThreadPool::cpu_pool(),
            io_pool: ThreadPool::io_pool(),
            shm_pool: ThreadPool::new(1, "shm"),
        }
    }
}

impl Default for PoolSet {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_pool_executes_jobs() {
        let pool = ThreadPool::new(2, "test");
        let counter = Arc::new(AtomicUsize::new(0));

        for _ in 0..10 {
            let c = Arc::clone(&counter);
            pool.execute(move || {
                c.fetch_add(1, Ordering::Relaxed);
            });
        }

        // Drop pool and wait for jobs
        drop(pool);
        // Give threads time to complete
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(counter.load(Ordering::Relaxed), 10);
    }
}
