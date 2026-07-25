//! 开机自启动(LaunchAgent plist —— 零成本:不依赖 SMAppService、不需签名)。
//!
//! 为何不用 SMAppService:它要求 proper code signing(Developer ID,$99);未签名 / ad-hoc
//! 签名的 app 调 `SMAppService.mainApp` 会抛 ObjC exception,foreign exception 跨 objc2
//! msg_send 的 FFI 边界(“cannot unwind”)→ abort。LaunchAgent 走 launchd,不需签名,是
//! 零成本 login item 的标准方案(Tauri autostart / Electron auto-launch 同款)。
//!
//! toggle on → 写 `~/Library/LaunchAgents/com.kokifish.asig.plist`(RunAtLoad=true),
//! 下次登录 launchd 自动 `open` 启动 app bundle;toggle off → 删 plist。
//! 注意:当次会话不立即启动,需重新登录/重启才生效(launchd 在登录时读 plist)。

use std::fs;
use std::path::PathBuf;

use objc2::runtime::NSObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;

const LABEL: &str = "com.kokifish.asig";

fn plist_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")),
    )
}

/// 当前 app bundle 路径(`NSBundle.mainBundle.bundlePath`)。决定 LaunchAgent 启动哪个 app。
fn app_bundle_path() -> Option<String> {
    let bundle: *mut NSObject = unsafe { msg_send![class!(NSBundle), mainBundle] };
    if bundle.is_null() {
        return None;
    }
    let path: *mut NSString = unsafe { msg_send![bundle, bundlePath] };
    if path.is_null() {
        return None;
    }
    Some(unsafe { (*path).to_string() })
}

/// LaunchAgent 总可用(不需签名、不依赖系统版本,只要 home 目录可写)。
pub fn available() -> bool {
    plist_path().is_some()
}

/// 注册开机自启:写 LaunchAgent plist。返回是否写入成功。
pub fn register() -> bool {
    let (Some(plist), Some(app)) = (plist_path(), app_bundle_path()) else {
        return false;
    };
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/open</string>
    <string>{app}</string>
  </array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#,
    );
    let _ = fs::create_dir_all(plist.parent().unwrap_or(std::path::Path::new(".")));
    fs::write(plist, xml).is_ok()
}

/// 注销开机自启:删 LaunchAgent plist。
pub fn unregister() -> bool {
    match plist_path() {
        Some(p) => {
            let _ = fs::remove_file(&p);
            !p.exists()
        }
        None => false,
    }
}
