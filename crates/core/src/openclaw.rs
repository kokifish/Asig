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

use crate::source::{AgentKind, AgentSession, AgentSource};
use crate::status::AgentStatus;
use rusqlite::{Connection, OpenFlags, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// "近期失败"窗口(ms):ended 落在此窗口内的 failed/lost/subagent-error 才报 Error。
/// 与 done_notif 同量级;过窗后 source 报 Done,sticky 状态机自动解锁 Error。
const ERROR_RECENT_MS: u64 = 30_000;
/// `agent_databases` 的 `last_seen_at` 过滤窗口(ms):只盯近期见过的 agent,滤历史垃圾。
const AGENT_RECENT_MS: u64 = 30 * 24 * 3600 * 1000; // 30 天
/// 交互式会话「近此窗口内活跃过」才看其尾部判在跑(防历史会话尾部 toolUse 永远 Working)。
/// 取 5 分钟:覆盖工具链内任意长间隙;turn 真完成后过窗即转 Done(完成通知略延后)。
const SESSION_RECENT_MS: u64 = 5 * 60 * 1000;
/// `sessions_yield` 协调态宽窗口:主 agent 用 `sessions_spawn` 派发的后台子 agent 走独立
/// trajectory、**不进 `subagent_runs` 表**,主 session 在 yield 等待期间尾部停在 `assistant
/// stop="stop"`+`leaf`(GLM 暂歇语义,非完成)。靠「文件以 `leaf` 结尾 ∧ 尾部含 yield/spawn」
/// 识别协调态 → Working。30 分钟覆盖 Deep 研究(announce 频繁刷新 mtime);过窗回落 Done 防异常卡死。
const SUBAGENT_WAIT_MS: u64 = 30 * 60 * 1000;

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

    /// 只读打开主库(WAL,不抢 openclaw 写锁)。失败 → None(discover/probe 据此回退)。
    fn connect(&self) -> Option<Connection> {
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

/// 收集每 agent 的累加器 + 最新会话信号(复用于 `discover_from` 与 `probe`;SQL/session 逻辑
/// 只此一处,杜绝 rs/sh 双实现漂移)。顺序同 `agent_databases` 返回顺序;无 run 的 agent 也含
/// (acc 默认),让面板与探针都能看到。
fn collect(
    conn: &Connection,
    now: u64,
    session_signals: &HashMap<String, SessionSignal>,
) -> Vec<(String, AgentAcc, Option<SessionSignal>)> {
    // now==0(系统时钟未就绪)→ cutoff 会失效(last_seen_at>=0 全过),早返回避免历史垃圾进结果。
    if now == 0 {
        return Vec::new();
    }
    let cutoff_err = now.saturating_sub(ERROR_RECENT_MS) as i64;
    let cutoff_agent = now.saturating_sub(AGENT_RECENT_MS) as i64;

    // 1) agent 集合:agent_databases 里近期见过的 agent_id(权威注册表)。
    let mut agents: Vec<String> = Vec::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT agent_id FROM agent_databases WHERE last_seen_at >= ?1")
    {
        if let Ok(rows) = stmt.query_map(params![cutoff_agent], |r| r.get::<_, String>(0)) {
            agents.extend(rows.flatten());
        }
    }

    // 2) 每 agent 状态累加器(预置所有 agent,确保无 run 的也输出 Done)。
    let mut acc: HashMap<&str, AgentAcc> = HashMap::new();
    for aid in &agents {
        acc.entry(aid.as_str()).or_default();
    }

    // task_runs(干净 agent_id 列,SQL 聚合 running / 近期 failed+lost)。
    if let Ok(mut stmt) = conn.prepare(
        "SELECT agent_id,
                SUM(ended_at IS NULL),
                SUM(ended_at IS NOT NULL AND ended_at > ?1 AND status IN ('failed','lost'))
         FROM task_runs GROUP BY agent_id",
    ) {
        if let Ok(rows) = stmt.query_map(params![cutoff_err], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                r.get::<_, Option<i64>>(2)?.unwrap_or(0),
            ))
        }) {
            for row in rows.flatten() {
                if let Some(aid) = row.0.as_ref() {
                    if let Some(a) = acc.get_mut(aid.as_str()) {
                        if row.1 > 0 {
                            a.run_active = true;
                        }
                        if row.2 > 0 {
                            a.recent_err = true;
                        }
                    }
                }
            }
        }
    }

    // flow_runs(owner_key `agent:<id>:…` 前缀,逐行 split)。WHERE 放进循环的行:
    //   blocked(ended_at NULL,在跑且阻塞)→ NeedsDeci;
    //   failed(任意 ended_at:近期 ended 的失败 / 或 ended NULL 的崩溃撕裂)→ Error;
    //   其余 ended_at IS NULL(运行中)→ Working。
    // 注意 failed 必须在 running 之前判:status='failed' 且 ended_at IS NULL(写入撕裂)的行,
    // 若先判 `ended_at IS NULL → running` 会把失败误当在跑,掩盖 Error。
    // 已结束的非 failed 行不进循环(WHERE 第二支限定 status='failed');已结束的 blocked
    // (cron 投递失败终态)更不在 WHERE —— 避免历史失败让 agent 永远 🟠。
    if let Ok(mut stmt) = conn.prepare(
        "SELECT owner_key, status, ended_at FROM flow_runs
         WHERE ended_at IS NULL
            OR (ended_at IS NOT NULL AND ended_at > ?1 AND status='failed')",
    ) {
        if let Ok(rows) = stmt.query_map(params![cutoff_err], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
            ))
        }) {
            for row in rows.flatten() {
                let Some(aid) = agent_of(&row.0) else {
                    continue;
                };
                let Some(a) = acc.get_mut(aid) else { continue };
                if row.1 == "blocked" {
                    a.blocked = true;
                } else if row.1 == "failed" {
                    a.recent_err = true; // 任意 ended_at:近期 failed 或 NULL 撕裂
                } else if row.2.is_none() {
                    a.run_active = true;
                }
            }
        }
    }

    // subagent_runs(requester_display_key 同前缀;running / 近期 subagent-error)。
    if let Ok(mut stmt) = conn.prepare(
        "SELECT requester_display_key, ended_at, ended_reason FROM subagent_runs
         WHERE ended_at IS NULL
            OR (ended_at IS NOT NULL AND ended_at > ?1 AND ended_reason='subagent-error')",
    ) {
        if let Ok(rows) = stmt.query_map(params![cutoff_err], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        }) {
            for row in rows.flatten() {
                let Some(aid) = agent_of(&row.0) else {
                    continue;
                };
                let Some(a) = acc.get_mut(aid) else { continue };
                if row.1.is_none() {
                    a.run_active = true;
                } else if row.2.as_deref() == Some("subagent-error") {
                    a.recent_err = true; // WHERE 已限定 ended_at > cutoff
                }
            }
        }
    }

    // 交互式会话合并 + 收集输出(含 sig,供 probe 诊断)。
    // user/toolResult 需 mtime 近 SESSION_RECENT_MS(防历史会话尾部永远 Working);
    // stopReason='toolUse' 跳过 mtime 闸门(工具长间隙不算完成)。协调态(leaf+yield):
    //   子 agent 全 ended(!run_active)或协调态超 SUBAGENT_WAIT_MS → 卡死 → Error。
    let mut out: Vec<(String, AgentAcc, Option<SessionSignal>)> = Vec::with_capacity(agents.len());
    for aid in &agents {
        let sig = session_signals.get(aid).cloned();
        let mut a = acc.remove(aid.as_str()).unwrap_or_default();
        if let Some(s) = &sig {
            let stop = s.stop.as_deref();
            let fresh = now.saturating_sub(s.mtime_ms) < SESSION_RECENT_MS;
            let stale = now.saturating_sub(s.mtime_ms) >= SUBAGENT_WAIT_MS;
            if session_running(&s.role, stop) && (fresh || stop == Some("toolUse")) {
                a.running = true;
            } else if s.ends_with_leaf && s.coordinating && (!a.run_active || stale) {
                a.stuck = true; // 协调态卡死(B 子 agent 全 ended / A 超 30min)→ Error
            }
        }
        out.push((aid.clone(), a, sig));
    }
    out
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

