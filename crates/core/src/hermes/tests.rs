//! hermes 集成测试:in-memory sqlite 测 `discover_from` / `classify_session`,
//! 临时文件测 `gateway::snapshot`。不碰真实 `~/.hermes`。

use super::*;
use rusqlite::{Connection, params};

const NOW: u64 = 10_000_000_000; // 固定 now(毫秒)

/// 建最小 schema(只含查询用到的列),seed 后返回内存连接。
fn db(seed: impl FnOnce(&Connection)) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE sessions (
            id TEXT PRIMARY KEY, source TEXT, started_at REAL, ended_at REAL,
            end_reason TEXT, display_name TEXT, title TEXT, cwd TEXT,
            archived INTEGER DEFAULT 0, handoff_error TEXT);
         CREATE TABLE messages (
            id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, content TEXT,
            tool_calls TEXT, tool_name TEXT, finish_reason TEXT,
            timestamp REAL, active INTEGER DEFAULT 1);
         CREATE TABLE async_delegations (
            delegation_id TEXT PRIMARY KEY, origin_session TEXT, state TEXT,
            completed_at REAL);",
    )
    .unwrap();
    seed(&conn);
    conn
}

fn session(conn: &Connection, id: &str, source: &str) {
    conn.execute(
        "INSERT INTO sessions(id, source, started_at, archived) VALUES(?1, ?2, ?3, 0)",
        params![id, source, (NOW as f64) / 1000.0],
    )
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn session_full(
    conn: &Connection,
    id: &str,
    source: &str,
    cwd: Option<&str>,
    display: Option<&str>,
    title: Option<&str>,
    end_reason: Option<&str>,
    handoff: Option<&str>,
) {
    conn.execute(
        "INSERT INTO sessions(id, source, started_at, archived, cwd, display_name, title, end_reason, handoff_error)
         VALUES(?1, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8)",
        params![id, source, (NOW as f64) / 1000.0, cwd, display, title, end_reason, handoff],
    )
    .unwrap();
}

/// `ts_sec_offset`:相对 NOW 的秒偏移(负=过去)。timestamp 存**秒**(同生产 schema)。
fn msg(
    conn: &Connection,
    id: i64,
    sid: &str,
    role: &str,
    finish: Option<&str>,
    tool_calls: bool,
    ts_sec_offset: i64,
) {
    let ts = (NOW as f64) / 1000.0 + ts_sec_offset as f64;
    let tc: Option<&str> = if tool_calls { Some("[{}]") } else { None };
    conn.execute(
        "INSERT INTO messages(id, session_id, role, finish_reason, tool_calls, timestamp, active)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, 1)",
        params![id, sid, role, finish, tc, ts],
    )
    .unwrap();
}

fn status_of(conn: &Connection, active_agents: u32) -> AgentStatus {
    let s = discover_from(conn, NOW, active_agents);
    assert_eq!(s.len(), 1, "单会话测试应有且仅有一个");
    s[0].status
}

#[test]
fn empty_db_returns_empty() {
    let conn = db(|_| {});
    assert!(discover_from(&conn, NOW, 0).is_empty());
}

#[test]
fn now_zero_returns_empty() {
    let conn = db(|c| session(c, "s1", "cli"));
    assert!(discover_from(&conn, 0, 0).is_empty());
}

#[test]
fn feishu_session_filtered() {
    let conn = db(|c| {
        session(c, "s1", "feishu");
        msg(c, 1, "s1", "assistant", Some("stop"), false, -1);
    });
    assert!(discover_from(&conn, NOW, 0).is_empty());
}

#[test]
fn archived_session_filtered() {
    let conn = db(|c| {
        c.execute(
            "INSERT INTO sessions(id, source, started_at, archived) VALUES('s1','cli',?1,1)",
            params![(NOW as f64) / 1000.0],
        )
        .unwrap();
        msg(c, 1, "s1", "assistant", Some("stop"), false, -1);
    });
    assert!(discover_from(&conn, NOW, 0).is_empty());
}

#[test]
fn zombie_session_filtered() {
    // 最后消息 >30min 不显示(实证的 cli 僵尸会话)
    let conn = db(|c| {
        session(c, "s1", "cli");
        msg(c, 1, "s1", "assistant", Some("stop"), false, -(31 * 60));
    });
    assert!(discover_from(&conn, NOW, 0).is_empty());
}

#[test]
fn ended_session_filtered() {
    // ended_at NOT NULL(已 cli_close / new_session 等)不显示,即便 last_msg 在 30min 内
    let conn = db(|c| {
        c.execute(
            "INSERT INTO sessions(id, source, started_at, ended_at, archived)
             VALUES('s1','cli',?1,?2,0)",
            params![(NOW as f64) / 1000.0, (NOW as f64) / 1000.0],
        )
        .unwrap();
        msg(c, 1, "s1", "assistant", Some("stop"), false, -1);
    });
    assert!(discover_from(&conn, NOW, 0).is_empty());
}

#[test]
fn working_tool_calls() {
    let conn = db(|c| {
        session(c, "s1", "cli");
        msg(c, 1, "s1", "assistant", Some("tool_calls"), true, -1);
    });
    assert_eq!(status_of(&conn, 0), AgentStatus::Working);
}

#[test]
fn working_user_tail() {
    let conn = db(|c| {
        session(c, "s1", "cli");
        msg(c, 1, "s1", "user", None, false, -1);
    });
    assert_eq!(status_of(&conn, 0), AgentStatus::Working);
}

#[test]
fn working_tool_tail() {
    let conn = db(|c| {
        session(c, "s1", "cli");
        msg(c, 1, "s1", "tool", None, false, -1);
    });
    assert_eq!(status_of(&conn, 0), AgentStatus::Working);
}

#[test]
fn needs_deci_recent_stop() {
    // assistant+stop + active_agents=0 + 最后消息 ≤10min → NeedsDeci
    let conn = db(|c| {
        session(c, "s1", "cli");
        msg(c, 1, "s1", "assistant", Some("stop"), false, -(5 * 60));
    });
    assert_eq!(status_of(&conn, 0), AgentStatus::NeedsDeci);
}

#[test]
fn done_stop_beyond_idle() {
    // assistant+stop + active_agents=0 + 最后消息 >10min(仍 <30min 活跃窗)→ Done
    let conn = db(|c| {
        session(c, "s1", "cli");
        msg(c, 1, "s1", "assistant", Some("stop"), false, -(11 * 60));
    });
    assert_eq!(status_of(&conn, 0), AgentStatus::Done);
}

#[test]
fn stop_with_active_agents_is_working() {
    // assistant+stop 但 active_agents>0 → Working(保守)
    let conn = db(|c| {
        session(c, "s1", "cli");
        msg(c, 1, "s1", "assistant", Some("stop"), false, -1);
    });
    assert_eq!(status_of(&conn, 2), AgentStatus::Working);
}

#[test]
fn error_end_reason_contains_error() {
    let conn = db(|c| {
        session_full(
            c,
            "s1",
            "cli",
            None,
            None,
            None,
            Some("error: crashed"),
            None,
        );
        msg(c, 1, "s1", "assistant", Some("stop"), false, -1);
    });
    assert_eq!(status_of(&conn, 0), AgentStatus::Error);
}

#[test]
fn error_handoff_field() {
    let conn = db(|c| {
        session_full(
            c,
            "s1",
            "cli",
            None,
            None,
            None,
            None,
            Some("handoff failed: timeout"),
        );
        msg(c, 1, "s1", "assistant", Some("stop"), false, -1);
    });
    assert_eq!(status_of(&conn, 0), AgentStatus::Error);
}

#[test]
fn error_failed_delegation() {
    let conn = db(|c| {
        session(c, "s1", "cli");
        msg(c, 1, "s1", "assistant", Some("stop"), false, -1);
        c.execute(
            "INSERT INTO async_delegations(delegation_id, origin_session, state, completed_at)
             VALUES('d1', 's1', 'failed', ?1)",
            params![(NOW as f64) / 1000.0],
        )
        .unwrap();
    });
    assert_eq!(status_of(&conn, 0), AgentStatus::Error);
}

#[test]
fn label_prefers_display_name() {
    let conn = db(|c| {
        session_full(
            c,
            "s1",
            "cli",
            Some("/work/proj"),
            Some("Disp"),
            Some("Title"),
            None,
            None,
        );
        msg(c, 1, "s1", "assistant", Some("stop"), false, -1);
    });
    let s = &discover_from(&conn, NOW, 0)[0];
    assert_eq!(s.label.as_deref(), Some("Disp"));
    assert_eq!(s.id, "Hermes:s1");
    assert_eq!(s.native_id, "s1");
    assert_eq!(s.cwd.as_deref(), Some(std::path::Path::new("/work/proj")));
}

#[test]
fn label_falls_back_to_title() {
    let conn = db(|c| {
        session_full(
            c,
            "s1",
            "cli",
            Some("/work/proj"),
            None,
            Some("My Title"),
            None,
            None,
        );
        msg(c, 1, "s1", "assistant", Some("stop"), false, -1);
    });
    assert_eq!(
        discover_from(&conn, NOW, 0)[0].label.as_deref(),
        Some("My Title")
    );
}

#[test]
fn label_falls_back_to_cwd_basename() {
    let conn = db(|c| {
        session_full(
            c,
            "abc12345-67890",
            "cli",
            Some("/work/proj"),
            None,
            None,
            None,
            None,
        );
        msg(c, 1, "abc12345-67890", "assistant", Some("stop"), false, -1);
    });
    assert_eq!(
        discover_from(&conn, NOW, 0)[0].label.as_deref(),
        Some("proj")
    );
}

#[test]
fn classify_session_unit() {
    use super::db::SessionRow;
    fn row(role: &str, finish: Option<&str>, age_ms: u64) -> SessionRow {
        SessionRow {
            session_id: "x".into(),
            cwd: None,
            display_name: None,
            title: None,
            end_reason: None,
            handoff_error: None,
            last_role: role.into(),
            last_finish_reason: finish.map(String::from),
            last_msg_at: NOW - age_ms,
        }
    }
    // Error 优先(即便尾部是 stop)
    assert_eq!(
        classify_session(&row("assistant", Some("stop"), 1000), NOW, 0, true),
        AgentStatus::Error
    );
    // Working(尾部信号)
    assert_eq!(
        classify_session(&row("assistant", Some("tool_calls"), 1000), NOW, 0, false),
        AgentStatus::Working
    );
    assert_eq!(
        classify_session(&row("user", None, 1000), NOW, 0, false),
        AgentStatus::Working
    );
    assert_eq!(
        classify_session(&row("tool", None, 1000), NOW, 0, false),
        AgentStatus::Working
    );
    // stop + active_agents>0 → Working(保守)
    assert_eq!(
        classify_session(&row("assistant", Some("stop"), 1000), NOW, 3, false),
        AgentStatus::Working
    );
    // stop + idle + 近期 → NeedsDeci
    assert_eq!(
        classify_session(
            &row("assistant", Some("stop"), 5 * 60 * 1000),
            NOW,
            0,
            false
        ),
        AgentStatus::NeedsDeci
    );
    // stop + idle + 超 10min → Done
    assert_eq!(
        classify_session(
            &row("assistant", Some("stop"), 11 * 60 * 1000),
            NOW,
            0,
            false
        ),
        AgentStatus::Done
    );
}

// ── gateway::snapshot 测试(临时文件,不碰真实 ~/.hermes)──

fn tmp(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("asig-hermes-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn snapshot_missing_file_is_dead() {
    let dir = tmp("missing");
    assert_eq!(super::gateway::snapshot(&dir.join("nope.json")), (false, 0));
}

#[test]
fn snapshot_corrupt_json_is_dead() {
    let dir = tmp("corrupt");
    let p = dir.join("g.json");
    std::fs::write(&p, "{bad json").unwrap();
    assert_eq!(super::gateway::snapshot(&p), (false, 0));
}

#[test]
fn snapshot_dead_pid_is_dead() {
    let dir = tmp("deadpid");
    let p = dir.join("g.json");
    std::fs::write(&p, r#"{"pid":999999,"active_agents":0}"#).unwrap();
    assert_eq!(super::gateway::snapshot(&p), (false, 0));
}

#[test]
fn snapshot_alive_self_pid() {
    let dir = tmp("alive");
    let p = dir.join("g.json");
    let pid = std::process::id();
    std::fs::write(&p, format!(r#"{{"pid":{pid},"active_agents":3}}"#)).unwrap();
    assert_eq!(super::gateway::snapshot(&p), (true, 3));
}

#[test]
#[ignore = "手动:ASIG_HERMES_ROOT=<测试目录> cargo test -- --ignored probe_env --nocapture"]
fn probe_env_discovers_all_states() {
    if std::env::var_os("ASIG_HERMES_ROOT").is_none() {
        eprintln!("未设 ASIG_HERMES_ROOT,跳过");
        return;
    }
    let src = HermesSource::new().expect("HermesSource::new");
    let sessions = src.discover();
    eprintln!("=== discover 返回 {} 个会话 ===", sessions.len());
    for s in &sessions {
        eprintln!(
            "  {} [{:?}] label={}",
            s.id,
            s.status,
            s.label.as_deref().unwrap_or("-")
        );
    }
    assert!(
        sessions.iter().any(|s| s.status == AgentStatus::Working),
        "应有 Working"
    );
    assert!(
        sessions.iter().any(|s| s.status == AgentStatus::NeedsDeci),
        "应有 NeedsDeci"
    );
    assert!(
        sessions.iter().any(|s| s.status == AgentStatus::Done),
        "应有 Done"
    );
}
