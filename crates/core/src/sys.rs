//! 跨 source 共享的系统工具:进程探测、当前时间、只读 sqlite 打开。
//! 抽离自 claude.rs / hermes/gateway.rs(pid_alive)与 openclaw/mod.rs / hermes/mod.rs
//! (now_ms / 只读 sqlite)的重复实现 —— 单一事实源。

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags};

/// `kill(pid, 0) == 0` → 进程存活。ESRCH(不存在)返回非 0;pid 非法兜底 false。
/// SAFETY: signal 0 不发信号,只探测进程存在性;pid 来自本地状态文件,非敌对输入。
pub(crate) fn pid_alive(pid: u32) -> bool {
    let pid = i32::try_from(pid).unwrap_or(-1);
    pid >= 0 && unsafe { libc::kill(pid, 0) == 0 }
}

/// 当前 epoch 毫秒(时钟倒跳兜底为 0)。
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 只读打开 sqlite(`READ_ONLY | NO_MUTEX`,WAL 友好)。失败 → None。
pub(crate) fn open_readonly(path: &Path) -> Option<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}
