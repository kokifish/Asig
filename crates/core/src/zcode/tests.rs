//! zcode 集成测试:in-memory sqlite(仿 zcode db 最小 schema)测 `discover_from` /
//! `classify_session`。不碰真实 `~/.zcode`。

use super::*;
use rusqlite::{Connection, params};
use std::path::Path;

const NOW: u64 = 10_000_000_000; // 固定 now(毫秒)
const ZROOT: &str = "/zz/.zcode"; // 测试用 zcode 根目录(cwd 在其下 = 无项目会话)

/// 建最小 schema(session/message/part,只含查询用到的列;sequence 显式填)。
fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE session (
            id TEXT PRIMARY KEY, parent_id TEXT, directory TEXT, title TEXT,
            task_type TEXT DEFAULT 'interactive', time_archived INTEGER,
            time_created INTEGER, time_updated INTEGER);
         CREATE TABLE message (
            id TEXT PRIMARY KEY, session_id TEXT, data TEXT,
            time_created INTEGER, time_updated INTEGER, sequence INTEGER);
         CREATE TABLE part (
            id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, data TEXT,
            time_created INTEGER, time_updated INTEGER, sequence INTEGER);",
    )
    .unwrap();
    conn
}

/// 种一个会话;`age_ms` = 最后更新距 NOW 的毫秒(负=过去;默认窗口内)。
fn session(conn: &Connection, id: &str, cwd: Option<&str>, age_ms: i64) {
    conn.execute(
        "INSERT INTO session(id, parent_id, directory, time_created, time_updated)
         VALUES(?1, NULL, ?2, ?3, ?4)",
        params![id, cwd, NOW as i64 - 1000, NOW as i64 + age_ms],
    )
    .unwrap();
}

/// 种 subagent 子会话(parent_id 非空)——不应出现在 discover 结果里。
fn subagent_child(conn: &Connection, id: &str, parent: &str) {
    conn.execute(
        "INSERT INTO session(id, parent_id, directory, time_created, time_updated)
         VALUES(?1, ?2, '/tmp/sub', ?3, ?3)",
        params![id, parent, NOW as i64],
    )
    .unwrap();
}

/// 种一条 message(时间默认紧跟 NOW)+ 可选 parts(顺序追加)。返回 message id。
fn msg(
    conn: &Connection,
    sid: &str,
    seq: i64,
    data: &str,
    parts: &[(&str, &str)], // (part type, 额外 JSON 片段,拼进 {"type":...})
) -> String {
    let mid = format!("{sid}m{seq}");
    conn.execute(
        "INSERT INTO message(id, session_id, data, time_created, time_updated, sequence)
         VALUES(?1, ?2, ?3, ?4, ?4, ?5)",
        params![mid, sid, data, NOW as i64 - 10, seq],
    )
    .unwrap();
    for (i, (ptype, extra)) in parts.iter().enumerate() {
        let pdata = match *extra {
            "" => format!("{{\"type\":\"{ptype}\"}}"),
            e => format!("{{\"type\":\"{ptype}\",{e}}}"),
        };
        conn.execute(
            "INSERT INTO part(id, message_id, session_id, data, time_created, time_updated, sequence)
             VALUES(?1, ?2, ?3, ?4, ?5, ?5, ?6)",
            params![format!("{mid}p{i}"), mid, sid, pdata, NOW as i64 - 10, i as i64],
        )
        .unwrap();
    }
    mid
}

fn user_msg(synthetic: bool, completed: Option<u64>) -> String {
    let syn = if synthetic { ",\"synthetic\":true" } else { "" };
    match completed {
        Some(t) => {
            format!("{{\"role\":\"user\",\"time\":{{\"created\":1,\"completed\":{t}}}{syn}}}")
        }
        None => format!("{{\"role\":\"user\",\"time\":{{\"created\":1}}{syn}}}"),
    }
}

fn assistant_msg(completed: bool, error: bool) -> String {
    let time = if completed {
        "{\"created\":1,\"completed\":2}"
    } else {
        "{\"created\":1}"
    };
    let err = if error {
        ",\"error\":{\"name\":\"AiSdkModelAdapterError\"}"
    } else {
        ""
    };
    format!("{{\"role\":\"assistant\",\"time\":{time}{err}}}")
}

/// 尾部带取消类 error(zcode 归档会话 / Esc 中断在途请求时写入,真实结构同款)。
fn assistant_msg_cancelled(completed: bool) -> String {
    let time = if completed {
        "{\"created\":1,\"completed\":2}"
    } else {
        "{\"created\":1}"
    };
    format!(
        "{{\"role\":\"assistant\",\"time\":{time},\"error\":{{\"name\":\"AiSdkModelAdapterError\",\
         \"data\":{{\"message\":\"Model request was cancelled.\",\"code\":\"model_request_cancelled\"}}}}}}"
    )
}

