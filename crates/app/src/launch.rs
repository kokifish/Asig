//! 开机自启动(SMAppService,macOS 13+)。
//!
//! 零依赖:不引 objc2-service-management crate(未确认发布/够用),照 `glass.rs` 范式
//! 直接 link ServiceManagement framework + `msg_send!` 调 `SMAppService`。macOS < 13
//! 运行时检测类不存在 → `available()` 返回 false → 设置面板开关保持禁用(与 NSGlassEffectView
//! 回退 vibrancy 同一套类存在性检查模式)。
//!
//! **注册持久**:`SMAppService.register` 一次即被系统记住,重启自动登录 —— 故 app 启动时
//! 无需主动 register,只在用户切开关时 register/unregister 同步系统状态。

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};

// Link ServiceManagement framework(SMAppService 所在)。`msg_send![class!(SMAppService), ...]`
// 需要该 framework 已 link,否则运行时 objc_getClass 找不到类。
#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

/// SMAppService 是否可用(macOS 13+)。minos=11.0,旧系统返回 false → 开关禁用。
pub fn available() -> bool {
    AnyClass::get(c"SMAppService").is_some()
}

/// `[SMAppService mainApp]`。类不存在(<13)→ None。
fn main_app() -> Option<Retained<AnyObject>> {
    let cls = AnyClass::get(c"SMAppService")?;
    unsafe { msg_send![cls, mainApp] }
}

/// 注册开机自启。返回 register 是否成功(BOOL)。不可用(<13)→ false。
pub fn register() -> bool {
    let svc = match main_app() {
        Some(s) => s,
        None => return false,
    };
    let ok: bool = unsafe { msg_send![&svc, register] };
    ok
}

/// 注销开机自启。返回 unregister 是否成功。不可用 → false。
pub fn unregister() -> bool {
    let svc = match main_app() {
        Some(s) => s,
        None => return false,
    };
    let ok: bool = unsafe { msg_send![&svc, unregister] };
    ok
}
