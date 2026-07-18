//! OpenClaw 状态出口:只读主状态库 + 各 agent 交互式会话 jsonl,按 agent 聚合映射状态。
//!
//! 两套数据源:
//! 1. **后台任务** — `~/.openclaw/state/openclaw.sqlite`(WAL,单一事实源、`.migrated`
//!    合并目标,升级只朝它收敛):只读打开 → 查询 → 丢弃;WAL 允许 N 读 + 1 写并发,
//!    不抢 openclaw 写锁;连接局部、源结构只存 root,天然 Send+Sync。
//! 2. **交互式会话** — `agents/<id>/sessions/<sid>.jsonl`(TUI/webchat 事件流,**不进主库**):
//!    读尾部最后一条 message 的 `role` + `message.stopReason` 判在跑(类比 Claude 的
//!    `stop_reason`)—— 不依赖写入连续性,工具执行的长间隙不会误判完成。
//!
//! 状态映射(每个近期 agent 一个会话):
//!   - 后台:task_runs/flow_runs/subagent_runs 跨表归并(`ended_at IS NULL`→Working);
//!   - 交互式:尾部 `role∈{user,toolResult}` 或 `stop_reason='toolUse'` → Working;
//!   - `flow_runs.status='blocked'` 且 `ended_at IS NULL` → NeedsDeci(已结束的 blocked
//!     投递失败终态不计);
//!   - 近期(`ERROR_RECENT_MS` 内)failed/lost/subagent-error → Error;否则 Done。
//!
//! 每 agent 取一个观测态(Error > NeedsDeci > Working > Done);Error 过窗后报 Done,
//! 由 `lib.rs` 的 sticky `transition()` 自动解锁。

mod db;
mod probe;
mod sessions;

pub use probe::{AgentProbe, probe};

use crate::source::{AgentKind, AgentSession, AgentSource};
use crate::status::AgentStatus;
use db::{AgentAcc, collect};
use rusqlite::Connection;
use sessions::{SessionSignal, latest_session_signals};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct OpenClawSource {
    root: PathBuf,
}

impl OpenClawSource {
    pub fn new() -> Option<Self> {
        Some(Self {
            root: dirs::home_dir()?.join(".openclaw"),
        })
    }

    /// 主库路径(cli 打印 / 单一事实源)。
    pub fn db_path(&self) -> PathBuf {
        self.root.join("state").join("openclaw.sqlite")
    }

    /// 会话 jsonl 根目录(discover/probe 扫 agents/<id>/sessions 用)。
    pub(super) fn root_path(&self) -> &Path {
        &self.root
    }