fn status_of(sessions: &[AgentSession], id_suffix: &str) -> AgentStatus {
    sessions
        .iter()
        .find(|s| s.id.ends_with(id_suffix))
        .map(|s| s.status)
        .unwrap_or_else(|| panic!("session {id_suffix} 未找到"))
}

// ---- classify:四态逐一 ----

#[test]
fn working_when_last_message_user() {
    let conn = db();
    session(&conn, "s1", Some("/w/a"), 0);
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg(true, false),
        &[("step-finish", "\"reason\":\"stop\"")],
    );
    msg(
        &conn,
        "s1",
        1,
        &user_msg(false, None),
        &[("text", "\"text\":\"跑一下\"")],
    );
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(status_of(&ss, "s1"), AgentStatus::Working);
    // start 事件内容 = 最近真人 user 文本
    let s = ss.iter().find(|s| s.id.ends_with("s1")).unwrap();
    assert_eq!(s.last_user_msg.as_deref(), Some("跑一下"));
}

#[test]
fn working_when_assistant_streaming() {
    let conn = db();
    session(&conn, "s1", Some("/w/a"), 0);
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg(false, false),
        &[("text", "\"text\":\"想\"")],
    );
    assert_eq!(
        status_of(&discover_from(&conn, NOW, Path::new(ZROOT)), "s1"),
        AgentStatus::Working
    );
}

#[test]
fn working_when_finish_reason_tool_calls() {
    let conn = db();
    session(&conn, "s1", Some("/w/a"), 0);
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg(true, false),
        &[("step-finish", "\"reason\":\"tool-calls\"")],
    );
    assert_eq!(
        status_of(&discover_from(&conn, NOW, Path::new(ZROOT)), "s1"),
        AgentStatus::Working
    );
}

#[test]
fn done_when_finish_reason_stop() {
    let conn = db();
    session(&conn, "s1", Some("/w/d"), 0);
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg(true, false),
        &[
            ("text", "\"text\":\"完成了\""),
            ("step-finish", "\"reason\":\"stop\""),
        ],
    );
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(status_of(&ss, "s1"), AgentStatus::Done);
    let s = ss.iter().find(|s| s.id.ends_with("s1")).unwrap();
    assert_eq!(s.last_assistant_msg.as_deref(), Some("完成了"));
}

#[test]
fn needs_deci_when_tool_pending_stale() {
    // pending 停留 ≥PENDING_CONFIRM_MS(等授权)→ NeedsDeci。
    let conn = db();
    session(&conn, "s1", Some("/w/n"), 0);
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg(false, false),
        &[("tool", "\"state\":{\"status\":\"pending\",\"input\":{}}")],
    );
    age_tail_part(&conn, "s1", (PENDING_CONFIRM_MS as i64) + 5_000);
    assert_eq!(
        status_of(&discover_from(&conn, NOW, Path::new(ZROOT)), "s1"),
        AgentStatus::NeedsDeci
    );
}

#[test]
fn working_when_tool_pending_fresh() {
    // pending 刚调度(排队瞬态,yolo 常态)→ Working,不误报 🟠。
    let conn = db();
    session(&conn, "s1", Some("/w/n"), 0);
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg(false, false),
        &[("tool", "\"state\":{\"status\":\"pending\",\"input\":{}}")],
    );
    age_tail_part(&conn, "s1", 2_000);
    assert_eq!(
        status_of(&discover_from(&conn, NOW, Path::new(ZROOT)), "s1"),
        AgentStatus::Working
    );
}

/// 把某会话「尾部 part」的 time_updated 改为 NOW-`off_ms`(测 pending 停留时长)。
fn age_tail_part(conn: &Connection, sid: &str, off_ms: i64) {
    conn.execute(
        "UPDATE part SET time_updated = ?1 WHERE id = (
            SELECT p.id FROM message m JOIN part p ON p.message_id = m.id
            WHERE m.session_id = ?2
            ORDER BY m.sequence DESC, p.sequence DESC LIMIT 1)",
        params![NOW as i64 - off_ms, sid],
    )
    .unwrap();
}

#[test]
fn working_when_tool_running() {
    let conn = db();
    session(&conn, "s1", Some("/w/r"), 0);
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg(false, false),
        &[("tool", "\"state\":{\"status\":\"running\",\"input\":{}}")],
    );
    assert_eq!(
        status_of(&discover_from(&conn, NOW, Path::new(ZROOT)), "s1"),
        AgentStatus::Working
    );
}

