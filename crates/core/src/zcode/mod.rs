//! Zcode 状态出口:只读 `~/.zcode/cli/db/db.sqlite`(zcode CLI/桌面版持续写,WAL)。
//! 和 Hermes/OpenClaw/Claude 同类 AgentSource,纯轮询聚合显示,不依赖 hooks。
//!
//! 数据源(session 表 + 每会话尾部 message/part):
//!   - `message.data`:`role`(user/assistant)、`time.completed`(流式回复写完才有)、
//!     `error`(模型流错误,如 stream stalled);
//!   - `part.data`(尾部):`step-finish.reason`(stop=回合结束 / tool-calls=继续跑)、
//!     `tool.state.status`(pending=待授权 / running=执行中 / completed / error)。
//!
//! 状态映射(优先级 Error > NeedsDeci > Working > Done):
//!   - 尾部 message 带 `error` → **Error**。但 `code=model_request_cancelled` 除外:那是
//!     用户主动取消(归档会话 / Esc 中断在途请求)写入的,非失败,按后续信号正常判
//!     (2026-09-04:归档会话闪红灯就是这么来的);
//!   - 尾部 tool part `state.status=pending` 且停留超过 `PENDING_CONFIRM_MS` →
//!     **NeedsDeci**。zcode 的 pending = ToolCallScheduled(已排队未开始):yolo 下转瞬即
//!     running(排队),非 yolo 下等用户授权也停在 pending —— 用停留时长区分两者,
//!     避免正常排队误报 🟠;
//!   - 尾部 `role=user`(真人输入 / 工具结果刚回)或 assistant 未写完(`completed` 缺失)
//!     或尾部 `step-finish.reason=tool-calls` → **Working**;
//!   - 尾部 assistant 已写完且 `reason=stop`(回复完成交还用户)→ **Done**(立即,
//!     学 hermes「stop 即完成」;用户继续追问写新 user 消息自动转 Working);
//!   - 僵尸(最后更新 >30min)与归档(`time_archived` 非空)不显示(会话永久留在 db,
//!     需窗口过滤,同 hermes;zcode 当前版本归档不写 time_archived,写了即生效)。
//!
//! 同 cwd 多会话聚合为一行(组内取最活跃状态),学 hermes cwd group。

mod db;

#[cfg(test)]
mod tests;

use crate::source::{AgentKind, AgentSession, AgentSource};
use crate::status::AgentStatus;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;

/// 会话活跃窗口:最后更新在此内才显示(会话永久留在 db,需窗口过滤;与 hermes 同款)。
pub(crate) const ACTIVE_WINDOW_MS: u64 = 30 * 60 * 1000; // 30 min

/// tool part 停在 `pending` 超过此时长才判 NeedsDeci(等授权)。yolo 下 pending 是排队
/// 瞬态(调度后毫秒级转 running);等用户授权则按人的反应时间停留。取 10s:默认 3s 轮询
/// 下既不闪误报,又不让真等待拖太久才亮 🟠。
pub(crate) const PENDING_CONFIRM_MS: u64 = 10_000;

pub struct ZcodeSource {
    root: PathBuf,
    db_path: PathBuf,
}

impl ZcodeSource {
    /// 生产构造:`~/.zcode/cli/db/db.sqlite` 不存在 → None(没装 zcode)。
    /// `ASIG_ZCODE_ROOT`(dev):指向测试用 `.zcode` 根目录(`<root>/cli/db/db.sqlite`);
    /// 生产不设 → 默认 `~/.zcode`。
    pub fn new() -> Option<Self> {
        let root = match std::env::var_os("ASIG_ZCODE_ROOT") {
            Some(r) => PathBuf::from(r),
            None => dirs::home_dir()?.join(".zcode"),
        };
        let db_path = root.join("cli").join("db").join("db.sqlite");
        if !db_path.is_file() {
            return None;
        }
        Some(Self { root, db_path })
    }
}

impl AgentSource for ZcodeSource {
    fn kind(&self) -> AgentKind {
        AgentKind::Zcode
    }

