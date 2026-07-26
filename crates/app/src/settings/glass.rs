//! 液态玻璃 / vibrancy 后端封装 + 侧栏选中态药丸 + tab 选中切换。

use objc2::DefinedClass;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, NSObject};
use objc2::{MainThreadMarker, class, msg_send};
use objc2_app_kit::{
    NSColor, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
    NSVisualEffectView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSString};

use crate::app_delegate::AppDelegate;

use super::consts::{RESIZE_WH, STATE_KEYS, TAB_GENERAL};
use super::controls::new_view;
use super::strings::strings_for;

/// 运行时是否存在真·液态玻璃类(macOS 26+)。minos=11.0,旧系统无此类,须回退 vibrancy。
pub(crate) fn glass_available() -> bool {
    AnyClass::get(c"NSGlassEffectView").is_some()
}

/// 一块液态玻璃面板 + 它「承载 UI 的 content 视图」。两种后端、上层无感:UI 一律加到 `content`。
/// - macOS 26+:NSGlassEffectView,UI 必须放进其 contentView(Apple 文档明确要求;叠成兄弟视图
///   会被盖住 —— 这正是早先 NSGlassEffectView 失败的原因)。cornerRadius 决定玻璃形状圆角。
/// - 旧系统:NSVisualEffectView(`fallback_material`),UI 作子视图叠在 vibrancy 上(`content` 即其自身)。
pub(crate) struct GlassPane {
    pub(crate) view: Retained<NSView>,
    pub(crate) content: Retained<NSView>,
}

pub(crate) fn glass_pane(
    frame: NSRect,
    corner_radius: CGFloat,
    fallback_material: i64,
) -> GlassPane {
    let mtm = MainThreadMarker::new().expect("glass_pane 须主线程");
    // Reduce Transparency 开启时跳过 NSGlassEffectView,改走 NSVisualEffectView 分支
    // (它在 Reduce Transparency 下自动变不透明实色),保证文字可读。
    if glass_available() && !crate::overlay::reduce_transparency_on() {
        // NSGlassEffectView 不在 cargo feature 表里(macOS 26 新类),保留 msg_send! 构造 + setter。
        let g: Retained<NSView> = unsafe { msg_send![class!(NSGlassEffectView), new] };
        let content = new_view(NSRect::new(NSPoint::new(0.0, 0.0), frame.size));
        unsafe {
            let _: () = msg_send![&g, setFrame: frame];
            let _: () = msg_send![&g, setCornerRadius: corner_radius];
            let _: () = msg_send![&g, setContentView: Some(&*content)];
        }
        // contentView 宽+高 随玻璃视图缩放(承载的右区 content_area 据此自适应窗宽)。
        content.setAutoresizingMask(RESIZE_WH);
        GlassPane { view: g, content }
    } else {
        let v = NSVisualEffectView::new(mtm);
        v.setFrame(frame);
        v.setMaterial(NSVisualEffectMaterial(fallback_material as isize));
        v.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow); // 模糊窗口背后
        v.setState(NSVisualEffectState::Active);
        v.setWantsLayer(true);
        // content = vibrancy 自身(回退路径:UI 直接叠在 vibrancy 上)。
        let content = v.into_super();
        GlassPane {
            view: content.clone(),
            content,
        }
    }
}

/// 侧栏选中药丸 = 实心强调色圆角块(controlAccentColor)。玻璃/vibrancy 材质的选中态在已带玻璃
/// 的侧栏上会与背景融为一体、不可辨(实测 NSGlassEffectView tint / NSVisualEffectView Selection
/// 均不可见),故用实心强调色(同 stats.app 的 selectedContentBackgroundColor),在玻璃侧栏上清晰、
/// 读作「选中」。一个共享视图,选中时移到对应 tab 行(见 update_selection)。初始隐藏。
pub(crate) fn make_selection_pill() -> Retained<NSView> {
    let mtm = MainThreadMarker::new().expect("make_selection_pill 须主线程");
    // 实心强调色圆角块(复用 controls::make_rounded_box 的圆角样板,cornerRadius=8 + accent 填充)。
    let b = super::controls::make_rounded_box(mtm, 8.0, &NSColor::controlAccentColor());
    b.setHidden(true); // 初始隐藏,update_selection 时显示
    b.into_super()
}

/// 给 borderless tab 按钮设文字色:选中 = 白、否则 = labelColor。用 attributedTitle
/// 实现(borderless NSButton 默认标题色无法直接改)。状态色圆点图片保持彩色不变。
pub(crate) fn set_tab_title(button: &Retained<NSView>, label: &str, selected: bool) {
    let color = if selected {
        NSColor::whiteColor()
    } else {
        NSColor::labelColor()
    };
    // attributedTitle 用 NSDictionary + NSAttributedString 构造(改字色),保留 msg_send!(复杂、行为敏感)。
    unsafe {
        let attrs: Retained<NSObject> = msg_send![
            class!(NSDictionary),
            dictionaryWithObject: &*color,
            forKey: &*NSString::from_str("NSColor"), // NSForegroundColorAttributeName
        ];
        let astr: Allocated<NSObject> = msg_send![class!(NSAttributedString), alloc];
        let astr: Retained<NSObject> = msg_send![
            astr,
            initWithString: &*NSString::from_str(label),
            attributes: &*attrs,
        ];
        let _: () = msg_send![&**button, setAttributedTitle: &*astr];
    }
}

/// 切换选中 tab:把液态玻璃药丸移到选中项并显示,选中文字转白、其余 labelColor。
pub fn update_selection(delegate: &AppDelegate, selected: i64) {
    let Some(sidebar) = delegate
        .ivars()
        .settings_ui
        .borrow()
        .sidebar
        .as_ref()
        .cloned()
    else {
        return;
    };
    let st = strings_for(delegate.ivars().settings.borrow().lang);
    let mut labels: Vec<(i64, &str)> = vec![(TAB_GENERAL, st.general)];
    labels.extend(
        STATE_KEYS
            .iter()
            .zip(st.state.iter())
            .map(|((t, _), n)| (*t, *n)),
    );
    let pill = delegate
        .ivars()
        .settings_ui
        .borrow()
        .selection
        .as_ref()
        .cloned();
    let is_tab = labels.iter().any(|(t, _)| *t == selected);
    for (tag, label) in labels {
        let Some(b) = super::view_with_tag(&sidebar, tag) else {
            continue;
        };
        let is_sel = tag == selected;
        // 选中项:把药丸移到该按钮 frame 并显示。
        if is_sel {
            if let Some(p) = &pill {
                let frame = b.frame();
                p.setFrame(frame);
                p.setHidden(false);
            }
        }
        set_tab_title(&b, label, is_sel);
        // General tab 的齿轮(template)随选中转白;状态色点保持彩色不变。
        if tag == TAB_GENERAL {
            let tint = if is_sel {
                NSColor::whiteColor()
            } else {
                NSColor::labelColor()
            };
            // b 在此是 NSView(由 viewWithTag 返回),setContentTintColor 走 NSView 的方法。
            // 但 NSButton 的 setContentTintColor 需 NSButton;此处 b 是 NSView 引用,保留 msg_send!。
            unsafe {
                let _: () = msg_send![&b, setContentTintColor: &*tint];
            }
        }
    }
    // 选中的是非 tab 项(如「关于」= pane 7)时隐藏药丸 —— 不让某个 tab 仍读作选中。
    if !is_tab {
        if let Some(p) = &pill {
            p.setHidden(true);
        }
    }
}
