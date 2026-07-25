//! Hermes 状态出口:只读 `~/.hermes/state.db`(sqlite, WAL, gateway 持续写)+
//! `~/.hermes/gateway_state.json`。和 OpenClaw/Claude 同类 AgentSource。
//!
//! 数据源:
//! 1. **state.db** — `sessions`(cli/tui,未 archived)+ `messages` 尾部信号判在跑/等用户;
//!    `async_delegations`(state='failed')判 Error。
//! 2. **gateway 存活** — `gateway_state.json` 的 pid 配 `kill(pid,0)`;不活 / json 缺失 →
//!    整 source 空 `discover`(Asig 该 kind 自然 Offline,与 OpenClaw/Claude 一致)。
//!
//! 状态映射:
//!   - 尾部 `tool_calls` / `user` / `tool` → Working;
//!   - 尾部 `assistant + stop`(无 tool_calls)+ `active_agents == 0` + 最后消息 >`IDLE_MS`(10min)
//!     → Done;同条件但 ≤`IDLE_MS` → NeedsDeci(刚问完用户,大概率还没回);
//!   - `active_agents > 0` 时 stop 不直接 Done,保守算 Working(本会话异步工具 / 别处仍忙);
//!   - `end_reason`/`handoff_error` 含 error,或有 failed `async_delegations` → Error;
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
/// Done vs NeedsDeci 分界:assistant + stop 后超此 → Done;否则 NeedsDeci(刚问完)。
const IDLE_MS: u64 = 10 * 60 * 1000; // 10 min

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
            let status = classify_session(&r, now, active_agents, error_flag);
            AgentSession {
                kind: AgentKind::Hermes,
                id: format!("Hermes:{}", r.session_id),
                native_id: r.session_id.clone(),
                cwd: r.cwd.clone().map(PathBuf::from),
                status,
                label: Some(label_of(&r)),
            }
        })
        .collect()
}

/// 单会话状态判定(纯函数,优先级):
///   Error > Working(尾部 tool_calls/user/tool,或 active_agents>0) > Done(stop+idle>10min)
///   > NeedsDeci(stop+idle≤10min)
fn classify_session(
    r: &db::SessionRow,
    now: u64,
    active_agents: u32,
    error_flag: bool,
) -> AgentStatus {
    if error_flag {
        return AgentStatus::Error;
    }
    if is_working_tail(&r.last_role, r.last_finish_reason.as_deref()) {
        return AgentStatus::Working;
    }
    // 尾部 assistant+stop(无 tool_calls):active_agents>0 保守算 Working;否则按空闲时长
    // 分 Done(等久了) / NeedsDeci(刚问完)。
    if active_agents > 0 {
        return AgentStatus::Working;
    }
    let age = now.saturating_sub(r.last_msg_at);
    if age > IDLE_MS {
        AgentStatus::Done
    } else {
        AgentStatus::NeedsDeci
    }
}

/// 尾部"在跑"信号:`tool_calls`(模型发工具调用)/ `user`(刚输入)/ `tool`(结果待处理)。
/// 对照 claude.rs read_tail_signal 的语义。
fn is_working_tail(role: &str, finish: Option<&str>) -> bool {
    role == "user" || role == "tool" || finish == Some("tool_calls")
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
