//! Semaphore

use crate::sync::deadlock;
use crate::sync::UPSafeCell;
use crate::task::{block_current_and_run_next, current_task, wakeup_task, TaskControlBlock};
use alloc::{collections::VecDeque, sync::Arc};

/// semaphore structure
pub struct Semaphore {
    /// semaphore inner
    pub inner: UPSafeCell<SemaphoreInner>,
}

pub struct SemaphoreInner {
    pub count: isize,
    pub wait_queue: VecDeque<Arc<TaskControlBlock>>,
}

impl Semaphore {
    /// Create a new semaphore
    pub fn new(res_count: usize) -> Self {
        trace!("kernel: Semaphore::new");
        Self {
            inner: unsafe {
                UPSafeCell::new(SemaphoreInner {
                    count: res_count as isize,
                    wait_queue: VecDeque::new(),
                })
            },
        }
    }

    /// up operation of semaphore
    pub fn up(&self) {
        trace!("kernel: Semaphore::up");
        let resource = self as *const Self as usize;
        let mut inner = self.inner.exclusive_access();
        inner.count += 1;
        if inner.count <= 0 {
            if let Some(task) = inner.wait_queue.pop_front() {
                let woken = deadlock::task_key(&task);
                deadlock::semaphore_released(resource, woken);
                drop(inner);
                wakeup_task(task);
                return;
            }
        }
        deadlock::semaphore_released(resource, None);
    }

    /// down operation of semaphore
    ///
    /// Return false (without blocking) if the wait would deadlock.
    pub fn down(&self) -> bool {
        trace!("kernel: Semaphore::down");
        let resource = self as *const Self as usize;
        let mut inner = self.inner.exclusive_access();
        inner.count -= 1;
        if inner.count < 0 {
            drop(inner);
            if !deadlock::before_block(resource) {
                self.inner.exclusive_access().count += 1;
                return false;
            }
            let mut inner = self.inner.exclusive_access();
            inner.wait_queue.push_back(current_task().unwrap());
            drop(inner);
            block_current_and_run_next();
            deadlock::acquire(resource);
            true
        } else {
            drop(inner);
            deadlock::acquire(resource);
            true
        }
    }
}
