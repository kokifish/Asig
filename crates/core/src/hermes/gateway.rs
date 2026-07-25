//! 解析 `~/.hermes/gateway_state.json` + pid 存活探测 → `(alive, active_agents)`。
//! json 缺失 / 损坏 / pid 死 → `(false, 0)`。

use serde::Deserialize;
use std::path::Path;

/// gateway_state.json 实测结构(节选):`{pid, gateway_state, active_agents, ...}`。
#[derive(Deserialize)]
struct GatewayState {
    pid: u32,
    #[serde(default)]
    active_agents: u32,
}

/// 读 json + 探 pid 存活。返回 `(alive, active_agents)`。
///
/// pid 探测为主(同 `sys::pid_alive`):gateway 崩溃时 json 不会自动改,残留的
/// `gateway_state: "running"` 不可信;`kill(pid, 0)` 是 OS 真相。
pub(crate) fn snapshot(path: &Path) -> (bool, u32) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (false, 0); // 文件缺失 = gateway 没开
    };
    let Ok(g): Result<GatewayState, _> = serde_json::from_str(&text) else {
        return (false, 0); // 损坏 = 当没活
    };
    if !crate::sys::pid_alive(g.pid) {
        return (false, 0);
    }
    (true, g.active_agents)
}
