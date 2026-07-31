//! Hermes 状态出口:只读 `~/.hermes/state.db`(sqlite, WAL, gateway 持续写)+
//! `~/.hermes/gateway_state.json`。和 OpenClaw/Claude 同类 AgentSource。
//!
//! 数据源:
//! 1. **state.db** — `sessions`(cli/tui,未 archived)+ `messages` 尾部信号判在跑/等用户;
//!    `async_delegations`(state='failed')判 Error。
//! 2. **gateway 存活** — `gateway_state.json` 的 pid 配 `kill(pid,0)`;不活 / json 缺失 →
//!    整 source 空 `discover`(Asig 该 kind 自然 Offline,与 OpenClaw/Claude 一致)。
//!
//! 状态映射(学 OpenClaw「stop 即完成」,无 10min 窗口):
//!   - 尾部 `tool_calls` / `user` / `tool` → Working;
//!   - 尾部 `assistant + stop`(无 tool_calls,回复完成交还用户)+ `active_agents == 0`
//!     → **Done(立即)**;用户继续追问会写新 `user` 消息 → 自动转 Working;
//!   - `active_agents > 0` 时 stop 保守算 Working(本会话异步工具 / 别处仍忙);
//!   - `end_reason`/`handoff_error` 含 error,或有 failed `async_delegations` → Error;
//!   - hermes 是 gateway 架构(agent 自主执行工具,无「等用户授权」中间态)→ 不产 NeedsDeci;
//!   - 僵尸(最后消息 >`ACTIVE_WINDOW_MS` = 30min)不显示(用户关终端窗口不触发 cli_close,
//!     会话永远 OPEN,实证有多个 3 天~1 月前的 cli 僵尸)。

mod db;
mod gateway;

#[cfg(test)]
mod tests;

use crate::source::{AgentKind, AgentSession, AgentSource};
use crate::status::AgentStatus;
use rusqlite::Connection;
use std::path::PathBuf;

/// 会话活跃窗口:最后消息在此内才显示(滤 cli 僵尸会话)。
const ACTIVE_WINDOW_MS: u64 = 30 * 60 * 1000; // 30 min

pub struct HermesSource {
    root: PathBuf,
}

impl HermesSource {
    /// 生产构造:`~/.hermes` 不存在 → None(没装 hermes)。
    /// `ASIG_HERMES_ROOT`(dev):指向测试用 hermes 目录(构造小 `state.db` +
    /// `gateway_state.json` 做端到端状态验证);生产不设 → 默认 `~/.hermes`。
    pub fn new() -> Option<Self> {
        let root = match std::env::var_os("ASIG_HERMES_ROOT") {
            Some(r) => PathBuf::from(r),
            None => dirs::home_dir()?.join(".hermes"),
        };
        Some(Self { root })
    }

    fn db_path(&self) -> PathBuf {
        self.root.join("state.db")
    }

    fn gateway_state_path(&self) -> PathBuf {
        self.root.join("gateway_state.json")
    }

    /// 只读打开 state.db(WAL,不抢 gateway 写锁);失败 → None。
    fn connect(&self) -> Option<Connection> {
        crate::sys::open_readonly(&self.db_path())
    }
}

impl AgentSource for HermesSource {
    fn kind(&self) -> AgentKind {
        AgentKind::Hermes
    }

    fn discover(&self) -> Vec<AgentSession> {
        // gateway 必须活着——否则空(Asig 该 kind 自然 Offline)。先判存活再开 sqlite,
        // 省 WAL 锁竞争。
        let (alive, active_agents) = gateway::snapshot(&self.gateway_state_path());
        if !alive {
            return Vec::new();
        }
        let Some(conn) = self.connect() else {
            log::warn!("hermes state.db 打不开: {}", self.db_path().display());
            return Vec::new();
        };
        discover_from(&conn, crate::sys::now_ms(), active_agents)
    }
}

/// 纯函数核心(便于 in-memory sqlite 单测):接连接 + now ms + 全局 active_agents。
fn discover_from(conn: &Connection, now: u64, active_agents: u32) -> Vec<AgentSession> {
    if now == 0 {
        return Vec::new(); // 时钟未就绪,早返回防历史垃圾
    }
    let cutoff = now.saturating_sub(ACTIVE_WINDOW_MS);
    let rows = match db::active_sessions(conn) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("hermes 查询失败: {e}");
            return Vec::new();
        }
    };
    let failed = db::failed_delegation_sessions(conn, now).unwrap_or_default();
    rows.into_iter()
        .filter(|r| r.last_msg_at >= cutoff)
        .map(|r| {
            let error_flag = r.has_error() || failed.contains(&r.session_id);
            let status = classify_session(&r, active_agents, error_flag);
            AgentSession {
                kind: AgentKind::Hermes,
                id: format!("Hermes:{}", r.session_id),
                native_id: r.session_id.clone(),
                cwd: r.cwd.clone().map(PathBuf::from),
                status,
                label: Some(label_of(&r)),
                last_user_msg: empty_to_none(&r.last_user_content),
                last_assistant_msg: empty_to_none(&r.last_assistant_content),
            }
        })
        .collect()
}

