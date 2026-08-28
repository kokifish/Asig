//! `~/.zcode/cli/db/db.sqlite` 只读查询:活跃顶层会话 + 每会话尾部 message/part 信号。
//! message/part 走 `message_session_sequence_idx` / `part_message_sequence_idx`(尾部
//! LIMIT 1 相关子查询,窗口内会话数少,毫秒级)。时间列本就是毫秒,不换算。

use rusqlite::{Connection, Row};

/// 单会话扁平视图(session 表 + 尾部 message/part 信号,已解析成判定输入)。
pub(crate) struct SessionRow {
    pub(crate) session_id: String,
    pub(crate) cwd: Option<String>,
    pub(crate) title: Option<String>,
    /// 尾部 message 的 role(user/assistant)。
    pub(crate) last_role: String,
    /// 尾部 assistant message 是否已写完(`time.completed` 存在)。
    pub(crate) last_completed: bool,
    /// 尾部 message 带 `error` 字段(模型流错误等)。
    pub(crate) last_error: bool,
    /// 尾部 part 的 `type`(step-finish/tool/text/…)。
    pub(crate) last_part_type: Option<String>,
    /// 尾部 tool part 的 `state.status`(pending/running/completed/error)。
    pub(crate) last_tool_status: Option<String>,
    /// 尾部 step-finish 的 `reason`(stop/tool-calls)。
    pub(crate) last_finish_reason: Option<String>,
    /// 尾部 part 的 time_updated(毫秒;判 pending 停留时长用)。
    pub(crate) last_part_updated_at: u64,
    /// 尾部 message 的 time_created(毫秒)。
    pub(crate) last_msg_at: u64,
    /// 最近一条真人 user 消息的文本(Panel start 事件用)。无 → 空串。
    pub(crate) last_user_content: String,
    /// 最近一条 assistant 消息的文本(Panel done 事件用)。无 → 空串。
    pub(crate) last_assistant_content: String,
}

/// 活跃窗口内的顶层会话(parent_id IS NULL 滤 subagent_child)+ 各自尾部信号。
/// `now_ms` = 0(时钟未就绪)→ 空,防历史垃圾。
pub(crate) fn active_sessions(conn: &Connection, now_ms: u64) -> rusqlite::Result<Vec<SessionRow>> {
    if now_ms == 0 {
        return Ok(Vec::new());
    }
    let cutoff = now_ms.saturating_sub(super::ACTIVE_WINDOW_MS) as i64;
    let mut stmt = conn.prepare(
        "SELECT s.id, s.directory, s.title,
                COALESCE((SELECT json_extract(m.data, '$.role') FROM message m
                          WHERE m.session_id = s.id
                          ORDER BY m.sequence DESC, m.time_created DESC LIMIT 1), '') AS last_role,
                COALESCE((SELECT json_extract(m.data, '$.time.completed') FROM message m
                          WHERE m.session_id = s.id
                          ORDER BY m.sequence DESC, m.time_created DESC LIMIT 1)
                         IS NOT NULL, 0) AS last_completed,
                COALESCE((SELECT json_extract(m.data, '$.error') FROM message m
                          WHERE m.session_id = s.id
                          ORDER BY m.sequence DESC, m.time_created DESC LIMIT 1)
                         IS NOT NULL, 0) AS last_error,
                (SELECT json_extract(p.data, '$.type') FROM message m
                 JOIN part p ON p.message_id = m.id
                 WHERE m.session_id = s.id
                 ORDER BY m.sequence DESC, m.time_created DESC,
                          p.sequence DESC, p.time_created DESC LIMIT 1) AS last_part_type,
                (SELECT CASE WHEN json_extract(p.data, '$.type') = 'tool'
                             THEN json_extract(p.data, '$.state.status') END
                 FROM message m
                 JOIN part p ON p.message_id = m.id
                 WHERE m.session_id = s.id
                 ORDER BY m.sequence DESC, m.time_created DESC,
                          p.sequence DESC, p.time_created DESC LIMIT 1) AS last_tool_status,
                (SELECT CASE WHEN json_extract(p.data, '$.type') = 'step-finish'
                             THEN json_extract(p.data, '$.reason') END
                 FROM message m
                 JOIN part p ON p.message_id = m.id
                 WHERE m.session_id = s.id
                 ORDER BY m.sequence DESC, m.time_created DESC,
                          p.sequence DESC, p.time_created DESC LIMIT 1) AS last_finish_reason,
                COALESCE((SELECT p.time_updated FROM message m
                          JOIN part p ON p.message_id = m.id
                          WHERE m.session_id = s.id
                          ORDER BY m.sequence DESC, m.time_created DESC,
                                   p.sequence DESC, p.time_created DESC LIMIT 1), 0) AS last_part_updated_at,
                COALESCE((SELECT m.time_created FROM message m
                          WHERE m.session_id = s.id
                          ORDER BY m.sequence DESC, m.time_created DESC LIMIT 1),
                         s.time_updated) AS last_msg_at,
                COALESCE((SELECT json_extract(p.data, '$.text') FROM message m
                          JOIN part p ON p.message_id = m.id
                          WHERE m.session_id = s.id
                            AND json_extract(m.data, '$.role') = 'user'
                            AND COALESCE(json_extract(m.data, '$.synthetic'), 0) = 0
                            AND json_extract(p.data, '$.type') = 'text'
                          ORDER BY m.sequence DESC, p.sequence DESC LIMIT 1), '') AS last_user_content,
                COALESCE((SELECT json_extract(p.data, '$.text') FROM message m
                          JOIN part p ON p.message_id = m.id
                          WHERE m.session_id = s.id
                            AND json_extract(m.data, '$.role') = 'assistant'
                            AND json_extract(p.data, '$.type') = 'text'
                          ORDER BY m.sequence DESC, p.sequence DESC LIMIT 1), '') AS last_assistant_content
         FROM session s
         WHERE s.parent_id IS NULL
           AND s.time_updated >= ?1
         ORDER BY s.time_updated DESC",
    )?;
    let rows = stmt.query_map([cutoff], map_row)?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn map_row(r: &Row) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        session_id: r.get::<_, String>(0)?,
        cwd: r.get::<_, Option<String>>(1)?,
        title: r.get::<_, Option<String>>(2)?,
        last_role: r.get::<_, String>(3)?,
        last_completed: r.get::<_, i64>(4)? != 0,
        last_error: r.get::<_, i64>(5)? != 0,
        last_part_type: r.get::<_, Option<String>>(6)?,
        last_tool_status: r.get::<_, Option<String>>(7)?,
        last_finish_reason: r.get::<_, Option<String>>(8)?,
        last_part_updated_at: r.get::<_, i64>(9)?.max(0) as u64,
        last_msg_at: r.get::<_, i64>(10)?.max(0) as u64,
        last_user_content: r.get::<_, String>(11)?,
        last_assistant_content: r.get::<_, String>(12)?,
    })
}