    /// 只读打开主库(WAL,不抢 openclaw 写锁)。失败 → None(discover/probe 据此回退)。
    fn connect(&self) -> Option<Connection> {
        use rusqlite::OpenFlags;
        Connection::open_with_flags(
            self.db_path(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .ok()
    }
}

impl AgentSource for OpenClawSource {
    fn kind(&self) -> AgentKind {
        AgentKind::OpenClaw
    }

    fn discover(&self) -> Vec<AgentSession> {
        let Some(conn) = self.connect() else {
            // 打不开:没装 openclaw(静默)vs 库损坏(应可见)。提示路径便于排障。
            eprintln!("Asig: openclaw 库打不开: {}", self.db_path().display());
            return Vec::new();
        };
        let signals = latest_session_signals(&self.root);
        discover_from(&conn, now_ms(), &signals)
    }
}

/// 查询 + 归并核心(纯函数:接连接 + 当前 ms + 各 agent 交互式会话尾部信号)。
fn discover_from(
    conn: &Connection,
    now: u64,
    session_signals: &HashMap<String, SessionSignal>,
) -> Vec<AgentSession> {
    collect(conn, now, session_signals)
        .into_iter()
        .map(|(aid, acc, _)| AgentSession {
            kind: AgentKind::OpenClaw,
            id: format!("OpenClaw:{aid}"),
            native_id: aid.clone(),
            cwd: None,
            project: None,
            status: classify_agent(acc),
            label: Some(aid),
        })
        .collect()
}

/// 纯函数:单 agent 观测态(优先级 Error > NeedsDeci > Working > Done)。
/// Error 同时覆盖「近期 failed」(recent_err)与「协调态卡死」(stuck)。
fn classify_agent(a: AgentAcc) -> AgentStatus {
    if a.recent_err || a.stuck {
        AgentStatus::Error
    } else if a.blocked {
        AgentStatus::NeedsDeci
    } else if a.running || a.run_active {
        AgentStatus::Working
    } else {
        AgentStatus::Done
    }
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::db::{AGENT_RECENT_MS, agent_of};
    use super::*;
    use rusqlite::{Connection, params};
    use std::collections::HashMap;

    /// 建最小 schema(只含查询用到的列),seed 后返回内存连接。
    fn db(seed: impl FnOnce(&Connection)) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE agent_databases (
                agent_id TEXT NOT NULL, path TEXT,
                schema_version INTEGER, last_seen_at INTEGER, size_bytes INTEGER,
                PRIMARY KEY(agent_id, path));
             CREATE TABLE task_runs (
                task_id TEXT PRIMARY KEY, agent_id TEXT, status TEXT NOT NULL,
                ended_at INTEGER);
             CREATE TABLE flow_runs (
                flow_id TEXT PRIMARY KEY, owner_key TEXT, status TEXT NOT NULL,
                ended_at INTEGER);
             CREATE TABLE subagent_runs (
                run_id TEXT PRIMARY KEY, requester_display_key TEXT,
                ended_at INTEGER, ended_reason TEXT);",
        )
        .unwrap();
        seed(&conn);
        conn
    }

    const NOW: u64 = 10_000_000_000; // 固定 now(单测不取系统时间)

    fn agent(conn: &Connection, aid: &str, last_seen: i64) {
        conn.execute(
            "INSERT INTO agent_databases(agent_id, path, schema_version, last_seen_at)
             VALUES(?1, 'p', 1, ?2)",
            params![aid, last_seen],
        )
        .unwrap();
    }

    fn task(conn: &Connection, aid: &str, status: &str, ended_at: Option<i64>) {
        let suffix = match ended_at {
            Some(x) => x.to_string(),
            None => "n".into(),
        };
        let id = format!("{aid}-{status}-{suffix}");
        conn.execute(
            "INSERT INTO task_runs(task_id, agent_id, status, ended_at) VALUES(?1, ?2, ?3, ?4)",
            params![id, aid, status, ended_at],
        )
        .unwrap();
    }

    fn status_of(conn: &Connection) -> AgentStatus {
        let s = discover_from(conn, NOW, &HashMap::new());
        assert_eq!(s.len(), 1, "单 agent 测试应有且仅有一个会话");
        s[0].status
    }

    #[test]
    fn classify_priority() {
        // Error(含 stuck 卡死) > NeedsDeci > Working(含 run_active) > Done
        assert_eq!(
            classify_agent(AgentAcc {
                running: true,
                run_active: true,
                blocked: true,
                recent_err: true,
                stuck: true
            }),
            AgentStatus::Error
        );
        assert_eq!(
            classify_agent(AgentAcc {
                running: true,
                run_active: true,
                blocked: true,
                recent_err: false,
                stuck: false
            }),
            AgentStatus::NeedsDeci
        );
        // run_active(后台 run 在跑)也算 Working。
        assert_eq!(
            classify_agent(AgentAcc {
                running: false,
                run_active: true,
                blocked: false,
                recent_err: false,
                stuck: false
            }),
            AgentStatus::Working
        );
        // stuck(协调态卡死)→ Error。
        assert_eq!(
            classify_agent(AgentAcc {
                running: false,
                run_active: false,
                blocked: false,
                recent_err: false,
                stuck: true
            }),
            AgentStatus::Error
        );
        assert_eq!(classify_agent(AgentAcc::default()), AgentStatus::Done);
    }

