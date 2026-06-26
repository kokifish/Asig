//! Drop-down Panel:单击菜单栏 Signal Icon 后弹出。含会话列表 + 顶部三按钮
//! (设置 / 锁定 / 退出)。位置见 DEV.md「Drop-down Panel」:图标左下方、左对齐图标左边,
//! 右侧空间不足则右贴屏幕边缘;不可拖动、不可改大小。

use agent_light_core::Snapshot;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, NSObject};
use objc2::{class, declare_class, msg_send, msg_send_id, mutability, sel, ClassType, DeclaredClass};
use objc2_app_kit::{
    NSApplication, NSBezierPath, NSButton, NSColor, NSFont, NSScreen, NSStatusBarButton, NSStatusItem,
    NSTextField, NSView, NSWindow,
};
use objc2_foundation::{CGFloat, NSPoint, NSRect, NSSize, NSString};

use crate::app_delegate::AppDelegate;
use crate::palette::status_emoji;

pub const PANEL_W: CGFloat = 280.0;
pub const PANEL_H: CGFloat = 220.0;

// 无边框 popover 用的窗口子类:默认 borderless 窗口 canBecomeKeyWindow 返回 NO,
// 里面的按钮就点不动。覆盖成 YES。
declare_class!(
    pub struct KeyPanel;

    unsafe impl ClassType for KeyPanel {
        type Super = NSWindow;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "KeyPanel";
    }

    impl DeclaredClass for KeyPanel {
        type Ivars = ();
    }

    #[allow(non_snake_case)]
    unsafe impl KeyPanel {
        #[method(canBecomeKeyWindow)]
        fn can_become_key(&self) -> Bool {
            Bool::YES
        }
        #[method(canBecomeMainWindow)]
        fn can_become_main(&self) -> Bool {
            Bool::NO
        }
    }
);

// 圆角卡片背景:自绘 NSBezierPath 圆角矩形填充 windowBackgroundColor(适配深/浅色),
// 绕开 CALayer CGColor 依赖。窗口设透明底,由它画出圆角 + 阴影跟随圆角 → OneDrive 风格。
declare_class!(
    pub struct CardView;

    unsafe impl ClassType for CardView {
        type Super = NSView;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "CardView";
    }

    impl DeclaredClass for CardView {
        type Ivars = ();
    }

    #[allow(non_snake_case)]
    unsafe impl CardView {
        #[method(drawRect:)]
        fn draw_rect(&self, _dirty: NSRect) {
            let bounds: NSRect = unsafe { msg_send![self, bounds] };
            let r: CGFloat = 12.0;
            let path: Retained<NSBezierPath> = unsafe {
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, r, r)
            };
            let bg: Retained<NSColor> = unsafe { msg_send_id![class!(NSColor), windowBackgroundColor] };
            let _: () = unsafe { msg_send![&bg, set] };
            unsafe { path.fill() };
        }
    }
);

impl CardView {
    fn new(frame: NSRect) -> Retained<Self> {
        let alloc: Allocated<Self> = unsafe { msg_send_id![Self::class(), alloc] };
        unsafe { msg_send_id![alloc, initWithFrame: frame] }
    }
}

pub struct Popover {
    pub window: Retained<KeyPanel>,
    label: Retained<NSTextField>,
}

/// 计算 Drop-down 左下角屏幕坐标(纯函数,便于单测)。
/// - x:与图标左边对齐;右侧溢出 `vis_max_x` 则右贴边;再不够则夹到 `vis_min_x`。
/// - y:图标底边正下方(`icon_min_y - panel_h`)。
fn dropdown_origin(
    icon_min_x: CGFloat,
    icon_min_y: CGFloat,
    panel_w: CGFloat,
    panel_h: CGFloat,
    vis_min_x: CGFloat,
    vis_max_x: CGFloat,
) -> NSPoint {
    let mut x = icon_min_x;
    if x + panel_w > vis_max_x {
        x = vis_max_x - panel_w;
    }
    if x < vis_min_x {
        x = vis_min_x;
    }
    NSPoint::new(x, icon_min_y - panel_h)
}

/// 读菜单栏 Signal Icon 的屏幕坐标(button bounds → window → screen)。
fn icon_screen_frame(item: &NSStatusItem) -> Option<NSRect> {
    unsafe {
        let button: Retained<NSStatusBarButton> = msg_send_id![item, button];
        let bounds: NSRect = msg_send![&button, bounds];
        let in_win: NSRect =
            msg_send![&button, convertRect: bounds, toView: std::ptr::null_mut::<NSView>()];
        let win: Retained<NSWindow> = msg_send_id![&button, window];
        let in_screen: NSRect = msg_send![&win, convertRectToScreen: in_win];
        Some(in_screen)
    }
}

