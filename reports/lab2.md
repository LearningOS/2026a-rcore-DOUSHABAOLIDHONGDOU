# 实验二：地址空间

## 一、实验目的

在第四章引入地址空间（SV39 页表）之后，系统调用不能再直接解引用用户指针。本实验要求：

1. 让 `sys_get_time` 通过页表把时间写回用户地址空间，并能处理 `TimeVal` 跨页的情况；
2. 让 `sys_trace` 在地址空间之上完成按字节的读、写，并检查页表权限；
3. 实现 `sys_mmap` / `sys_munmap`，支持用户态按页申请、释放虚拟内存区域。

## 二、实验环境

| 组件 | 版本 |
| --- | --- |
| Rust 工具链 | `nightly-2024-05-02` |
| 目标平台 | `riscv64gc-unknown-none-elf` |
| QEMU | 8.2.x 及兼容版本 |
| 固件 | RustSBI 0.4.1 |
| 检查器 | `LearningOS/rCore-Tutorial-Checker` + `rCore-Tutorial-Test` |

## 三、实现思路

### 3.1 地址翻译的辅助函数

用户内存访问统一走一个辅助函数 `translate_user_byte(token, va, write)`：

1. 用当前进程的 `satp` 构造 `PageTable::from_token`；
2. 对 `va` 所在虚页调用 `translate` 得到 PTE；
3. 校验 PTE 有效、带 `U` 位，且写操作要求 `W`、读操作要求 `R`；
4. 用 `PTE.ppn * PAGE_SIZE + va % PAGE_SIZE` 算出物理地址，返回裸指针。

这里必须检查 `U` 位：页表遍历只使用虚拟地址的低 27 位，`isize::MAX` 这类非法地址
会在索引上“别名”到高地址的 trampoline 页，若不检查 `U` 位就会被误判为可访问。

### 3.2 sys_get_time

`TimeVal` 可能跨越两个页，因此使用 `translated_byte_buffer` 把用户缓冲区拆成
一到两个 `&mut [u8]`，再按字节把内核里构造好的 `TimeVal` 拷贝进去。

### 3.3 sys_trace

- `Read = 0`：翻译用户地址后 `read_volatile` 读一个字节；
- `Write = 1`：翻译并检查写权限后 `write_volatile` 写一个字节；
- `Syscall = 2`：返回当前任务对指定系统调用的调用次数（沿用实验一的统计）。

计数在 `trap_handler` 分发系统调用**之前**完成，因此 `sys_trace` 能统计到自己这一次调用。

### 3.4 sys_mmap / sys_munmap

在 `MemorySet` 上新增两个方法：

- `mmap(start, len, perm)`：要求 `start` 页对齐、`len > 0`；
  `len` 向上取整到整页；与已有 `MapArea` 有任何重叠都返回失败；
  通过后以 `Framed` 方式映射，并给 PTE 加上 `U` 位。
- `munmap(start, len)`：要求 `start`、`len` 都页对齐，且范围与某个用户
  `MapArea` 完全一致；找到后调用 `unmap` 回收物理页并从区域表中移除。

`sys_mmap` 的 `prot` 按 POSIX 约定解析：bit0 为读、bit1 为写、bit2 为执行；
为 0 或带其它位都返回 -1。只写（`W=1, R=0`）在 RISC-V 中是非法组合，
映射后一旦访问就会触发页错误，这正是测试 04_3 期望的行为。

## 四、测试结果

使用官方检查器在本地运行：

```
make -C ci-user test CHAPTER=4
```

关键输出：

```
get_time OK! 45
Test 04_1 OK!
Test 04_4 test OK!
Test trace_1 OK!
Test 04_5 ummap OK!
Test 04_6 ummap2 OK!
Test sleep1 passed!
Test trace OK!
Test sleep OK!
Test passed: 16/16
```

16 项全部通过，其中：

- `Test 04_1 OK!`：mmap 后可正常读写；
- `Test 04_4 test OK!`：重叠、未对齐、非法 `prot` 均被拒绝；
- `Test 04_5 ummap OK!`：munmap 后可重新映射并读写；
- `Test 04_6 ummap2 OK!`：未对齐的 munmap 返回 -1；
- `Test trace_1 OK!`：`sys_trace` 在地址空间上正确读写并检查权限；
- `Test sleep OK!` / `Test sleep1 passed!` / `Test trace OK!`：实验一的功能在新地址空间下仍然正确。

## 五、遇到的问题

1. **物理地址计算优先级错误**：写成了
   `ppn * PAGE_SIZE + va & (PAGE_SIZE - 1)`，Rust 中 `+` 的优先级高于 `&`，
   实际得到的是 `(ppn * PAGE_SIZE + va) & (PAGE_SIZE - 1)`，导致内核访问小地址
   触发 LoadPageFault。加上括号后修复。
2. **`trace_read(isize::MAX)` 误命中 trampoline**：见 3.1，补充 `U` 位检查后修复。

## 六、实验总结

本实验的核心是：**内核不能信任用户指针**。所有用户地址都必须经过页表的翻译与权限
检查；同时页表只使用低 27 位索引，必须借助 `U` 位区分用户页与内核页。
`mmap`/`munmap` 则要求在 `MemorySet` 层面维护区域表，保证映射不重叠、释放精确。
