//! 最小主菜单:仅切到 regular 激活策略(开设置窗)后由菜单栏显示。
//! App 菜单留空子菜单——macOS 自动补全 About / Hide / Quit(⌘Q,action=terminate:)等标准项;
//! File 菜单含 Close(⌘W → `performClose:`,走 responder chain 关当前 key window 即设置窗,
//! 进而触发 AppDelegate::windowWillClose: 切回 accessory)。文案按 `Settings.lang`。

use agent_light_core::Lang;
use objc2::MainThreadMarker;
use objc2::sel;
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

/// 幂等:首次调用建主菜单并 `setMainMenu`;已存在则 no-op(lang 切换后菜单标题不跟变,可接受)。
pub fn ensure_main_menu(lang: Lang) {
    let mtm = MainThreadMarker::new().expect("menu 须主线程");
    let app = NSApplication::sharedApplication(mtm);
    if app.mainMenu().is_some() {
        return;
    }
    let menubar = NSMenu::new(mtm);

    // App 菜单:子菜单留空,系统自动补 About/Hide/Quit ⌘Q(标题自动取 app 名)。
    let app_item = NSMenuItem::new(mtm);
    app_item.setSubmenu(Some(&NSMenu::new(mtm)));
    menubar.addItem(&app_item);

    // File 菜单:Close ⌘W(performClose: target=nil,走 responder chain)。
    let file_item = NSMenuItem::new(mtm);
    let file_menu = NSMenu::new(mtm);
    let (file_t, close_t) = match lang {
        Lang::Zh => ("文件", "关闭"),
        Lang::En => ("File", "Close"),
    };
    file_item.setTitle(&NSString::from_str(file_t));
    unsafe {
        let _ = file_menu.addItemWithTitle_action_keyEquivalent(
            &NSString::from_str(close_t),
            Some(sel!(performClose:)),
            &NSString::from_str("w"),
        );
        file_item.setSubmenu(Some(&file_menu));
    }
    menubar.addItem(&file_item);

    app.setMainMenu(Some(&menubar));
}
