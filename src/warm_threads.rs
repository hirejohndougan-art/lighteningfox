//! Threads that are kept for the next page once theirs closes (X33).
//!
//! Compiled scripts are cached per thread (vendor/boa script_cache.rs: compiled code can't
//! move between threads), and every page evaluates Ironswift's own ~14,000 lines of
//! JavaScript first. A site process used to start a fresh thread for each page it opened, so
//! every navigation parsed all of it again: about 60% of the work of loading a small page.
//! Kept threads have that cache warm.
//!
//! A thread whose job panicked is never reused (its per-thread state may be half updated).
//! At most `MAX_IDLE` wait, each for `IDLE_FOR`: an idle thread holds its cache's memory.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Mutex;
use std::time::Duration;

type Job = Box<dyn FnOnce() + Send>;

const MAX_IDLE: usize = 2;
const IDLE_FOR: Duration = Duration::from_secs(120);

/// Idle threads: their kind (thread name: pages and workers have different stack sizes), an
/// id, and the way to hand each one its next job.
static IDLE: Mutex<Vec<(String, u64, Sender<Job>)>> = Mutex::new(Vec::new());
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Run `job` on an idle kept thread named `name`, or a new one with `stack` bytes of stack.
/// False if no thread could be started.
pub fn spawn(name: &str, stack: usize, job: impl FnOnce() + Send + 'static) -> bool {
    let mut job: Job = Box::new(job);
    loop {
        let idle = IDLE.lock().ok().and_then(|mut v| v.iter().rposition(|(n, _, _)| n == name).map(|i| v.remove(i)));
        let Some((_, _, tx)) = idle else { break };
        match tx.send(job) {
            Ok(()) => return true,
            // That thread has just gone: try another, or start one.
            Err(e) => job = e.0,
        }
    }
    let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let kind = name.to_string();
    std::thread::Builder::new().name(name.to_string()).stack_size(stack).spawn(move || run(kind, id, job)).is_ok()
}

fn run(kind: String, id: u64, first: Job) {
    let (tx, rx): (Sender<Job>, Receiver<Job>) = channel();
    let mut job = first;
    loop {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
            return;
        }
        {
            let Ok(mut idle) = IDLE.lock() else { return };
            if idle.iter().filter(|(n, _, _)| *n == kind).count() >= MAX_IDLE {
                return;
            }
            idle.push((kind.clone(), id, tx.clone()));
        }
        job = match rx.recv_timeout(IDLE_FOR) {
            Ok(j) => j,
            Err(_) => {
                // Leave the pool, then take a job that was handed over meanwhile, if any.
                if let Ok(mut idle) = IDLE.lock() {
                    idle.retain(|(_, i, _)| *i != id);
                }
                match rx.try_recv() {
                    Ok(j) => j,
                    Err(_) => return,
                }
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle_tests() -> usize {
        IDLE.lock().unwrap().iter().filter(|(n, _, _)| n == "warm-test").count()
    }

    fn on_pool() -> (std::thread::ThreadId, bool) {
        let (tx, rx) = channel();
        spawn("warm-test", 1 << 20, move || {
            let _ = tx.send(std::thread::current().id());
        });
        let id = rx.recv_timeout(Duration::from_secs(5)).expect("ran");
        // Wait until it's back in the pool.
        for _ in 0..200 {
            if idle_tests() > 0 {
                return (id, true);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        (id, false)
    }

    #[test]
    fn a_finished_thread_takes_the_next_job_and_a_panicked_one_doesnt() {
        // One test: the pool is process-wide.
        let (first, idle) = on_pool();
        assert!(idle, "back in the pool");
        let (second, _) = on_pool();
        assert_eq!(first, second, "the same thread, with its caches");
        // A panicking job: that thread retires.
        let before = idle_tests();
        spawn("warm-test", 1 << 20, || panic!("a page's bug (expected in this test)"));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(idle_tests(), before.saturating_sub(1), "not returned to the pool");
        let (third, _) = on_pool();
        assert_ne!(third, first, "a new thread");
    }
}