/// 单 agent 诊断探针(CLI `probe-openclaw` 输出用;复用 `collect`,不另写判定)。
pub struct AgentProbe {
    pub aid: String,
    pub status: AgentStatus,
    /// 尾部最后一条 message 的 role / stopReason(无会话则空 / None)。
    pub role: String,
    pub stop: Option<String>,
    /// 文件以 leaf 结尾 ∧ 尾部含 yield/spawn(协调态)。
    pub coordinating: bool,
    /// 最新会话 mtime 距 now 的秒数;无会话 = -1。
    pub age_s: i64,
    pub run_active: bool,
    pub blocked: bool,
    pub recent_err: bool,
    pub stuck: bool,
}

/// 探针:读真实 openclaw(主库 + 各 agent session),返回每 agent 诊断 + 最终 status。
/// 供 CLI `agent-light probe-openclaw` 用 —— 单一判定源,替代 scripts/probe-openclaw.sh 的
/// bash 重新实现(消除 CLAUDE.md 强制 rs/sh 同步负担)。没装 openclaw / 打不开库 → 空。
pub fn probe() -> Vec<AgentProbe> {
    let Some(src) = OpenClawSource::new() else {
        return Vec::new();
    };
    let Some(conn) = src.connect() else {
        return Vec::new();
    };
    let signals = latest_session_signals(&src.root);
    let now = now_ms();
    collect(&conn, now, &signals)
        .into_iter()
        .map(|(aid, acc, sig)| {
            let (role, stop, coordinating, age_s) = match &sig {
                Some(s) => (
                    s.role.clone(),
                    s.stop.clone(),
                    s.ends_with_leaf && s.coordinating,
                    (now.saturating_sub(s.mtime_ms) / 1000) as i64,
                ),
                None => (String::new(), None, false, -1),
            };
            AgentProbe {
                aid,
                status: classify_agent(acc),
                role,
                stop,
                coordinating,
                age_s,
                run_active: acc.run_active,
                blocked: acc.blocked,
                recent_err: acc.recent_err,
                stuck: acc.stuck,
            }
        })
        .collect()
}