    #[test]
    fn agent_of_parses_prefix() {
        assert_eq!(agent_of("agent:munger:dashboard:abc"), Some("munger"));
        assert_eq!(agent_of("agent:main:main"), Some("main"));
        assert_eq!(agent_of("agent::x"), None); // 空 id
        // 无 agent: 前缀的纯 agent_id(subagent_runs 旧格式 requester)→ 认。
        assert_eq!(agent_of("main"), Some("main"));
        // 非该前缀的长 session key(含 :)→ 不认,避免误归。
        assert_eq!(agent_of("session:foo"), None);
    }

    #[test]
    fn empty_db_no_sessions() {
        let conn = db(|_| {});
        assert!(discover_from(&conn, NOW, &HashMap::new()).is_empty());
    }

    #[test]
    fn idle_agent_is_done() {
        let conn = db(|c| {
            agent(c, "main", NOW as i64);
        });
        assert_eq!(status_of(&conn), AgentStatus::Done);
    }

    /// 构造一个交互式会话尾部信号(agent=kotomi,mtime 默认近期,普通会话:非 leaf、非协调态)。
    fn sig(role: &str, stop: Option<&str>, age_ms: u64) -> (String, SessionSignal) {
        sig_ext(role, stop, age_ms, false, false)
    }

    /// 同上,但可指定 ends_with_leaf + coordinating(用于 sessions_yield 协调态测试)。
    fn sig_ext(
        role: &str,
        stop: Option<&str>,
        age_ms: u64,
        leaf: bool,
        coord: bool,
    ) -> (String, SessionSignal) {
        (
            "kotomi".into(),
            SessionSignal {
                mtime_ms: NOW - age_ms,
                role: role.into(),
                stop: stop.map(String::from),
                ends_with_leaf: leaf,
                coordinating: coord,
            },
        )
    }

    #[test]
    fn session_tooluse_is_working() {
        // 尾部 assistant + stop_reason='toolUse' → 模型在跑工具链 → Working。
        // (工具执行的长间隙也不会误判:只看尾部 stop_reason,不靠 mtime 连续。)
        let conn = db(|c| {
            agent(c, "kotomi", NOW as i64);
        });
        let s = discover_from(
            &conn,
            NOW,
            &HashMap::from([sig("assistant", Some("toolUse"), 5_000)]),
        );
        assert_eq!(s[0].status, AgentStatus::Working);
    }

    #[test]
    fn session_toolresult_is_working() {
        // 尾部 toolResult → 工具结果,模型要继续 → Working。
        let conn = db(|c| {
            agent(c, "kotomi", NOW as i64);
        });
        let s = discover_from(&conn, NOW, &HashMap::from([sig("toolResult", None, 5_000)]));
        assert_eq!(s[0].status, AgentStatus::Working);
    }

    #[test]
    fn session_user_message_is_working() {
        // 尾部 user → 刚发消息,模型要处理 → Working。
        let conn = db(|c| {
            agent(c, "kotomi", NOW as i64);
        });
        let s = discover_from(&conn, NOW, &HashMap::from([sig("user", None, 5_000)]));
        assert_eq!(s[0].status, AgentStatus::Working);
    }

    #[test]
    fn session_assistant_end_is_done() {
        // 尾部 assistant 且 stop 非 toolUse(模型回完等用户)→ 不在跑 → Done。
        let conn = db(|c| {
            agent(c, "kotomi", NOW as i64);
        });
        let s = discover_from(
            &conn,
            NOW,
            &HashMap::from([sig("assistant", Some("end_turn"), 5_000)]),
        );
        assert_eq!(s[0].status, AgentStatus::Done);
    }

    #[test]
    fn session_stale_role_is_done() {
        // user/toolResult 尾部 + 过 SESSION_RECENT_MS(无新活动)→ 不算在跑 → Done。
        let conn = db(|c| {
            agent(c, "kotomi", NOW as i64);
        });
        let s = discover_from(
            &conn,
            NOW,
            &HashMap::from([sig("user", None, 6 * 60 * 1000)]),
        );
        assert_eq!(s[0].status, AgentStatus::Done);
    }