    fn discover(&self) -> Vec<AgentSession> {
        let Some(conn) = crate::sys::open_readonly(&self.db_path) else {
            log::warn!("zcode db 打不开: {}", self.db_path.display());
            return Vec::new();
        };
        discover_from(&conn, crate::sys::now_ms(), &self.root)
    }
}

/// 纯函数核心(便于 in-memory sqlite 单测):接连接 + now ms + zcode 根目录(判「无项目」)。
fn discover_from(conn: &Connection, now: u64, zcode_root: &std::path::Path) -> Vec<AgentSession> {
    let rows = match db::active_sessions(conn, now) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("zcode 查询失败: {e}");
            return Vec::new();
        }
    };

    // 按 cwd 分组(同路径多会话聚合为一行,学 hermes cwd group)。cwd 缺失的会话各成一组
    // (key 用 session_id,不合并)。每组取 last_msg_at 最新者为代表(label/cwd/content 用它),
    // 状态取组内最活跃(Error/NeedsDeci > Working > Done)。
    let mut groups: HashMap<String, Vec<db::SessionRow>> = HashMap::new();
    for r in rows {
        let key = r
            .cwd
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("__no_cwd__:{}", r.session_id));
        groups.entry(key).or_default().push(r);
    }

    let mut sessions: Vec<(u64, AgentSession)> = groups
        .into_values()
        .map(|mut group| {
            group.sort_by_key(|r| r.last_msg_at);
            let primary = group.last().expect("组非空");
            let status = group
                .iter()
                .map(|r| classify_session(r, now))
                .reduce(most_active)
                .unwrap_or(AgentStatus::Done);
            let last = primary.last_msg_at;
            (
                last,
                AgentSession {
                    kind: AgentKind::Zcode,
                    id: format!("Zcode:{}", primary.session_id),
                    native_id: primary.session_id.clone(),
                    cwd: primary.cwd.clone().map(PathBuf::from),
                    status,
                    label: Some(label_of(primary, zcode_root)),
                    last_user_msg: empty_to_none(&primary.last_user_content),
                    last_assistant_msg: empty_to_none(&primary.last_assistant_content),
                },
            )
        })
        .collect();
    // 组按最新活动时间 DESC(最新路径在前)
    sessions.sort_by_key(|x| std::cmp::Reverse(x.0));
    sessions.into_iter().map(|(_, s)| s).collect()
}

/// 组内状态聚合:取更活跃者(Error/NeedsDeci > Working > Done)。同 cwd 多会话时,
/// 任一在跑(Working)或出错(Error)都拉起整组,不被同路径的 Done 会话压下。
fn most_active(a: AgentStatus, b: AgentStatus) -> AgentStatus {
    fn rank(s: AgentStatus) -> u8 {
        match s {
            AgentStatus::Error | AgentStatus::NeedsDeci => 4,
            AgentStatus::Working => 3,
            AgentStatus::Offline => 2,
            AgentStatus::Done => 1,
        }
    }
    if rank(a) >= rank(b) { a } else { b }
}

/// 单会话状态判定(纯函数,优先级):Error > NeedsDeci(pending 停留超时)> Working > Done。
fn classify_session(r: &db::SessionRow, now: u64) -> AgentStatus {
    if r.last_error {
        return AgentStatus::Error;
    }
    if r.last_tool_status.as_deref() == Some("pending")
        && now.saturating_sub(r.last_part_updated_at) >= PENDING_CONFIRM_MS
    {
        return AgentStatus::NeedsDeci;
    }
    // 尾部 user(真人输入 / 工具结果刚回)或 assistant 未写完 → 在跑。
    if r.last_role == "user" || (r.last_role == "assistant" && !r.last_completed) {
        return AgentStatus::Working;
    }
    // assistant 已写完:step-finish=tool-calls(等工具结果继续)→ Working;stop → Done。
    if r.last_finish_reason.as_deref() == Some("tool-calls") {
        return AgentStatus::Working;
    }
    AgentStatus::Done
}

