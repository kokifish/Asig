//! CLI 子命令入口(供 main 的 argv 分支调用)。
//!
//! `probe-openclaw`:打印 openclaw 各 agent 的诊断信号 + 最终 status。判定统一走
//! `core::openclaw::probe`(单一事实源),替代 `scripts/probe-openclaw.sh` 的 bash 重新实现
//! —— 消除 CLAUDE.md 强制的 rs/sh 双实现同步负担,判定改动零漂移。

use agent_light_core::openclaw::{self, AgentProbe, OpenClawSource};

/// 打印 db 路径 + 每 agent 诊断行(一行一个,便于 watch / jq / CI 断言)。
pub fn probe_openclaw() {
    let db = OpenClawSource::new()
        .map(|s| s.db_path().display().to_string())
        .unwrap_or_else(|| "(home 未找到)".into());
    eprintln!("db: {db}");
    for p in openclaw::probe() {
        println!("{}", format_probe(&p));
    }
}

/// `<aid>  <status>  role=.. stop=.. yl=.. age=.. runs=.. blk=.. err=.. stuck=..`
/// yl=协调态(leaf∧yield);age 无会话=`-`;runs/blk/err/stuck 是 0/1。
fn format_probe(p: &AgentProbe) -> String {
    let stop = p.stop.as_deref().unwrap_or("-");
    let age = if p.age_s < 0 {
        "-".to_string()
    } else {
        format!("{}s", p.age_s)
    };
    format!(
        "{aid:<10} {status:<9} role={role:<10} stop={stop:<7} yl={yl} age={age:<6} runs={runs} blk={blk} err={err} stuck={stuck}",
        aid = p.aid,
        status = format!("{:?}", p.status),
        role = p.role,
        stop = stop,
        yl = if p.coordinating { "Y" } else { "-" },
        age = age,
        runs = p.run_active as u8,
        blk = p.blocked as u8,
        err = p.recent_err as u8,
        stuck = p.stuck as u8,
    )
}