/// 单 agent 的状态累加器。
#[derive(Default, Clone, Copy)]
struct AgentAcc {
    /// 最终判 Working 的「在跑」(普通 session 在跑)。
    running: bool,
    /// run 表(task/flow/subagent)有 running —— 协调态区分「子 agent 在跑」vs「卡死」用;
    /// classify 也算 Working(有后台 run 在跑)。
    run_active: bool,
    blocked: bool,
    recent_err: bool,
    /// 协调态卡死:主 agent sessions_yield 等子 agent,但子 agent 全 ended(B)或协调态超时(A)。
    stuck: bool,
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

/// 提取 agent_id:`agent:<id>:…` 前缀取首段;无前缀时本身可能是纯 agent_id
/// (subagent_runs 旧格式 requester="main",不含 `:`)→ 直接用。空或含其他结构 → None。
fn agent_of(key: &str) -> Option<&str> {
    if let Some(rest) = key.strip_prefix("agent:") {
        let id = rest.split(':').next()?;
        return (!id.is_empty()).then_some(id);
    }
    // 无 agent: 前缀:仅当是纯 agent_id(非空、不含 `:`)才认,避免把长 session key 误归。
    (!key.is_empty() && !key.contains(':')).then_some(key)
}

/// 一个 agent 最新交互式会话的尾部信号(mtime + 最后一条 message 的 role + stop_reason
/// + 文件是否以 `leaf` 结尾 + 尾部是否含 sessions_yield/spawn 协调信号)。
#[derive(Clone)]
struct SessionSignal {
    mtime_ms: u64,
    role: String,
    stop: Option<String>,
    /// 文件最后一条事件是否为 `leaf`(OpenClaw 回合 marker;yield 循环每回合以 leaf 收尾)。
    ends_with_leaf: bool,
    /// 尾部 6 条事件内是否含 `sessions_yield`/`sessions_spawn`(主 agent 在协调后台子 agent)。
    coordinating: bool,
}

/// 交互式会话尾部判在跑:user(刚发)/ toolResult(工具结果,模型继续)/ stop='toolUse'(模型
/// 发工具调用,工具在执行)。三者都表示模型还会接着动 → Working。final assistant(纯文本
/// 回复,stop 非 toolUse)→ 等用户,不算在跑。
fn session_running(role: &str, stop: Option<&str>) -> bool {
    role == "user" || role == "toolResult" || stop == Some("toolUse")
}

/// 读 jsonl 尾部(末 ~32KB),一次性算出:(最后一条 message 的 role+stop_reason,
/// 文件是否以 `leaf` 结尾, 尾部是否含 sessions_yield/spawn)。
///
/// 尾部 I/O(seek+read+lossy+丢首行+解析)走共用 `jsonl_tail::read_tail_lines`;此处只做
/// 字段提取。文件打不开 → None(上游 `unwrap_or_default` 回退空信号);空文件 → Some 空信号。
/// `message.stopReason` 如 "toolUse"/"stop"。
fn read_tail_signals(path: &Path) -> Option<(String, Option<String>, bool, bool)> {
    let events = crate::jsonl_tail::read_tail_lines(path, 32_768)?;

    // 文件最后一条事件是否为 `leaf`(OpenClaw 回合 marker)。
    let ends_with_leaf = events
        .last()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()))
        == Some("leaf");

    // 尾部 6 条事件内是否含「主 agent 协调后台子 agent」信号:
    //   - custom_message:`message.customType == "openclaw.sessions_yield"`
    //   - assistant 工具调用:`message.content[].toolCall.name ∈ {sessions_yield, sessions_spawn}`
    // (89c26b75 实测两种形式并存;倒序取末 6 条覆盖一个完整 yield 循环。)
    let coordinating = events.iter().rev().take(6).any(|v| {
        if v.get("message")
            .and_then(|m| m.get("customType"))
            .and_then(|c| c.as_str())
            == Some("openclaw.sessions_yield")
        {
            return true;
        }
        v.get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
            .is_some_and(|content| {
                content.iter().any(|b| {
                    b.get("type").and_then(|t| t.as_str()) == Some("toolCall")
                        && matches!(
                            b.get("name").and_then(|n| n.as_str()),
                            Some("sessions_yield") | Some("sessions_spawn")
                        )
                })
            })
    });

    // 反序找最后一条 type=message → (role, stopReason);无 message 则 role 空。
    for v in events.iter().rev() {
        if v.get("type").and_then(|t| t.as_str()) == Some("message") {
            let role = v
                .get("message")
                .and_then(|m| m.get("role"))
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();
            let stop = v
                .get("message")
                .and_then(|m| m.get("stopReason"))
                .and_then(|s| s.as_str())
                .map(String::from);
            return Some((role, stop, ends_with_leaf, coordinating));
        }
    }
    Some((String::new(), None, ends_with_leaf, coordinating))
}

