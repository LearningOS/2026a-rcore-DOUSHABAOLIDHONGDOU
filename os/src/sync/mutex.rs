//! Mutex (spin-like and blocking(sleep))

use super::deadlock;
use super::UPSafeCell;
use crate::task::TaskControlBlock;
use crate::task::{block_current_and_run_next, suspend_current_and_run_next};
use crate::task::{current_task, wakeup_task};
use alloc::{collections::VecDeque, sync::Arc};

/// Mutex trait
pub trait Mutex: Sync + Send {
    /// Lock the mutex. Return false if blocking would deadlock.
    fn lock(&self) -> bool;
    /// Unlock the mutex
    fn unlock(&self);
}

/// Spinlock Mutex struct
pub struct MutexSpin {
    locked: UPSafeCell<bool>,
}

impl MutexSpin {
    /// Create a new spinlock mutex
    pub fn new() -> Self {
        Self {
            locked: unsafe { UPSafeCell::new(false) },
        }
    }
}

impl Mutex for MutexSpin {
    /// Lock the spinlock mutex
    fn lock(&self) -> bool {
        trace!("kernel: MutexSpin::lock");
        loop {
            let mut locked = self.locked.exclusive_access();
            if *locked {
                drop(locked);
                suspend_current_and_run_next();
                continue;
            } else {
                *locked = true;
                return true;
            }
        }
    }

    fn unlock(&self) {
        trace!("kernel: MutexSpin::unlock");
        let mut locked = self.locked.exclusive_access();
        *locked = false;
    }
}

/// Blocking Mutex struct
pub struct MutexBlocking {
    inner: UPSafeCell<MutexBlockingInner>,
}

pub struct MutexBlockingInner {
    locked: bool,
    wait_queue: VecDeque<Arc<TaskControlBlock>>,
}

impl MutexBlocking {
    /// Create a new blocking mutex
    pub fn new() -> Self {
        trace!("kernel: MutexBlocking::new");
        Self {
            inner: unsafe {
                UPSafeCell::new(MutexBlockingInner {
                    locked: false,
                    wait_queue: VecDeque::new(),
                })
            },
        }
    }
}

impl Mutex for MutexBlocking {
    /// lock the blocking mutex
    ///
    /// Return false (without blocking) if the wait would deadlock.
    fn lock(&self) -> bool {
        trace!("kernel: MutexBlocking::lock");
        let resource = self as *const Self as usize;
        let mut mutex_inner = self.inner.exclusive_access();
        if mutex_inner.locked {
            drop(mutex_inner);
            if !deadlock::before_block(resource) {
                return false;
            }
            let mut mutex_inner = self.inner.exclusive_access();
            mutex_inner.wait_queue.push_back(current_task().unwrap());
            drop(mutex_inner);
            block_current_and_run_next();
            deadlock::acquire(resource);
            true
        } else {
            mutex_inner.locked = true;
            drop(mutex_inner);
            deadlock::acquire(resource);
            true
        }
    }

    /// unlock the blocking mutex
    fn unlock(&self) {
        trace!("kernel: MutexBlocking::unlock");
        let resource = self as *const Self as usize;
        let mut mutex_inner = self.inner.exclusive_access();
        assert!(mutex_inner.locked);
        if let Some(waking_task) = mutex_inner.wait_queue.pop_front() {
            let woken = deadlock::task_key(&waking_task);
            deadlock::mutex_released(resource, woken);
            drop(mutex_inner);
            wakeup_task(waking_task);
        } else {
            mutex_inner.locked = false;
            drop(mutex_inner);
            deadlock::mutex_released(resource, None);
        }
    }
}
