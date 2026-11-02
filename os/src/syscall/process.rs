//! Process management syscalls
use crate::config::PAGE_SIZE;
use crate::mm::{translated_byte_buffer, MapPermission, PageTable, VirtAddr};
use crate::task::{
    change_program_brk, current_user_token, exit_current_and_run_next, get_syscall_count,
    mmap_current, munmap_current, suspend_current_and_run_next,
};
use crate::timer::get_time_us;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// Translate one user address into a kernel-writable pointer.
///
/// Return `None` if the address is not mapped or lacks the requested permission.
fn translate_user_byte(token: usize, va: usize, write: bool) -> Option<*mut u8> {
    let page_table = PageTable::from_token(token);
    let pte = page_table.translate(VirtAddr::from(va).floor())?;
    if !pte.is_valid() || (write && !pte.writable()) || (!write && !pte.readable()) {
        return None;
    }
    let pa = pte.ppn().0 * PAGE_SIZE + va & (PAGE_SIZE - 1);
    Some(pa as *mut u8)
}

/// get time with second and microsecond
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let time_val = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    // The user buffer may straddle two pages, so copy it page by page.
    let src = unsafe {
        core::slice::from_raw_parts(
            &time_val as *const TimeVal as *const u8,
            core::mem::size_of::<TimeVal>(),
        )
    };
    let token = current_user_token();
    let mut copied = 0;
    for page in translated_byte_buffer(token, ts as *const u8, src.len()) {
        let len = page.len();
        page.copy_from_slice(&src[copied..copied + len]);
        copied += len;
    }
    0
}

/// trace user memory or syscall counters
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    let token = current_user_token();
    match trace_request {
        0 => match translate_user_byte(token, id, false) {
            Some(ptr) => unsafe { ptr.read_volatile() as isize },
            None => -1,
        },
        1 => match translate_user_byte(token, id, true) {
            Some(ptr) => {
                unsafe { ptr.write_volatile(data as u8) };
                0
            }
            None => -1,
        },
        2 => get_syscall_count(id) as isize,
        _ => -1,
    }
}

/// map a user area: `port` bit0/bit1/bit2 mean R/W/X
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    trace!("kernel: sys_mmap");
    // only R/W/X bits are meaningful, and at least one is required
    if port == 0 || port & !0x7 != 0 {
        return -1;
    }
    let mut map_perm = MapPermission::U;
    if port & 0x1 != 0 {
        map_perm |= MapPermission::R;
    }
    if port & 0x2 != 0 {
        map_perm |= MapPermission::W;
    }
    if port & 0x4 != 0 {
        map_perm |= MapPermission::X;
    }
    if mmap_current(VirtAddr(start), len, map_perm) {
        0
    } else {
        -1
    }
}

/// unmap a user area
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    if munmap_current(VirtAddr(start), len) {
        0
    } else {
        -1
    }
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