/// 给定 Signal Icon + 面板尺寸,算出面板应出现的屏幕坐标(图标左下方)。读不到图标则兜底右上。
pub fn dropdown_position_for(item: &NSStatusItem, panel_w: CGFloat, panel_h: CGFloat) -> NSPoint {
    let screen: Retained<NSScreen> = unsafe { msg_send_id![class!(NSScreen), mainScreen] };
    let vis: NSRect = unsafe { msg_send![&screen, visibleFrame] };
    let vis_min_x = vis.origin.x;
    let vis_max_x = vis.origin.x + vis.size.width;
    match icon_screen_frame(item) {
        Some(icon) => dropdown_origin(icon.origin.x, icon.origin.y, panel_w, panel_h, vis_min_x, vis_max_x),
        None => NSPoint::new(vis_max_x - panel_w - 12.0, vis.origin.y + vis.size.height - panel_h - 8.0),
    }
}

/// 构建面板(每次显示都新建,定位在 `pos`;隐藏即丢弃 → 不占常驻内存,且每次拿最新位置/锁定态)。
pub fn build(delegate: &AppDelegate, pos: Option<NSPoint>) -> Popover {
    let fallback = {
        let screen: Retained<NSScreen> = unsafe { msg_send_id![class!(NSScreen), mainScreen] };
        let vis: NSRect = unsafe { msg_send![&screen, visibleFrame] };
        NSPoint::new(
            vis.origin.x + vis.size.width - PANEL_W - 12.0,
            vis.origin.y + vis.size.height - PANEL_H - 8.0,
        )
    };
    let origin = pos.unwrap_or(fallback);
    let frame = NSRect::new(origin, NSSize::new(PANEL_W, PANEL_H));

    let alloc: Allocated<KeyPanel> = unsafe { msg_send_id![KeyPanel::class(), alloc] };
    let window: Retained<KeyPanel> = unsafe {
        msg_send_id![
            alloc,
            initWithContentRect: frame,
            styleMask: 0u64, // borderless:不可拖动、不可改大小
            backing: 2u64,
            defer: Bool::NO,
        ]
    };
    unsafe {
        let clear = NSColor::clearColor();
        let _: () = msg_send![&window, setOpaque: Bool::NO];
        let _: () = msg_send![&window, setBackgroundColor: &*clear]; // 透明底,由 CardView 画圆角
        let _: () = msg_send![&window, setHasShadow: Bool::YES]; // 阴影跟随圆角内容
        let _: () = msg_send![&window, setLevel: 3i64]; // floating
        let _: () = msg_send![&window, setHidesOnDeactivate: Bool::YES]; // 点别处(失焦)自动关
        // 测试时用 ASIG_NO_HIDE 临时关掉,便于截图(见 main.rs 钩子说明)
        if std::env::var("ASIG_NO_HIDE").is_ok() {
            let _: () = msg_send![&window, setHidesOnDeactivate: Bool::NO];
        }
        let _: () = msg_send![&window, setReleasedWhenClosed: Bool::NO];
    }
    // 圆角卡片作为内容视图;后续子视图加到它上面。
    let card: Retained<CardView> = CardView::new(frame);
    unsafe {
        let _: () = msg_send![&window, setContentView: &*card];
    }
    let content: Retained<NSView> = unsafe { msg_send_id![&window, contentView] };
    let locked = *delegate.ivars().click_through.borrow(); // 锁定 = 不可拖动 = click_through

    // 标题
    add_label(
        &content,
        NSRect::new(NSPoint::new(16.0, PANEL_H - 28.0), NSSize::new(PANEL_W - 32.0, 18.0)),
        "Asig",
        true,
    );

    // —— 顶部三按钮(左→右):设置 / 锁定 / 退出 ——
    let btn_settings: Retained<NSButton> = unsafe { msg_send_id![class!(NSButton), new] };
    unsafe {
        let _: () = msg_send![&btn_settings, setFrame: NSRect::new(NSPoint::new(16.0, PANEL_H - 64.0), NSSize::new(84.0, 30.0))];
        let _: () = msg_send![&btn_settings, setTitle: &*NSString::from_str("设置")];
        let _: () = msg_send![&btn_settings, setTarget: delegate];
        let _: () = msg_send![&btn_settings, setAction: sel!(openSettings:)];
        let _: () = msg_send![&content, addSubview: &*btn_settings];
    }

    // 锁定:勾选=Signal Light 不可拖动(click_through)。复用 toggleClickThrough:。
    let btn_lock: Retained<NSButton> = unsafe { msg_send_id![class!(NSButton), new] };
    unsafe {
        let _: () = msg_send![&btn_lock, setButtonType: 3u64]; // NSSwitchButton = 圆角勾选
        let _: () = msg_send![&btn_lock, setTitle: &*NSString::from_str("锁定")];
        let _: () = msg_send![&btn_lock, setState: if locked { 1i64 } else { 0 }];
        let _: () = msg_send![&btn_lock, setTarget: delegate];
        let _: () = msg_send![&btn_lock, setAction: sel!(toggleClickThrough:)];
        let _: () = msg_send![&btn_lock, setFrame: NSRect::new(NSPoint::new(110.0, PANEL_H - 64.0), NSSize::new(74.0, 30.0))];
        let _: () = msg_send![&content, addSubview: &*btn_lock];
    }

    let btn_quit: Retained<NSButton> = unsafe { msg_send_id![class!(NSButton), new] };
    unsafe {
        let _: () = msg_send![&btn_quit, setFrame: NSRect::new(NSPoint::new(PANEL_W - 16.0 - 84.0, PANEL_H - 64.0), NSSize::new(84.0, 30.0))];
        let _: () = msg_send![&btn_quit, setTitle: &*NSString::from_str("退出")];
        let _: () = msg_send![&btn_quit, setTarget: delegate];
        let _: () = msg_send![&btn_quit, setAction: sel!(quit:)];
        let _: () = msg_send![&content, addSubview: &*btn_quit];
    }

    // 会话列表
    let label: Retained<NSTextField> = unsafe {
        msg_send_id![class!(NSTextField), labelWithString: &*NSString::from_str("(无会话)")]
    };
    unsafe {
        let _: () = msg_send![&label, setFrame: NSRect::new(NSPoint::new(16.0, 16.0), NSSize::new(PANEL_W - 32.0, PANEL_H - 96.0))];
        let font: Retained<NSFont> = msg_send_id![class!(NSFont), systemFontOfSize: 12.0];
        let _: () = msg_send![&label, setFont: &*font];
        let _: () = msg_send![&content, addSubview: &*label];
    }

    Popover { window, label }
}