/// 派生/历史会话文件后缀(非真实交互式会话):trajectory / deleted / bak / reset。
const SESSION_EXCLUDE: &[&str] = &[".trajectory.", ".deleted", ".bak", ".reset"];

/// 是否为真实交互式会话文件(以 `.jsonl` 结尾且非派生/历史后缀)。
fn is_active_session(name: &str) -> bool {
    name.ends_with(".jsonl") && !SESSION_EXCLUDE.iter().any(|x| name.contains(x))
}

/// 文件修改时间距 epoch 的毫秒数;取不到(文件消失 / 不支持 mtime / 时钟倒跳)→ None。
fn mtime_ms(path: &Path) -> Option<u64> {
    use std::time::UNIX_EPOCH;
    let m = path.metadata().ok()?.modified().ok()?;
    Some(m.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// 扫 `agents/<aid>/sessions/*.jsonl`,每 agent 取 mtime 最新的会话尾部信号。
/// 只看活跃会话文件(排除 `.trajectory.jsonl` / `.deleted` / `.bak` / `.reset` 等派生/历史)。
fn latest_session_signals(root: &Path) -> HashMap<String, SessionSignal> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(root.join("agents")) else {
        return out;
    };
    for e in entries.flatten() {
        let aid = e.file_name().to_string_lossy().to_string();
        let Ok(sess) = std::fs::read_dir(e.path().join("sessions")) else {
            continue;
        };
        let mut best: Option<(u64, PathBuf)> = None;
        for f in sess.flatten() {
            if !is_active_session(&f.file_name().to_string_lossy()) {
                continue;
            }
            let Some(mt) = mtime_ms(&f.path()) else {
                continue;
            };
            if best.as_ref().is_none_or(|(b, _)| mt > *b) {
                best = Some((mt, f.path()));
            }
        }
        if let Some((mt, path)) = best {
            let (role, stop, ends_with_leaf, coordinating) =
                read_tail_signals(&path).unwrap_or_default();
            out.insert(
                aid,
                SessionSignal {
                    mtime_ms: mt,
                    role,
                    stop,
                    ends_with_leaf,
                    coordinating,
                },
            );
        }
    }
    out
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
    use super::*;
    use rusqlite::Connection;

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

    #[test]
    fn read_tail_reads_stopreason() {
        // 端到端:写一个临时 jsonl,确认 read_tail_signals 读到 message.stopReason
        // (字段是 message.stopReason 驼峰,非顶层 stop_reason —— 读错会永远空 → 工具链误判)。
        use std::io::Write;
        let p = std::env::temp_dir().join("asig_openclaw_tail_test.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"user","content":"hi"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"assistant","stopReason":"toolUse"}}}}"#
        )
        .unwrap();
        writeln!(f, r#"{{"type":"custom","customType":"model-snapshot"}}"#).unwrap();
        drop(f);
        let (role, stop, ends_with_leaf, coordinating) = read_tail_signals(&p).unwrap();
        assert_eq!(role, "assistant");
        assert_eq!(stop.as_deref(), Some("toolUse"));
        assert!(!ends_with_leaf, "末行是 custom,非 leaf");
        assert!(!coordinating, "无 yield/spawn");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn read_tail_detects_yield_leaf() {
        // 端到端:yield 中断态文件(sessions_spawn → sessions_yield → cache-ttl → assistant
        // stop="stop" → leaf),read_tail_signals 必须同时报 coordinating=true + ends_with_leaf=true,
        // 否则派发子 agent 期间会误判 Done。两种 yield 表达(custom_message + assistant toolCall)都测。
        use std::io::Write;
        let p = std::env::temp_dir().join("asig_openclaw_yield_test.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"assistant","content":[{{"type":"toolCall","name":"sessions_spawn"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"custom_message","message":{{"customType":"openclaw.sessions_yield"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"custom","customType":"openclaw.cache-ttl"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"assistant","stopReason":"stop"}}}}"#
        )
        .unwrap();
        writeln!(f, r#"{{"type":"leaf"}}"#).unwrap();
        drop(f);
        let (role, stop, ends_with_leaf, coordinating) = read_tail_signals(&p).unwrap();
        assert_eq!(role, "assistant");
        assert_eq!(stop.as_deref(), Some("stop"));
        assert!(ends_with_leaf, "应以 leaf 结尾");
        assert!(coordinating, "尾部应检出 sessions_yield/spawn");
        std::fs::remove_file(&p).ok();
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