/// 空串 → None(Panel 事件 content 用;无内容则记事件时该字段为 None)。
fn empty_to_none(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// 标签(Panel 会话列表第二段)降级链,规则学 Claude Code 行「项目名优先」:
/// 1. **有项目**(cwd 存在且非 zcode 内部 workspace)→ cwd basename(如 `Asig`)。cwd 在
///    zcode 根内(未打开文件夹时的默认 workspace,如 `~/.zcode/workspace/default`)
///    不是真项目,basename 无意义(default),不算;
/// 2. **无项目** → zcode 自动生成的会话题名(如「深圳南山出行行李与穿着准备」)——zcode
///    自己的会话列表就用它,内容描述性强,不会与 agent 名重复;
/// 3. 兜底:session_id 前 8 字符(无题名时「default」这类 basename 无意义,不用)。
fn label_of(r: &db::SessionRow, zcode_root: &std::path::Path) -> String {
    if let Some(c) = r.cwd.as_deref().filter(|s| !s.is_empty()) {
        let p = std::path::Path::new(c);
        if !is_zcode_internal(p, zcode_root) {
            if let Some(base) = p.file_name().and_then(|n| n.to_str()) {
                if !base.is_empty() {
                    return base.to_string();
                }
            }
        }
    }
    if let Some(t) = r.title.as_deref().filter(|s| !s.is_empty()) {
        return t.to_string();
    }
    r.session_id.chars().take(8).collect()
}

/// cwd 是否 zcode 内部 workspace(= 无项目会话):在 zcode 根目录下,**或**路径含
/// `.zcode/workspace` 目录段(后者兜底:root 探测与真实安装位置不一致时——如 dev 的
/// `ASIG_ZCODE_ROOT` 回放库——仍按内容判定,不误显「default」)。
fn is_zcode_internal(p: &std::path::Path, zcode_root: &std::path::Path) -> bool {
    if p.starts_with(zcode_root) {
        return true;
    }
    let comps: Vec<_> = p.components().collect();
    comps
        .windows(2)
        .any(|w| w[0].as_os_str() == ".zcode" && w[1].as_os_str() == "workspace")
}

// ---- CLI 探针(`probe-zcode`):复用 discover_from 的查询 + classify_session ----

/// 单会话诊断视图(probe 用)。
pub struct ZcodeProbe {
    pub session_id: String,
    pub label: String,
    pub status: AgentStatus,
    pub last_role: String,
    pub completed: bool,
    pub error_flag: bool,
    pub part_type: Option<String>,
    pub tool_status: Option<String>,
    pub finish_reason: Option<String>,
    /// 最后一条消息距 now 的秒数。
    pub last_msg_age_s: i64,
    /// 尾部 tool pending 停留秒数(非 pending → None;probe 诊断用)。
    pub pending_age_s: Option<i64>,
    pub cwd: Option<String>,
}

/// 探针:读真实 `~/.zcode`,每会话输出诊断 + 最终 status(供 CLI `probe-zcode`)。
/// 复用 `discover_from` 的查询 + `classify_session`(单一判定源)。db 不存在 / 打不开 → 空。
pub fn probe() -> Vec<ZcodeProbe> {
    let Some(src) = ZcodeSource::new() else {
        return Vec::new();
    };
    let Some(conn) = crate::sys::open_readonly(&src.db_path) else {
        return Vec::new();
    };
    let now = crate::sys::now_ms();
    let rows = match db::active_sessions(&conn, now) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.into_iter()
        .map(|r| ZcodeProbe {
            last_msg_age_s: (now.saturating_sub(r.last_msg_at) / 1000) as i64,
            pending_age_s: if r.last_tool_status.as_deref() == Some("pending") {
                Some((now.saturating_sub(r.last_part_updated_at) / 1000) as i64)
            } else {
                None
            },
            status: classify_session(&r, now),
            label: label_of(&r, &src.root),
            session_id: r.session_id.clone(),
            last_role: r.last_role.clone(),
            completed: r.last_completed,
            error_flag: r.last_error,
            part_type: r.last_part_type.clone(),
            tool_status: r.last_tool_status.clone(),
            finish_reason: r.last_finish_reason.clone(),
            cwd: r.cwd.clone(),
        })
        .collect()
}
