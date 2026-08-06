//! CLI 探针(`probe-claude`):复用 discover 的聚合 + classify,输出诊断视图。

use super::{
    AgentStatus, ClaudeLikeSource, SessionFile, classify, group_status, last_signal,
    read_session_files,
};
use std::collections::HashMap;

/// 一个 session 文件的诊断视图(probe 用)。
pub struct ClaudeMember {
    pub pid: u32,
    pub kind: Option<String>,
    /// session 文件的 status:busy / idle / shell / waiting。
    pub status_field: Option<String>,
    /// status 最后更新距 now 的秒数;缺字段/未知 = -1。
    pub age_s: i64,
    pub alive: bool,
    pub session_id: Option<String>,
    /// transcript 尾部信号(busy 才读):end_turn / user / tool_use / None。
    pub signal: Option<String>,
    /// 单成员 classify(probe 无 seen → 死 pid 按「古老残留」跳过 = None,不报 Offline)。
    pub classify: Option<AgentStatus>,
    pub is_primary: bool,
    /// 是否参与组聚合(primary 或 bg);其他 interactive = false(方案 A:不合并)。
    pub merged: bool,
}

/// 同 cwd 一组的诊断视图。组状态 = `group_status`(primary + bg);其他 interactive 展示但不合并。
pub struct ClaudeProbe {
    pub cwd: Option<String>,
    pub primary_pid: u32,
    pub status: AgentStatus,
    pub members: Vec<ClaudeMember>,
}

/// 探针:读真实 ~/.claude,按 cwd 聚合,每组建 primary + 组状态 + 成员诊断(供 CLI
/// `probe-claude`)。复用 `discover_from` 的聚合选择 + `classify`/`group_status`(单一判定源)。
/// probe 无状态(seen=None)→ 死 pid 按「古老残留」跳过(classify=None),不报 Offline;
/// 诊断 Offline 需看运行中的 Asig(probe 不跟踪 seen 历史)。
pub fn probe() -> Vec<ClaudeProbe> {
    let Some(src) = ClaudeLikeSource::claude() else {
        return Vec::new();
    };
    let files = read_session_files(&src.root);
    // 按 cwd 聚合(同 discover_from),保持首次出现顺序。
    let mut groups: HashMap<Option<&str>, Vec<&SessionFile>> = HashMap::new();
    let mut order: Vec<Option<&str>> = Vec::new();
    for f in &files {
        let c = f.cwd.as_deref();
        if !groups.contains_key(&c) {
            order.push(c);
        }
        groups.entry(c).or_default().push(f);
    }
    let now_ms = crate::sys::now_ms();
    let mut out = Vec::new();
    for cwd in order {
        let group = &groups[&cwd];
        let Some(primary) = group
            .iter()
            .filter(|f| f.kind.as_deref() != Some("bg"))
            .max_by_key(|f| f.status_updated_at)
            .copied()
        else {
            continue;
        };
        let status = group_status(primary, group, None, &crate::sys::pid_alive, &|sid| {
            last_signal(&src.root, sid)
        })
        .map(|(st, _)| st)
        .unwrap_or(AgentStatus::Done);
        let members = group
            .iter()
            .map(|&f| {
                let alive = crate::sys::pid_alive(f.pid);
                let is_primary = f.pid == primary.pid;
                let merged = is_primary || f.kind.as_deref() == Some("bg");
                let tail = if alive && f.status.as_deref() == Some("busy") {
                    f.session_id
                        .as_deref()
                        .and_then(|sid| last_signal(&src.root, sid))
                } else {
                    None
                };
                let signal = tail.as_ref().map(|t| t.signal.clone());
                let age_s = if now_ms >= f.status_updated_at && f.status_updated_at > 0 {
                    ((now_ms - f.status_updated_at) / 1000) as i64
                } else {
                    -1
                };
                let classify_val = classify(f, None, alive, signal.as_deref());
                ClaudeMember {
                    pid: f.pid,
                    kind: f.kind.clone(),
                    status_field: f.status.clone(),
                    age_s,
                    alive,
                    session_id: f.session_id.clone(),
                    signal,
                    classify: classify_val,
                    is_primary,
                    merged,
                }
            })
            .collect();
        out.push(ClaudeProbe {
            cwd: cwd.map(String::from),
            primary_pid: primary.pid,
            status,
            members,
        });
    }
    out
}
