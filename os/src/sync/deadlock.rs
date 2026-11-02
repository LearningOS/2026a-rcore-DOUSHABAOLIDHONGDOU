//! Wait-for graph based deadlock detection.
//!
//! We record, for every blocking mutex and semaphore, which tasks hold it and
//! which tasks are waiting for it.  Before a task blocks, we check whether the
//! system can still make progress: a blocked task can proceed if one of the
//! current holders of its resource is runnable (that holder will eventually
//! release the resource).  If the requesting task can never become runnable in
//! this simulation, blocking would deadlock and the syscall reports `-0xdead`
//! instead of blocking.

use crate::sync::UPSafeCell;
use crate::task::{current_task, TaskControlBlock};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    vec::Vec,
};
use lazy_static::*;

/// Error code returned when a deadlock is detected
pub const DEADLOCK: isize = -0xdead;

struct ResourceState {
    /// tasks currently holding a lock / a unit of the semaphore
    holders: Vec<usize>,
    /// tasks blocked on this resource
    waiters: Vec<usize>,
}

struct ProcessState {
    /// resource address -> state
    resources: BTreeMap<usize, ResourceState>,
    /// task id -> resource address it is blocked on
    blocked: BTreeMap<usize, usize>,
}

struct DeadlockDetector {
    enabled: bool,
    processes: BTreeMap<usize, ProcessState>,
}

lazy_static! {
    static ref DETECTOR: UPSafeCell<DeadlockDetector> = unsafe {
        UPSafeCell::new(DeadlockDetector {
            enabled: false,
            processes: BTreeMap::new(),
        })
    };
}

/// Enable or disable deadlock detection (clears the graph when disabling)
pub fn enable(enabled: bool) {
    let mut detector = DETECTOR.exclusive_access();
    detector.enabled = enabled;
    if !enabled {
        detector.processes.clear();
    }
}

/// Get the `(pid, tid)` key of a task
pub(crate) fn task_key(task: &Arc<TaskControlBlock>) -> Option<(usize, usize)> {
    let pid = task.process.upgrade()?.pid.0;
    let tid = task.inner_exclusive_access().res.as_ref()?.tid;
    Some((pid, tid))
}

fn current_key() -> Option<(usize, usize)> {
    task_key(&current_task()?)
}

impl ProcessState {
    fn resource_mut(&mut self, resource: usize) -> &mut ResourceState {
        self.resources.entry(resource).or_insert_with(|| ResourceState {
            holders: Vec::new(),
            waiters: Vec::new(),
        })
    }

    fn remove_waiter(&mut self, tid: usize) {
        if let Some(resource) = self.blocked.remove(&tid) {
            if let Some(state) = self.resources.get_mut(&resource) {
                state.waiters.retain(|t| *t != tid);
            }
        }
    }

    /// Simulate the progress of blocked tasks.  Return true if `tid` would
    /// eventually acquire its resource.
    fn can_progress(&self, tid: usize) -> bool {
        let mut known = BTreeSet::new();
        for state in self.resources.values() {
            known.extend(state.holders.iter().copied());
        }
        known.extend(self.blocked.keys().copied());
        let mut runnable: BTreeSet<usize> = known
            .into_iter()
            .filter(|task| !self.blocked.contains_key(task))
            .collect();
        loop {
            let mut changed = false;
            for (task, resource) in self.blocked.iter() {
                if runnable.contains(task) {
                    continue;
                }
                let can_acquire = match self.resources.get(resource) {
                    None => true,
                    Some(state) => {
                        state.holders.is_empty()
                            || state.holders.iter().any(|holder| runnable.contains(holder))
                    }
                };
                if can_acquire {
                    runnable.insert(*task);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        runnable.contains(&tid)
    }
}

/// Record that the current task is about to block on `resource`.
///
/// Return `false` if doing so would deadlock (the registration is rolled back).
pub fn before_block(resource: usize) -> bool {
    let mut detector = DETECTOR.exclusive_access();
    if !detector.enabled {
        return true;
    }
    let (pid, tid) = match current_key() {
        Some(key) => key,
        None => return true,
    };
    let process = detector
        .processes
        .entry(pid)
        .or_insert_with(|| ProcessState {
            resources: BTreeMap::new(),
            blocked: BTreeMap::new(),
        });
    process.resource_mut(resource).waiters.push(tid);
    process.blocked.insert(tid, resource);
    let progress = process.can_progress(tid);
    if progress {
        true
    } else {
        process.remove_waiter(tid);
        false
    }
}

/// Record that the current task acquired `resource` (after blocking or not)
pub fn acquire(resource: usize) {
    let mut detector = DETECTOR.exclusive_access();
    if !detector.enabled {
        return;
    }
    let (pid, tid) = match current_key() {
        Some(key) => key,
        None => return,
    };
    let process = detector
        .processes
        .entry(pid)
        .or_insert_with(|| ProcessState {
            resources: BTreeMap::new(),
            blocked: BTreeMap::new(),
        });
    process.remove_waiter(tid);
    let state = process.resource_mut(resource);
    if !state.holders.contains(&tid) {
        state.holders.push(tid);
    }
}

/// Record a release of a mutex.  If `woken` is given, it becomes the new owner.
pub fn mutex_released(resource: usize, woken: Option<(usize, usize)>) {
    let mut detector = DETECTOR.exclusive_access();
    if !detector.enabled {
        return;
    }
    let pid = match woken.as_ref().map(|key| key.0).or_else(|| current_key().map(|key| key.0)) {
        Some(pid) => pid,
        None => return,
    };
    let process = match detector.processes.get_mut(&pid) {
        Some(process) => process,
        None => return,
    };
    if let Some((_, tid)) = woken {
        process.remove_waiter(tid);
        let state = process.resource_mut(resource);
        state.holders.clear();
        state.holders.push(tid);
    } else {
        process.resource_mut(resource).holders.clear();
    }
}

/// Record a release of one semaphore unit.  If `woken` is given, it becomes
/// the holder of the released unit.
pub fn semaphore_released(resource: usize, woken: Option<(usize, usize)>) {
    let mut detector = DETECTOR.exclusive_access();
    if !detector.enabled {
        return;
    }
    let current = current_key();
    let pid = match woken
        .as_ref()
        .map(|key| key.0)
        .or_else(|| current.map(|key| key.0))
    {
        Some(pid) => pid,
        None => return,
    };
    let process = match detector.processes.get_mut(&pid) {
        Some(process) => process,
        None => return,
    };
    if let Some((_, tid)) = current {
        let state = process.resource_mut(resource);
        if let Some(pos) = state.holders.iter().position(|holder| *holder == tid) {
            state.holders.remove(pos);
        }
    }
    if let Some((_, tid)) = woken {
        process.remove_waiter(tid);
        let state = process.resource_mut(resource);
        if !state.holders.contains(&tid) {
            state.holders.push(tid);
        }
    }
}

/// Remove a task from the detection graph (it is exiting)
pub fn task_exit(pid: usize, tid: usize) {
    let mut detector = DETECTOR.exclusive_access();
    if !detector.enabled {
        return;
    }
    if let Some(process) = detector.processes.get_mut(&pid) {
        process.blocked.remove(&tid);
        for state in process.resources.values_mut() {
            state.waiters.retain(|task| *task != tid);
            state.holders.retain(|task| *task != tid);
        }
    }
}

/// Remove every record belonging to a process (it is exiting)
pub fn process_exit(pid: usize) {
    let mut detector = DETECTOR.exclusive_access();
    detector.processes.remove(&pid);
}
