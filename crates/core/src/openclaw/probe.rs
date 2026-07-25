//! CLI 诊断探针:复用 `db::collect` + `sessions`,产出每 agent 的诊断 DTO + 最终 status。
//! 供 CLI `agent-light probe-openclaw` 用 —— 单一判定源,替代 scripts/probe-openclaw.sh 的
//! bash 重新实现(消除 rs/sh 双实现同步负担)。没装 openclaw / 打不开库 → 空。

use super::db::collect;
use super::sessions::latest_session_signals;
use super::{OpenClawSource, classify_agent};
use crate::status::AgentStatus;
use crate::sys::now_ms;

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
pub fn probe() -> Vec<AgentProbe> {
    let Some(src) = OpenClawSource::new() else {
        return Vec::new();
    };
    let Some(conn) = src.connect() else {
        return Vec::new();
    };
    let signals = latest_session_signals(src.root_path());
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