    #[test]
    fn session_tooluse_stale_still_working() {
        // stopReason='toolUse' 是「模型已发工具调用、工具在执行」的权威在跑信号:即便 jsonl
        // 已 6 分钟没新写入(长工具),也保持 Working —— toolUse 跳过 mtime 闸门,否则 >5min
        // 的工具会误判完成闪蓝(本次修复 3 的核心)。
        let conn = db(|c| {
            agent(c, "kotomi", NOW as i64);
        });
        let s = discover_from(
            &conn,
            NOW,
            &HashMap::from([sig("assistant", Some("toolUse"), 6 * 60 * 1000)]),
        );
        assert_eq!(s[0].status, AgentStatus::Working);
    }

    #[test]
    fn session_yield_leaf_subagents_running_is_working() {
        // 协调态 + 子 agent 在跑(task_runs running → run_active)→ 正常 Working。
        let conn = db(|c| {
            agent(c, "kotomi", NOW as i64);
            task(c, "kotomi", "running", None); // sessions_spawn 子 agent 在 task_runs running
        });
        let s = discover_from(
            &conn,
            NOW,
            &HashMap::from([sig_ext("assistant", Some("stop"), 60_000, true, true)]),
        );
        assert_eq!(s[0].status, AgentStatus::Working);
    }

    #[test]
    fn session_yield_leaf_subagents_ended_is_error() {
        // 协调态 + 子 agent 全 ended(!run_active)→ 主 agent 等不到 announce → 卡死 → Error(B)。
        // (实测:7 子 agent 全 succeeded,主 session 停 leaf+yield 无最终输出。)
        let conn = db(|c| {
            agent(c, "kotomi", NOW as i64);
        });
        let s = discover_from(
            &conn,
            NOW,
            &HashMap::from([sig_ext("assistant", Some("stop"), 60_000, true, true)]),
        );
        assert_eq!(s[0].status, AgentStatus::Error);
    }

    #[test]
    fn session_yield_leaf_stale_is_error() {
        // 协调态超 SUBAGENT_WAIT_MS(30min)→ 即使子 agent 在跑也判卡死 → Error(A 兜底)。
        // (用 running task 让 run_active=true,避开 B,纯测 A 超时兜底。)
        let conn = db(|c| {
            agent(c, "kotomi", NOW as i64);
            task(c, "kotomi", "running", None);
        });
        let s = discover_from(
            &conn,
            NOW,
            &HashMap::from([sig_ext(
                "assistant",
                Some("stop"),
                31 * 60 * 1000,
                true,
                true,
            )]),
        );
        assert_eq!(s[0].status, AgentStatus::Error);
    }

    #[test]
    fn session_leaf_without_yield_is_done() {
        // leaf 结尾但尾部无 yield/spawn(普通回合结束,非协调态)→ 不算在跑 → Done。
        let conn = db(|c| {
            agent(c, "kotomi", NOW as i64);
        });
        let s = discover_from(
            &conn,
            NOW,
            &HashMap::from([sig_ext("assistant", Some("stop"), 60_000, true, false)]),
        );
        assert_eq!(s[0].status, AgentStatus::Done);
    }

    #[test]
    fn running_task_is_working() {
        let conn = db(|c| {
            agent(c, "main", NOW as i64);
            task(c, "main", "running", None); // ended_at NULL
        });
        assert_eq!(status_of(&conn), AgentStatus::Working);
    }

    #[test]
    fn blocked_flow_is_needs_deci() {
        let conn = db(|c| {
            agent(c, "kotomi", NOW as i64);
            c.execute(
                "INSERT INTO flow_runs(flow_id, owner_key, status, ended_at)
                 VALUES('f', 'agent:kotomi:tui-x', 'blocked', NULL)",
                [],
            )
            .unwrap();
        });
        assert_eq!(status_of(&conn), AgentStatus::NeedsDeci);
    }