#[test]
fn error_when_last_message_has_error() {
    let conn = db();
    session(&conn, "s1", Some("/w/e"), 0);
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg(true, true),
        &[("text", "\"text\":\"x\"")],
    );
    assert_eq!(
        status_of(&discover_from(&conn, NOW, Path::new(ZROOT)), "s1"),
        AgentStatus::Error
    );
}

#[test]
fn cancelled_error_not_error() {
    // 归档会话 / Esc 中断在途请求 → 尾部 error code=model_request_cancelled(用户主动
    // 取消,非失败)→ 不判 Error;回合已收尾(completed + stop)→ Done。
    let conn = db();
    session(&conn, "s1", Some("/w/c"), 0);
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg_cancelled(true),
        &[
            ("text", "\"text\":\"x\""),
            ("step-finish", "\"reason\":\"stop\""),
        ],
    );
    assert_eq!(
        status_of(&discover_from(&conn, NOW, Path::new(ZROOT)), "s1"),
        AgentStatus::Done
    );
}

#[test]
fn cancelled_error_does_not_pull_group_to_error() {
    // 回归 2026-09-04:归档的会话尾部留 cancelled error(zcode 归档取消在途请求,
    // 不写 time_archived),同 cwd 的活跃会话不被它拉成 Error。
    let conn = db();
    session(&conn, "cancelled", Some("/w/g"), -60_000);
    session(&conn, "live", Some("/w/g"), 0);
    msg(
        &conn,
        "cancelled",
        0,
        &assistant_msg_cancelled(true),
        &[("text", "\"text\":\"x\"")],
    );
    msg(
        &conn,
        "live",
        0,
        &assistant_msg(true, false),
        &[("step-finish", "\"reason\":\"stop\"")],
    );
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(ss.len(), 1, "同 cwd 聚合为一行");
    assert_eq!(ss[0].status, AgentStatus::Done, "cancelled 不拉组");
}

#[test]
fn archived_session_hidden() {
    // time_archived 非空 → 不显示(zcode 当前版本归档不写它;写了即生效)。
    let conn = db();
    session(&conn, "live", Some("/w/f"), 0);
    session(&conn, "archived", Some("/w/a"), 0);
    conn.execute(
        "UPDATE session SET time_archived = ?1 WHERE id = 'archived'",
        params![NOW as i64],
    )
    .unwrap();
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(ss.len(), 1);
    assert!(ss[0].id.ends_with("live"));
}

#[test]
fn synthetic_user_not_used_for_start_content() {
    // 尾部是工具结果(synthetic user)→ Working;start 事件内容取更早的真人输入。
    let conn = db();
    session(&conn, "s1", Some("/w/s"), 0);
    msg(
        &conn,
        "s1",
        0,
        &user_msg(false, None),
        &[("text", "\"text\":\"真人输入\"")],
    );
    msg(
        &conn,
        "s1",
        1,
        &user_msg(true, Some(3)),
        &[("text", "\"text\":\"tool result\"")],
    );
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(status_of(&ss, "s1"), AgentStatus::Working);
    let s = ss.iter().find(|s| s.id.ends_with("s1")).unwrap();
    assert_eq!(s.last_user_msg.as_deref(), Some("真人输入"));
}

// ---- 窗口过滤 / 分组聚合 / subagent ----

#[test]
fn stale_session_outside_window_hidden() {
    let conn = db();
    session(&conn, "fresh", Some("/w/f"), 0);
    session(
        &conn,
        "stale",
        Some("/w/s"),
        -(ACTIVE_WINDOW_MS as i64 + 60_000),
    );
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(ss.len(), 1);
    assert!(ss[0].id.ends_with("fresh"));
}

#[test]
fn subagent_sessions_excluded() {
    let conn = db();
    session(&conn, "s1", Some("/w/a"), 0);
    subagent_child(&conn, "child1", "s1");
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(ss.len(), 1, "subagent_child 不应出现");
    assert!(ss[0].id.ends_with("s1"));
}

#[test]
fn same_cwd_grouped_with_most_active_status() {
    let conn = db();
    session(&conn, "s1", Some("/w/g"), -5_000);
    session(&conn, "s2", Some("/w/g"), 0);
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg(true, false),
        &[("step-finish", "\"reason\":\"stop\"")],
    );
    msg(
        &conn,
        "s2",
        0,
        &assistant_msg(false, false),
        &[("text", "\"text\":\"working\"")],
    );
    // s2 的消息更晚(组代表 = 最新者;相同时间戳下 HashMap 序不稳定)。
    conn.execute(
        "UPDATE message SET time_created = ?1 WHERE id = 's2m0'",
        params![NOW as i64 - 5],
    )
    .unwrap();
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(ss.len(), 1, "同 cwd 聚合为一行");
    assert_eq!(
        ss[0].status,
        AgentStatus::Working,
        "组内任一 Working 拉起整组"
    );
    // 代表 = 组内最新(s2)
    assert!(ss[0].id.ends_with("s2"));
    assert_eq!(ss[0].cwd.as_deref(), Some(std::path::Path::new("/w/g")));
}

