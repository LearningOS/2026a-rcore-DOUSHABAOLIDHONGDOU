# 实验一：多道程序与分时多任务

## 一、实验目的

在第三章内核的基础上实现 `sys_trace` 系统调用，使应用程序能够：

1. 读取当前进程用户地址空间中的一个字节；
2. 向当前进程用户地址空间写入一个字节；
3. 查询当前进程对某个系统调用的调用次数。

同时需要在任务控制块中维护每个任务的系统调用统计信息。

## 二、实验环境

| 组件 | 版本 |
| --- | --- |
| Rust 工具链 | `nightly-2024-05-02` |
| 目标平台 | `riscv64gc-unknown-none-elf` |
| QEMU | 8.2.x |
| 固件 | RustSBI 0.4.1 |

## 三、实验实现

### 3.1 扩展任务控制块

在 `os/src/config.rs` 中新增常量：

```rust
pub const MAX_SYSCALL_NUM: usize = 500;
```

在 `TaskControlBlock` 中增加系统调用计数数组：

```rust
pub struct TaskControlBlock {
    pub task_status: TaskStatus,
    pub task_cx: TaskContext,
    pub syscall_times: [u32; MAX_SYSCALL_NUM],
}
```

每个任务拥有独立的计数数组，初始化为 0。`MAX_SYSCALL_NUM` 取 500，覆盖本实验用到的
`SYSCALL_TRACE = 410`。

### 3.2 在陷入处理中统计系统调用

统计放在 `trap_handler` 的 `UserEnvCall` 分支中、调用 `syscall()` **之前**：

```rust
Trap::Exception(Exception::UserEnvCall) => {
    cx.sepc += 4;
    add_syscall_count(cx.x[17]);          // 先计数
    cx.x[10] = syscall(cx.x[17], [cx.x[10], cx.x[11], cx.x[12]]) as usize;
}
```

先计数再分发有两个好处：

- 所有系统调用（包括 `sys_trace` 自身）都会被计入；
- `sys_trace(Syscall, id, 0)` 查询时能看到本次调用，符合测例
  `assert_eq!(2, count_syscall(SYSCALL_TRACE))` 的语义。

`TaskManager` 中新增 `add_syscall_count` / `get_syscall_count` 两个方法，通过
`current_task` 索引访问当前任务的控制块。

### 3.3 实现 sys_trace

`TraceRequest` 的三个变体在用户库中按声明顺序取值为 `Read = 0`、`Write = 1`、
`Syscall = 2`。内核实现：

```rust
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    match trace_request {
        0 => unsafe { (id as *const u8).read_volatile() as isize },
        1 => {
            unsafe { (id as *mut u8).write_volatile(data as u8) };
            0
        }
        2 => get_syscall_count(id) as isize,
        _ => -1,
    }
}
```

- `Read`：把 `id` 当作用户地址读取一个字节；
- `Write`：把 `data` 的低 8 位写入 `id` 指向的地址；
- `Syscall`：返回当前任务对系统调用 `id` 的调用次数。

第三章尚未启用分页，内核与用户程序处于同一物理地址空间，因此可以直接解引用用户传入的
地址，不需要页表翻译。

## 四、测试结果

```
get_time OK! ...
Test sleep OK!
current time_msec = ...
time_msec = ... after sleeping 100 ticks, delta = ...ms!
Test sleep1 passed!
string from task trace test
Test trace OK!

Test passed: 3/3
```

`ch3_sleep`、`ch3_sleep1`、`ch3_trace` 三个测例全部通过。

## 五、实验总结

本实验的关键是理清系统调用计数与 `sys_trace` 查询的先后关系：只有先计数、后处理，
`trace` 才能观察到自己，测例中 `TRACE` 的期望值 2 和 7 都依赖于这一顺序。此外，
`trace` 读写的地址来自用户态，在启用分页的后续章节中必须通过页表翻译并检查权限，
这一点在实验二中予以完善。