    #[test]
    fn ended_blocked_flow_is_done() {
        // blocked 但 ended_at 有值(已结束的投递失败终态)→ 不算 NeedsDeci → Done。
        // 否则历史失败的 cron 会让该 agent 永远 🟠(用户看到的「一直运行中」根因)。
        let ended = NOW as i64 - 1_000;
        let conn = db(|c| {
            agent(c, "main", NOW as i64);
            c.execute(
                "INSERT INTO flow_runs(flow_id, owner_key, status, ended_at)
                 VALUES('f', 'agent:main:cron-x', 'blocked', ?1)",
                params![ended],
            )
            .unwrap();
        });
        assert_eq!(status_of(&conn), AgentStatus::Done);
    }

    #[test]
    fn flow_failed_null_ended_is_error() {
        // flow status='failed' 且 ended_at IS NULL(崩溃/写入撕裂)→ 应判 Error,而非 Working。
        // 修复 2 前:分支顺序 blocked→running→failed 把这种行当 running,掩盖了 Error。
        let conn = db(|c| {
            agent(c, "main", NOW as i64);
            c.execute(
                "INSERT INTO flow_runs(flow_id, owner_key, status, ended_at)
                 VALUES('f', 'agent:main:crash', 'failed', NULL)",
                [],
            )
            .unwrap();
        });
        assert_eq!(status_of(&conn), AgentStatus::Error);
    }

    #[test]
    fn recent_failed_is_error() {
        let ended = NOW as i64 - 5_000; // 5s 前,在 30s 窗口内
        let conn = db(|c| {
            agent(c, "main", NOW as i64);
            task(c, "main", "failed", Some(ended));
        });
        assert_eq!(status_of(&conn), AgentStatus::Error);
    }

    #[test]
    fn stale_failed_is_done() {
        // 1 分钟前的 failed —— 超出 30s 窗口 → 不再报 Error → Done
        let ended = NOW as i64 - 60_000;
        let conn = db(|c| {
            agent(c, "main", NOW as i64);
            task(c, "main", "failed", Some(ended));
        });
        assert_eq!(status_of(&conn), AgentStatus::Done);
    }

    #[test]
    fn error_beats_running_same_agent() {
        let conn = db(|c| {
            agent(c, "main", NOW as i64);
            task(c, "main", "running", None);
            task(c, "main", "failed", Some(NOW as i64 - 1_000));
        });
        assert_eq!(status_of(&conn), AgentStatus::Error);
    }

    #[test]
    fn flow_failed_via_owner_prefix() {
        let ended = NOW as i64 - 2_000;
        let conn = db(|c| {
            agent(c, "munger", NOW as i64);
            c.execute(
                "INSERT INTO flow_runs(flow_id, owner_key, status, ended_at)
                 VALUES('f', 'agent:munger:dashboard:y', 'failed', ?1)",
                params![ended],
            )
            .unwrap();
        });
        assert_eq!(status_of(&conn), AgentStatus::Error);
    }

    #[test]
    fn subagent_running_via_display_key() {
        let conn = db(|c| {
            agent(c, "main", NOW as i64);
            c.execute(
                "INSERT INTO subagent_runs(run_id, requester_display_key, ended_at, ended_reason)
                 VALUES('r', 'agent:main:tui-z', NULL, NULL)",
                [],
            )
            .unwrap();
        });
        assert_eq!(status_of(&conn), AgentStatus::Working);
    }

    #[test]
    fn stale_agent_filtered() {
        // last_seen_at 早于 30 天窗口 → 被滤掉(防历史垃圾 agent)
        let old = NOW as i64 - AGENT_RECENT_MS as i64 - 1;
        let conn = db(|c| {
            agent(c, "ghost", old);
        });
        assert!(discover_from(&conn, NOW, &HashMap::new()).is_empty());
    }

    #[test]
    fn collect_now_zero_returns_empty() {
        // 时钟未就绪(now=0):cutoff 会失效(last_seen_at>=0 全过),collect 早返回空。
        let conn = db(|c| {
            agent(c, "ghost", 0);
        });
        assert!(collect(&conn, 0, &HashMap::new()).is_empty());
    }
}
