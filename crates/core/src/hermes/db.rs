//! state.db 只读查询:活跃 cli/tui 会话 + 每会话尾部 message + 近期 failed async_delegations。
//! 走 `idx_messages_session`(每会话尾部相关子查询 LIMIT 1,会话数少,毫秒级)。

use rusqlite::{Connection, Row};
use std::collections::HashSet;

/// 错误关键字(小写包含匹配 end_reason / handoff_error)。
const ERROR_KEYS: &[&str] = &["error", "fail", "panic"];

/// 单会话扁平视图(取自 sessions 表 + 其尾部 message)。
pub(crate) struct SessionRow {
    pub(crate) session_id: String,
    pub(crate) cwd: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) end_reason: Option<String>,
    pub(crate) handoff_error: Option<String>,
    /// 尾部 message 的 role(user/assistant/tool)。
    pub(crate) last_role: String,
    /// 尾部 message 的 finish_reason(stop/tool_calls/...)。
    pub(crate) last_finish_reason: Option<String>,
    /// 尾部 message 的 timestamp(**毫秒**,sessions/messages 存秒,×1000)。
    pub(crate) last_msg_at: u64,
}

impl SessionRow {
    /// `end_reason` 或 `handoff_error` 含 error 关键字。
    pub(crate) fn has_error(&self) -> bool {
        [self.end_reason.as_deref(), self.handoff_error.as_deref()]
            .iter()
            .filter_map(|s| *s)
            .any(|t| {
                let lo = t.to_lowercase();
                ERROR_KEYS.iter().any(|k| lo.contains(k))
            })
    }
}

/// 取所有 cli/tui 未归档会话 + 各自尾部 message(role / finish_reason / 时间)。
/// 时间过滤(>30min 僵尸)由上层 `discover_from` 做。
///
/// 相关子查询(每会话 LIMIT 1)而非全表窗口排序:state.db ~300MB,messages 全表窗口函数
/// 会很慢;按 `idx_messages_session` 取每会话尾部,会话数少,快。
pub(crate) fn active_sessions(conn: &Connection) -> rusqlite::Result<Vec<SessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.cwd, s.display_name, s.title, s.end_reason, s.handoff_error,
                COALESCE((SELECT m.role FROM messages m
                          WHERE m.session_id = s.id
                          ORDER BY m.timestamp DESC, m.id DESC LIMIT 1), '') AS last_role,
                (SELECT m.finish_reason FROM messages m
                 WHERE m.session_id = s.id
                 ORDER BY m.timestamp DESC, m.id DESC LIMIT 1) AS last_finish_reason,
                COALESCE((SELECT CAST(m.timestamp * 1000.0 AS INTEGER) FROM messages m
                          WHERE m.session_id = s.id
                          ORDER BY m.timestamp DESC, m.id DESC LIMIT 1),
                         CAST(s.started_at * 1000.0 AS INTEGER)) AS last_msg_at
         FROM sessions s
         WHERE s.source IN ('cli', 'tui') AND s.archived = 0 AND s.ended_at IS NULL
         ORDER BY s.started_at DESC",
    )?;
    let rows = stmt.query_map([], map_row)?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn map_row(r: &Row) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        session_id: r.get::<_, String>(0)?,
        cwd: r.get::<_, Option<String>>(1)?,
        display_name: r.get::<_, Option<String>>(2)?,
        title: r.get::<_, Option<String>>(3)?,
        end_reason: r.get::<_, Option<String>>(4)?,
        handoff_error: r.get::<_, Option<String>>(5)?,
        last_role: r.get::<_, String>(6)?,
        last_finish_reason: r.get::<_, Option<String>>(7)?,
        last_msg_at: r.get::<_, i64>(8)?.max(0) as u64,
    })
}

/// 有近期 failed `async_delegations` 的 origin_session 集合(决策 5 的 Error 信号)。
/// `completed_at` 存秒,×1000 与 now(ms) 比;表/列缺失(老 schema)→ 空集容错。
pub(crate) fn failed_delegation_sessions(
    conn: &Connection,
    now: u64,
) -> rusqlite::Result<HashSet<String>> {
    let cutoff_ms = now.saturating_sub(super::ACTIVE_WINDOW_MS) as i64;
    let mut stmt = match conn.prepare(
        "SELECT DISTINCT origin_session FROM async_delegations
         WHERE state = 'failed'
           AND (completed_at IS NULL OR CAST(completed_at * 1000.0 AS INTEGER) >= ?1)",
    ) {
        Ok(s) => s,
        Err(_) => return Ok(HashSet::new()), // 表缺失(老 schema)容错
    };
    let rows = stmt.query_map([cutoff_ms], |r| r.get::<_, String>(0))?;
    let mut set = HashSet::new();
    for r in rows {
        set.insert(r?);
    }
    Ok(set)
}