fn add_label(content: &Retained<NSView>, frame: NSRect, text: &str, bold: bool) {
    let label: Retained<NSTextField> =
        unsafe { msg_send_id![class!(NSTextField), labelWithString: &*NSString::from_str(text)] };
    unsafe {
        if bold {
            let font: Retained<NSFont> = msg_send_id![class!(NSFont), boldSystemFontOfSize: 14.0];
            let _: () = msg_send![&label, setFont: &*font];
        }
        let _: () = msg_send![&label, setFrame: frame];
        let _: () = msg_send![&**content, addSubview: &*label];
    }
}

pub fn is_visible(p: &Popover) -> bool {
    unsafe { msg_send![&p.window, isVisible] }
}

pub fn show(p: &Popover) {
    unsafe {
        // 状态栏点击不会激活 app;不激活的话浮动窗会因 hidesOnDeactivate 立刻消失。
        let app: Retained<NSApplication> = msg_send_id![class!(NSApplication), sharedApplication];
        let _: () = msg_send![&app, activateIgnoringOtherApps: Bool::YES];
        let _: () = msg_send![&p.window, makeKeyAndOrderFront: std::ptr::null_mut::<NSObject>()];
    }
}

pub fn hide(p: &Popover) {
    let _: () = unsafe { msg_send![&p.window, orderOut: std::ptr::null_mut::<NSObject>()] };
}

/// 用最新快照刷新会话列表。
pub fn update_label(p: &Popover, snap: &Snapshot) {
    let text = if snap.sessions.is_empty() {
        "(无活跃会话)".to_string()
    } else {
        snap.sessions
            .iter()
            .map(|s| {
                format!("{} {:?} · {}", status_emoji(s.status), s.kind, s.project.as_deref().unwrap_or("-"))
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    unsafe {
        let _: () = msg_send![&p.label, setStringValue: &*NSString::from_str(&text)];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_left_aligned_below_icon() {
        // 图标在屏幕左侧,空间充足 → 左对齐、下方
        let p = dropdown_origin(100.0, 1400.0, 280.0, 220.0, 0.0, 2560.0);
        assert_eq!(p.x, 100.0); // 左对齐图标
        assert_eq!(p.y, 1400.0 - 220.0); // 图标底下方
    }

    #[test]
    fn origin_right_clamp_when_overflow() {
        // 图标靠右,左对齐会溢出 → 右贴屏幕边
        let p = dropdown_origin(2400.0, 1400.0, 280.0, 220.0, 0.0, 2560.0);
        assert_eq!(p.x, 2560.0 - 280.0); // 右贴边
    }

    #[test]
    fn origin_left_floor_when_still_overflow() {
        // 即使右贴边仍超出左边界(面板比屏宽)→ 夹到 vis_min_x
        let p = dropdown_origin(2400.0, 1400.0, 3000.0, 220.0, 0.0, 2560.0);
        assert_eq!(p.x, 0.0);
    }
}
