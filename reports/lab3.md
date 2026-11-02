# 实验三：进程管理

## 一、实验目的

第五章把“任务”升级为“进程”，本实验要求：

1. 在新进程模型下重新实现 `sys_get_time`、`sys_mmap`、`sys_munmap`；
2. 实现 `sys_spawn`：根据 ELF 文件名创建一个全新的进程（不是 fork + exec）；
3. 实现 `sys_set_priority`，并把调度器改造成 stride 调度（步长调度）。

## 二、实验环境

| 组件 | 版本 |
| --- | --- |
| Rust 工具链 | `nightly-2024-05-02` |
| 目标平台 | `riscv64gc-unknown-none-elf` |
| QEMU | 8.2.x 及兼容版本 |
| 固件 | RustSBI 0.4.1 |
| 检查器 | `LearningOS/rCore-Tutorial-Checker` + `rCore-Tutorial-Test` |

## 三、实现思路

### 3.1 地址空间相关系统调用

第五章的进程拥有独立的 `MemorySet`，因此：

- `sys_get_time`：取 `get_time_us()`，构造 `TimeVal`，再用
  `translated_byte_buffer` 把结构体按页写入用户缓冲区，天然支持跨页；
- `sys_mmap` / `sys_munmap`：把实验二的 `MemorySet::mmap` / `munmap`
  移植过来，通过 `current_task().inner_exclusive_access().memory_set` 访问
  当前进程的地址空间。`prot` 仍然按 bit0=读、bit1=写、bit2=执行解析。

### 3.2 sys_spawn

`spawn` 与 `fork + exec` 的区别在于：子进程不需要复制父进程的地址空间，
而是直接用 ELF 文件新建一套地址空间。为此在 `TaskControlBlock` 上新增：

```rust
pub fn spawn(self: &Arc<Self>, elf_data: &[u8]) -> Arc<Self>
```

它完成以下工作：

1. `MemorySet::from_elf` 构建新地址空间并取得用户栈顶与入口地址；
2. `pid_alloc` / `kstack_alloc` 分配 pid 和内核栈；
3. 构造 `TaskControlBlockInner`，`parent` 指向调用者，`children` 为空；
4. 初始化 `TrapContext`；
5. 把新进程加入父进程的 `children` 列表，以便 `waitpid` 能回收它。

`sys_spawn` 通过 `translated_str` 读出路径，查 `get_app_data_by_name`
拿到 ELF 数据，调用 `spawn` 后 `add_task` 放入就绪队列并返回 pid。

### 3.3 stride 调度

在 `TaskControlBlockInner` 中增加三个字段：

- `priority`：调度优先级，必须 ≥ 2；
- `stride`：`BIG_STRIDE / priority`，表示每被调度一次 `pass` 的增量；
- `pass`：累计的步进值。

`TaskManager::fetch` 不再简单地 `pop_front`，而是遍历就绪队列，选出
`pass` 最小的进程；取出后令 `pass += stride`。这样优先级越高的进程
`stride` 越小、被选中的频率越高，长期运行下来 CPU 时间与优先级成正比。

`sys_set_priority(prio)` 的语义：

- `prio < 2` 返回 -1；
- 否则把当前进程的 `priority` 设为 `prio`、`stride` 设为
  `BIG_STRIDE / prio`，并返回 `prio` 本身。

`BIG_STRIDE` 取 `1 << 16`，默认优先级取 16。

## 四、测试结果

```
make -C ci-user test CHAPTER=5
```

```
Test 04_1 OK!
Test 04_4 test OK!
Test 04_5 ummap OK!
Test 04_6 ummap2 OK!
Test sleep OK!
Test sleep1 passed!
Test spawn0 OK!
Test wait OK!
Test waitpid OK!
Test set_priority OK!
ch5 Usertests passed!
Test passed: 15/15
```

stride 调度的实测输出（6 个进程分别设置优先级 5~10，运行相同时间后统计循环次数）：

```
priority = 5,  exitcode = 19781600, ratio = 3956320
priority = 6,  exitcode = 24231200, ratio = 4038533
priority = 7,  exitcode = 28561600, ratio = 4080228
priority = 8,  exitcode = 32559600, ratio = 4069950
priority = 9,  exitcode = 36744800, ratio = 4082755
priority = 10, exitcode = 41225600, ratio = 4122560
```

各进程的 `count / priority` 都稳定在 4.0×10⁶ 附近，说明获得的 CPU 时间
与优先级严格成正比，stride 调度工作正常。

## 五、实验总结

本实验最关键的两点是：

1. **进程创建的语义**：`spawn` 直接以 ELF 建立新地址空间，父进程关系必须
   显式登记，否则 `waitpid` 找不到子进程；
2. **调度器的选择策略**：stride 调度把“优先级”转化为 `pass` 增长速度的
   差异，用最小 `pass` 选择下一个进程，实现简单且比例公平。