#[test]
fn label_project_cwd_wins_over_title() {
    // 有项目(cwd 不在 zcode 根内)→ 项目名优先,题名不用(学 Claude Code 行)。
    let conn = db();
    session_titled(&conn, "s1", Some("/w/proj"), 0, "某个题名");
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg(true, false),
        &[("step-finish", "\"reason\":\"stop\"")],
    );
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(ss[0].label.as_deref(), Some("proj"));
}

#[test]
fn label_no_project_uses_title() {
    // 无项目(cwd 在 zcode 默认 workspace)→ zcode 自动题名(zcode 自家会话列表同款)。
    let conn = db();
    session_titled(
        &conn,
        "s1",
        Some("/zz/.zcode/workspace/default"),
        0,
        "深圳南山出行行李与穿着准备",
    );
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg(true, false),
        &[("step-finish", "\"reason\":\"stop\"")],
    );
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(ss[0].label.as_deref(), Some("深圳南山出行行李与穿着准备"));
}

#[test]
fn label_no_project_detected_via_path_segment() {
    // cwd 不在探测根下、但含 `.zcode/workspace` 目录段(如回放库场景)→ 仍判无项目,
    // 显示题名而非「default」。
    let conn = db();
    session_titled(
        &conn,
        "s1",
        Some("/Users/koki/.zcode/workspace/default"),
        0,
        "调研智谱GLM Flash模型并配置zcode",
    );
    msg(
        &conn,
        "s1",
        0,
        &assistant_msg(true, false),
        &[("step-finish", "\"reason\":\"stop\"")],
    );
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(
        ss[0].label.as_deref(),
        Some("调研智谱GLM Flash模型并配置zcode")
    );
}

#[test]
fn label_no_project_no_title_falls_to_sid() {
    // 无项目且无题名 → sid 前 8 字符(「default」这类无意义 basename 不用)。
    let conn = db();
    session(
        &conn,
        "sid1234567890",
        Some("/zz/.zcode/workspace/default"),
        0,
    );
    msg(
        &conn,
        "sid1234567890",
        0,
        &assistant_msg(true, false),
        &[("step-finish", "\"reason\":\"stop\"")],
    );
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(ss[0].label.as_deref(), Some("sid12345"));
}

#[test]
fn label_no_cwd_uses_title_then_sid() {
    let conn = db();
    session_titled(&conn, "s1", None, 0, "无目录会话题名");
    msg(&conn, "s1", 0, &assistant_msg(true, false), &[]);
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(ss[0].label.as_deref(), Some("无目录会话题名"));
    let conn = db();
    session(&conn, "sidabcdefgh", None, 0);
    msg(&conn, "sidabcdefgh", 0, &assistant_msg(true, false), &[]);
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(ss[0].label.as_deref(), Some("sidabcde"));
}

/// 种一个带题名的会话(测 label 降级链;title 即 zcode 自动生成的会话题名)。
fn session_titled(conn: &Connection, id: &str, cwd: Option<&str>, age_ms: i64, title: &str) {
    conn.execute(
        "INSERT INTO session(id, parent_id, directory, title, time_created, time_updated)
         VALUES(?1, NULL, ?2, ?3, ?4, ?5)",
        params![id, cwd, title, NOW as i64 - 1000, NOW as i64 + age_ms],
    )
    .unwrap();
}

#[test]
fn no_cwd_sessions_not_merged() {
    let conn = db();
    session(&conn, "a", None, 0);
    session(&conn, "b", None, 0);
    let ss = discover_from(&conn, NOW, Path::new(ZROOT));
    assert_eq!(ss.len(), 2, "cwd 缺失各成一组,不合并");
}

#[test]
fn zero_now_returns_empty() {
    let conn = db();
    session(&conn, "s1", Some("/w/a"), 0);
    assert!(discover_from(&conn, 0, Path::new(ZROOT)).is_empty());
}

#[test]
fn missing_message_part_columns_tolerated() {
    // 老schema缺 part 表 → 查询整体失败 → 空结果(不 panic)。
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE session (id TEXT PRIMARY KEY, parent_id TEXT, directory TEXT,
            title TEXT, time_created INTEGER, time_updated INTEGER);",
    )
    .unwrap();
    assert!(discover_from(&conn, NOW, Path::new(ZROOT)).is_empty());
}
