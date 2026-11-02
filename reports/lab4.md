# 实验四：文件系统

## 一、实验目的

第七章把内核与 easy-fs 文件系统连接起来，本实验要求：

1. 实现 `sys_fstat`：通过文件描述符取得文件的元数据；
2. 实现 `sys_linkat`：为已有文件创建一个硬链接；
3. 实现 `sys_unlinkat`：删除一个目录项，并在链接数归零时回收 inode 与数据块。

## 二、实验环境

| 组件 | 版本 |
| --- | --- |
| Rust 工具链 | `nightly-2024-05-02` |
| 目标平台 | `riscv64gc-unknown-none-elf` |
| QEMU | 8.2.x 及兼容版本 |
| 固件 | RustSBI 0.4.1 |
| 文件系统 | 仓库内置 `easy-fs` |
| 检查器 | `LearningOS/rCore-Tutorial-Checker` + `rCore-Tutorial-Test` |

## 三、实现思路

### 3.1 DiskInode 增加链接计数

`easy-fs/src/layout.rs` 的 `DiskInode` 增加 `pub nlink: u32` 字段，`initialize()`
时置为 1（新创建的文件/目录都有一个目录项指向它）。磁盘布局按
`core::mem::size_of::<DiskInode>()` 动态计算，只要镜像由同一份 easy-fs 生成，
增加字段是安全的。

### 3.2 easy-fs 增加 link / unlink

在 `easy-fs/src/vfs.rs` 的 `Inode` 上实现：

- `inode_id()`：用 `(block_id << 32) | block_offset` 生成稳定的 inode 编号，
  供 `fstat` 填写 `ino`；
- `link(old_name, new_name)`：旧文件不存在或新名字已存在则返回 -1；否则把旧
  inode 的 `nlink` 加一，并在目录末尾追加一个 `DirEntry`；
- `unlink(name)`：找到目录项下标与 inode 编号，`nlink` 减一；归零时用
  `clear_size` 释放全部数据块（`dealloc_data`）并释放 inode（`dealloc_inode`）；
  最后把后续目录项整体左移，目录大小减去一个 `DIRENT_SZ`。

`EasyFileSystem` 另外补充了 `dealloc_inode`，通过 inode bitmap 回收 inode。

### 3.3 File trait 与 OSInode::stat

`os/src/fs/mod.rs` 的 `File` trait 增加带默认实现的 `fn stat(&self) -> Option<Stat>`，
管道、标准输入输出沿用默认的 `None`。`OSInode` 覆写它，用 `is_dir()` 选择
`StatMode::DIR` / `StatMode::FILE`，并填入 `inode_id()` 与 `nlink()`。

### 3.4 三个系统调用

- `sys_fstat(fd, st)`：取当前进程的 `fd_table`，越界或空的 fd 返回 -1，否则
  `*translated_refmut(token, st) = file.stat()?` 并返回 0；
- `sys_linkat(old_name, new_name)`、`sys_unlinkat(name)`：用 `translated_str`
  读出用户态路径字符串，转交 `ROOT_INODE.link` / `ROOT_INODE.unlink`。

### 3.5 其他实验的兼容

第七章没有 `loader` 模块，应用都从文件系统读入，因此 `sys_spawn` 改为通过
`open_file(path, OpenFlags::RDONLY)` + `read_all()` 取得 ELF 数据；同时保留
`get_time`、`mmap`、`munmap`、`spawn`、`set_priority`（stride 调度），以保证
检查器中前面章节的测试仍能通过。

## 四、实验结果

执行：

```sh
make -C ci-user test CHAPTER=6 BASE=2
```

结果：`Test passed: 31/31`，`fstat`、`link`、`mass open/unlink`、`file_test`
等全部通过。

## 五、遇到的问题

1. **误用 loader**：照搬第五章的 `get_app_data_by_name` 导致
   `unresolved import crate::loader`。改为从文件系统读取 ELF 后解决。
2. **missing_docs**：`ROOT_INODE` 被 `pub use` 导出后触发 `#![deny(missing_docs)]`，
   在 `lazy_static!` 内部补上文档注释即可。
3. **删除目录项的边界**：`unlink` 必须同时对目录的 `size` 减 `DIRENT_SZ`，
   否则 `ls` 会读到已删除的残留项。
