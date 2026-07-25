//! 开机自启动(LaunchAgent plist —— 零成本,不依赖 SMAppService、不需签名)。
//!
//! SMAppService 要 Developer ID($99);未签名 app 调它抛 ObjC exception,foreign exception
//! 跨 objc2 msg_send FFI 边界("cannot unwind")→ abort。LaunchAgent 走 launchd,零成本可靠
//! (Tauri autostart / Electron auto-launch 同款)。toggle on → 写
//! ~/Library/LaunchAgents/com.kokifish.asig.plist(RunAtLoad=true),下次登录 launchd `open`
//! 启动 app;toggle off → 删 plist(当次会话不立即启,需重新登录才生效)。

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

/// 当前 app bundle 路径(NSBundle.mainBundle.bundlePath)。
fn app_bundle_path() -> Option<String> {
    // mainBundle / bundlePath 返回 nil 时 objc 向 nil 发消息仍返回 nil,故只查 path。
    let bundle: *mut NSObject = unsafe { msg_send![class!(NSBundle), mainBundle] };
    let path: *mut NSString = unsafe { msg_send![bundle, bundlePath] };
    (!path.is_null()).then(|| unsafe { (*path).to_string() })
}

/// 注册开机自启:写 LaunchAgent plist。
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
    let Some(p) = plist_path() else {
        return false;
    };
    let _ = fs::remove_file(&p);
    !p.exists()
}
