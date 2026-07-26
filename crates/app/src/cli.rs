//! CLI 子命令入口(供 main 的 argv 分支调用)。
//!
//! `probe-openclaw`:打印 openclaw 各 agent 的诊断信号 + 最终 status。判定统一走
//! `core::openclaw::probe`(单一事实源),替代 `scripts/probe-openclaw.sh` 的 bash 重新实现
//! —— 消除 CLAUDE.md 强制的 rs/sh 双实现同步负担,判定改动零漂移。

use agent_light_core::claude;
use agent_light_core::hermes;
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

/// 打印 Claude 每 cwd 组:每成员一行(组状态 + 成员诊断)。复用 core `claude::probe`。
pub fn probe_claude() {
    let groups = claude::probe();
    if groups.is_empty() {
        eprintln!("(无 Claude 会话或 ~/.claude 不存在)");
        return;
    }
    let total: usize = groups.iter().map(|g| g.members.len()).sum();
    eprintln!("claude: {} 组 / {} session 文件", groups.len(), total);
    for g in &groups {
        let cwd = g.cwd.as_deref().unwrap_or("(无 cwd)");
        for m in &g.members {
            let age = if m.age_s < 0 {
                "-".to_string()
            } else {
                format!("{}s", m.age_s)
            };
            let mark = if m.is_primary {
                "PRIMARY"
            } else if m.merged {
                "bg"
            } else {
                "skip"
            };
            let cls = m
                .classify
                .map(|c| format!("{c:?}"))
                .unwrap_or_else(|| "skip".into());
            println!(
                "{:<9} pid={:<7} kind={:<12} field={:<8} alive={} age={:<6} sig={:<9} cls={:<9} {:<7} cwd={cwd}",
                format!("{:?}", g.status),
                m.pid,
                m.kind.as_deref().unwrap_or("-"),
                m.status_field.as_deref().unwrap_or("-"),
                if m.alive { "Y" } else { "-" },
                age,
                m.signal.as_deref().unwrap_or("-"),
                cls,
                mark,
            );
        }
    }
}

/// 打印 Hermes 每 session:一行(status + sid/role/finish/age/active/err + label)。
pub fn probe_hermes() {
    let probes = hermes::probe();
    if probes.is_empty() {
        eprintln!("(无 Hermes 会话 / gateway 未运行 / ~/.hermes 不存在)");
        return;
    }
    eprintln!("hermes: {} 会话", probes.len());
    for p in &probes {
        let sid: String = p.session_id.chars().take(14).collect();
        println!(
            "{:<9} sid={:<14} role={:<10} finish={:<8} age={:<6} act={:<3} err={} {}",
            format!("{:?}", p.status),
            sid,
            p.last_role,
            p.last_finish.as_deref().unwrap_or("-"),
            format!("{}s", p.last_msg_age_s),
            p.active_agents,
            if p.error_flag { "Y" } else { "-" },
            p.label,
        );
    }
}