/// 单会话状态判定(纯函数,优先级):
///   Error > Working(尾部 tool_calls/user/tool,或 active_agents>0) > Done(stop + 全局空闲)
/// hermes 无 NeedsDeci(gateway 架构 agent 自主执行工具,无「等用户决策」中间态)。
fn classify_session(r: &db::SessionRow, active_agents: u32, error_flag: bool) -> AgentStatus {
    if error_flag {
        return AgentStatus::Error;
    }
    if is_working_tail(&r.last_role, r.last_finish_reason.as_deref()) {
        return AgentStatus::Working;
    }
    // 尾部 assistant+stop(回复完成)。active_agents>0(别处仍忙)保守算 Working;否则立即
    // Done——学 OpenClaw「stop 即完成」:用户继续追问写新 user 消息自动转 Working,无需缓冲。
    if active_agents > 0 {
        return AgentStatus::Working;
    }
    AgentStatus::Done
}

/// 尾部"在跑"信号:`tool_calls`(模型发工具调用)/ `user`(刚输入)/ `tool`(结果待处理)。
/// 对照 claude.rs read_tail_signal 的语义。
fn is_working_tail(role: &str, finish: Option<&str>) -> bool {
    role == "user" || role == "tool" || finish == Some("tool_calls")
}

/// 空串 → None(Panel 事件 content 用;无内容则记事件时该字段为 None)。
fn empty_to_none(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// 标签降级链:display_name > title > cwd basename > session_id 前 8 字符。
fn label_of(r: &db::SessionRow) -> String {
    if let Some(d) = r.display_name.as_deref().filter(|s| !s.is_empty()) {
        return d.to_string();
    }
    if let Some(t) = r.title.as_deref().filter(|s| !s.is_empty()) {
        return t.to_string();
    }
    if let Some(c) = r.cwd.as_deref().filter(|s| !s.is_empty()) {
        if let Some(base) = std::path::Path::new(c).file_name().and_then(|n| n.to_str()) {
            return base.to_string();
        }
    }
    r.session_id.chars().take(8).collect()
}

// ---- CLI 探针(`probe-hermes`):复用 discover_from 的查询 + classify_session ----

/// 单会话诊断视图(probe 用)。
pub struct HermesProbe {
    pub session_id: String,
    pub label: String,
    pub status: AgentStatus,
    pub last_role: String,
    pub last_finish: Option<String>,
    /// 最后一条消息距 now 的秒数。
    pub last_msg_age_s: i64,
    pub active_agents: u32,
    pub error_flag: bool,
    pub cwd: Option<String>,
}

/// 探针:读真实 ~/.hermes,每会话输出诊断 + 最终 status(供 CLI `probe-hermes`)。
/// 复用 `discover_from` 的查询 + `classify_session`(单一判定源)。gateway 不活 → 空。
pub fn probe() -> Vec<HermesProbe> {
    let Some(src) = HermesSource::new() else {
        return Vec::new();
    };
    let (alive, active_agents) = gateway::snapshot(&src.gateway_state_path());
    if !alive {
        return Vec::new();
    }
    let Some(conn) = src.connect() else {
        return Vec::new();
    };
    let now = crate::sys::now_ms();
    if now == 0 {
        return Vec::new();
    }
    let cutoff = now.saturating_sub(ACTIVE_WINDOW_MS);
    let rows = match db::active_sessions(&conn) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("hermes probe 查询失败: {e}");
            return Vec::new();
        }
    };
    let failed = db::failed_delegation_sessions(&conn, now).unwrap_or_default();
    rows.into_iter()
        .filter(|r| r.last_msg_at >= cutoff)
        .map(|r| {
            let error_flag = r.has_error() || failed.contains(&r.session_id);
            let status = classify_session(&r, active_agents, error_flag);
            HermesProbe {
                last_msg_age_s: (now.saturating_sub(r.last_msg_at) / 1000) as i64,
                session_id: r.session_id.clone(),
                label: label_of(&r),
                status,
                last_role: r.last_role.clone(),
                last_finish: r.last_finish_reason.clone(),
                active_agents,
                error_flag,
                cwd: r.cwd.clone(),
            }
        })
        .collect()
}
