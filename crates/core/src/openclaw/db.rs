//! 主状态库查询:只读 openclaw.sqlite,按 agent 归并 task/flow/subagent runs 成 `AgentAcc`。
//! 连接由上层(`OpenClawSource::connect`)只读打开;WAL 允许 N 读 + 1 写,不抢 openclaw 写锁。

use super::sessions::{SessionSignal, session_running};
use rusqlite::{Connection, params};
use std::collections::HashMap;

/// "近期失败"窗口(ms):ended 落在此窗口内的 failed/lost/subagent-error 才报 Error。
/// 与 done_notif 同量级;过窗后 source 报 Done,sticky 状态机自动解锁 Error。
pub(super) const ERROR_RECENT_MS: u64 = 30_000;
/// `agent_databases` 的 `last_seen_at` 过滤窗口(ms):只盯近期见过的 agent,滤历史垃圾。
pub(super) const AGENT_RECENT_MS: u64 = 30 * 24 * 3600 * 1000; // 30 天
/// 交互式会话「近此窗口内活跃过」才看其尾部判在跑(防历史会话尾部 toolUse 永远 Working)。
/// 取 5 分钟:覆盖工具链内任意长间隙;turn 真完成后过窗即转 Done(完成通知略延后)。
const SESSION_RECENT_MS: u64 = 5 * 60 * 1000;
/// `sessions_yield` 协调态宽窗口:主 agent 用 `sessions_spawn` 派发的后台子 agent 走独立
/// trajectory、**不进 `subagent_runs` 表**,主 session 在 yield 等待期间尾部停在 `assistant
/// stop="stop"`+`leaf`(GLM 暂歇语义,非完成)。靠「文件以 `leaf` 结尾 ∧ 尾部含 yield/spawn」
/// 识别协调态 → Working。30 分钟覆盖 Deep 研究(announce 频繁刷新 mtime);过窗回落 Done 防异常卡死。
const SUBAGENT_WAIT_MS: u64 = 30 * 60 * 1000;

/// 单 agent 的状态累加器。
#[derive(Default, Clone, Copy)]
pub(super) struct AgentAcc {
    /// 最终判 Working 的「在跑」(普通 session 在跑)。
    pub(super) running: bool,
    /// run 表(task/flow/subagent)有 running —— 协调态区分「子 agent 在跑」vs「卡死」用;
    /// classify 也算 Working(有后台 run 在跑)。
    pub(super) run_active: bool,
    pub(super) blocked: bool,
    pub(super) recent_err: bool,
    /// 协调态卡死:主 agent sessions_yield 等子 agent,但子 agent 全 ended(B)或协调态超时(A)。
    pub(super) stuck: bool,
}

/// 收集每 agent 的累加器 + 最新会话信号(复用于 `discover_from` 与 `probe`;SQL/session 逻辑
/// 只此一处,杜绝 rs/sh 双实现漂移)。顺序同 `agent_databases` 返回顺序;无 run 的 agent 也含
/// (acc 默认),让面板与探针都能看到。
pub(super) fn collect(
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

/// 提取 agent_id:`agent:<id>:…` 前缀取首段;无前缀时本身可能是纯 agent_id
/// (subagent_runs 旧格式 requester="main",不含 `:`)→ 直接用。空或含其他结构 → None。
pub(super) fn agent_of(key: &str) -> Option<&str> {
    if let Some(rest) = key.strip_prefix("agent:") {
        let id = rest.split(':').next()?;
        return (!id.is_empty()).then_some(id);
    }
    // 无 agent: 前缀:仅当是纯 agent_id(非空、不含 `:`)才认,避免把长 session key 误归。
    (!key.is_empty() && !key.contains(':')).then_some(key)
}
