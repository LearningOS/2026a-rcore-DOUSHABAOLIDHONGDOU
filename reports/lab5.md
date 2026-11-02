# 实验五：并发与同步

## 一、实验目的

第八章引入线程与同步原语，本实验要求：

1. 支持一个进程内的多个线程（`thread_create` / `waittid`）；
2. 实现互斥锁、信号量与条件变量，并区分忙等锁与阻塞锁；
3. 实现死锁检测：当一次加锁/`down` 会形成死锁时，系统调用返回 `-0xdead`
   而不是永久阻塞。

## 二、实验环境

| 组件 | 版本 |
| --- | --- |
| Rust 工具链 | `nightly-2024-05-02` |
| 目标平台 | `riscv64gc-unknown-none-elf` |
| QEMU | 8.2.x 及兼容版本 |
| 固件 | RustSBI 0.4.1 |
| 检查器 | `LearningOS/rCore-Tutorial-Checker` + `rCore-Tutorial-Test` |

## 三、实现思路

### 3.1 等待图与可运行性分析

在 `os/src/sync/deadlock.rs` 中维护一个按进程隔离的等待图：

- 每个互斥锁/信号量记录 `holders`（当前持有者）与 `waiters`（阻塞者）；
- `blocked` 表记录每个线程当前等待的资源。

线程准备阻塞前，先把它登记进图中，然后做一次“可运行性”推演：

1. 初始可运行的线程 = 图中所有线程里当前没有阻塞的线程；
2. 反复扫描：若某个被阻塞线程所等待资源的**任一持有者**可运行（该持有者
   迟早会释放资源），则该线程也可运行；
3. 若推演结束后申请者仍不可运行，则说明它一旦阻塞就永远无法被唤醒——
   这是一次死锁，撤销登记并返回 `false`。

这样处理的优点是能正确区分多实例资源：semaphore 2 测试里虽然等待关系
出现了环，但环外还有可运行的持有者会释放资源，因此不会被误判。

### 3.2 在同步原语中挂钩

- `MutexBlocking::lock` 在真正睡眠前调用 `deadlock::before_block`，返回
  `false` 时不入等待队列，直接返回失败；
- `MutexBlocking::unlock` 在唤醒等待者时，把持有者转交给被唤醒的线程；
- `Semaphore::down` 同理；`Semaphore::up` 在唤醒等待者时转移一个“单位”；
- 线程退出（`exit_current_and_run_next`）时把它从等待图中清除，进程退出时
  清除整个进程的记录，避免悬挂的等待关系导致误判。

### 3.3 系统调用返回码

`sys_mutex_lock` 与 `sys_semaphore_down` 根据加锁/`down` 的返回值决定：

```rust
if mutex.lock() { 0 } else { -0xdead }
```

`sys_enable_deadlock_detect(enabled)` 设置全局开关，关闭时清空等待图。

### 3.4 补充 `sys_get_time`

用户库中的 `sleep()` 是“读时间 + `yield`”的忙等实现，而第八章的
`sys_get_time` 尚未实现会返回 -1，导致死锁测试在 `sleep` 中空转。
因此本实验同时补上了 `sys_get_time`：由 `get_time_us()` 构造 `TimeVal`，
再用 `translated_byte_buffer` 按页拷贝回用户地址空间，天然支持结构体跨页。

## 四、实验结果

执行：

```sh
make -C ci-user test CHAPTER=8 BASE=2
```

结果：`Test passed: 25/25`，包括线程、信号量、条件变量、哲学家就餐、
自旋锁与三个死锁检测测试（`deadlock test mutex 1`、`semaphore 1`、
`semaphore 2`）全部通过。

## 五、遇到的问题

1. **持有者没有登记**：`acquire` 只在进程已有等待图记录时才写入，导致第一次
   加锁没有记录持有者，自锁时误以为资源空闲。改为 `entry().or_insert_with()`
   总是创建记录后解决。
2. **用户 `sleep` 卡死**：见 3.4，补上 `sys_get_time` 后三个死锁测试才通过。
3. **多实例资源的误判**：仅做环检测会把 semaphore 2 的场景误判为死锁，改为
   基于“可运行持有者”的推演后，能区分“暂时等待”与“真正死锁”。
