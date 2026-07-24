//! 设置窗口控件工厂:卡片 / 文本 / 按钮 / 滑块 / 开关 / 下拉 / chip 等构造 helper。
//!
//! 这些函数被 pane_general / pane_state / pane_about / glass(mod.rs sidebar) 跨模块复用,
//! 故统一 pub(crate)。原文件内为私有,拆分后仅提升到 crate 内可见。

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{MainThreadMarker, Message, msg_send, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBezelStyle, NSBox, NSBoxType, NSButton, NSButtonType,
    NSCellImagePosition, NSColor, NSControlStateValueOff, NSControlStateValueOn, NSFont, NSImage,
    NSImageScaling, NSImageView, NSPopUpButton, NSSlider, NSSwitch, NSTextAlignment, NSTextField,
    NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use agent_light_core::Color;

use crate::app_delegate::AppDelegate;
use crate::overlay::swatch_image;

use super::consts::SWATCH_D;
use super::tags::sf_symbol;

/// 给控件配 target(=delegate)+ action。收口各 add_* 里重复的 setTarget/setAction
/// (setTarget:/setAction: 在所有 NSControl 子类都响应,用 msg_send! 绕过具体子类类型)。
fn wire_action<T: Message>(control: &Retained<T>, delegate: &AppDelegate, action: Sel) {
    unsafe {
        let _: () = msg_send![control, setTarget: delegate];
        let _: () = msg_send![control, setAction: action];
    }
}

/// 圆角填充 NSBox(setBoxType Custom + cornerRadius + 连续圆角 + wantsLayer)。
/// add_card 与 glass::make_selection_pill 共用(仅 cornerRadius / fillColor 不同)。
pub(crate) fn make_rounded_box(
    mtm: MainThreadMarker,
    corner_radius: CGFloat,
    fill: &NSColor,
) -> Retained<NSBox> {
    let b = NSBox::new(mtm);
    b.setBoxType(NSBoxType::Custom);
    b.setCornerRadius(corner_radius);
    b.setBorderWidth(0.0);
    b.setFillColor(fill);
    b.setTitle(&NSString::from_str(""));
    b.setWantsLayer(true);
    if let Some(layer) = b.layer() {
        layer.setCornerCurve(&NSString::from_str("continuous"));
    }
    b
}

/// 分组圆角卡片背景(NSBox custom:细边 + 圆角 + 浅填充),置于行后面。返回卡片引用(layout 重排用)。
pub(crate) fn add_card(pane: &Retained<NSView>, frame: NSRect) -> Retained<NSBox> {
    let mtm = MainThreadMarker::new().expect("add_card 须主线程");
    let b = make_rounded_box(mtm, 10.0, &NSColor::quaternaryLabelColor());
    b.setFrame(frame);
    b.setAutoresizingMask(NSAutoresizingMaskOptions(2)); // 宽度随 pane(state 卡片高度由 layout 重排覆盖)
    pane.addSubview(&b);
    b
}

pub(crate) fn new_view(frame: NSRect) -> Retained<NSView> {
    let mtm = MainThreadMarker::new().expect("new_view 须主线程");
    let v = NSView::new(mtm);
    v.setFrame(frame);
    v
}

/// 无边框按钮(Reset):标题 + action。
pub(crate) fn add_plain_button(
    pane: &Retained<NSView>,
    frame: NSRect,
    title: &str,
    tag: i64,
    action: Sel,
    delegate: &AppDelegate,
) -> Retained<NSButton> {
    let mtm = MainThreadMarker::new().expect("add_plain_button 须主线程");
    let btn = NSButton::new(mtm);
    btn.setBezelStyle(NSBezelStyle::Push);
    btn.setTitle(&NSString::from_str(title));
    btn.setTag(tag as isize);
    wire_action(&btn, delegate, action);
    btn.setFrame(frame);
    pane.addSubview(&btn);
    btn
}

/// 多选 chip:原生 NSButton + NSBezelStyle::AccessoryBar(=13,亦别名 Recessed;Apple 专为
/// scope/chip toggle 设计的 bezel,如 Mail 文件夹切换)+ NSButtonType::PushOnPushOff(点击 on↔off)。
/// 选中态由系统绘制(accentColor 浅底),单层 button —— 无 NSBox contentView 内缩导致的文字
/// 错位、无 layer.borderColor 的 CGColor 坑。宽按 sizeToFit 自适应。`action` 决定点击调用的
/// 选择子(General 多个多选行复用此 helper:Agent chip → changeEnabledAgents:,状态通知 chip →
/// changeNotifyOn:)。返回 button(tag 由调用方设)。
pub(crate) fn add_toggle_chip(
    pane: &Retained<NSView>,
    origin: NSPoint,
    title: &str,
    tag: i64,
    delegate: &AppDelegate,
    action: Sel,
) -> Retained<NSButton> {
    let mtm = MainThreadMarker::new().expect("add_toggle_chip 须主线程");
    let btn = NSButton::new(mtm);
    btn.setBezelStyle(NSBezelStyle::AccessoryBar);
    btn.setButtonType(NSButtonType::PushOnPushOff);
    btn.setBordered(true);
    btn.setTitle(&NSString::from_str(title));
    btn.setTag(tag as isize);
    wire_action(&btn, delegate, action);
    btn.sizeToFit();
    let fit = btn.frame();
    btn.setFrame(NSRect::new(origin, fit.size));
    pane.addSubview(&btn);
    btn
}

/// 侧栏 tab 按钮:无边框、左对齐;可选 image(状态色圆点)置于标题左侧。
pub(crate) fn add_tab_button(
    pane: &Retained<NSView>,
    frame: NSRect,
    title: &str,
    image: Option<&Retained<NSImage>>,
    tag: i64,
    delegate: &AppDelegate,
) -> Retained<NSButton> {
    let mtm = MainThreadMarker::new().expect("add_tab_button 须主线程");
    let btn = NSButton::new(mtm);
    btn.setBordered(false);
    btn.setAlignment(NSTextAlignment::Left);
    btn.setTitle(&NSString::from_str(title));
    if let Some(img) = image {
        btn.setImage(Some(img));
        btn.setImagePosition(NSCellImagePosition::ImageLeft);
    }
    btn.setTag(tag as isize);
    wire_action(&btn, delegate, sel!(switchSettingsTab:));
    btn.setFrame(frame);
    pane.addSubview(&btn);
    btn
}

/// 底栏图标按钮:单色 SF Symbol,无标题(image only)。
pub(crate) fn add_icon_button(
    pane: &Retained<NSView>,
    frame: NSRect,
    symbol: &str,
    tag: i64,
    delegate: &AppDelegate,
) -> Retained<NSButton> {
    let mtm = MainThreadMarker::new().expect("add_icon_button 须主线程");
    let btn = NSButton::new(mtm);
    let img = sf_symbol(symbol);
    btn.setBordered(false);
    btn.setTitle(&NSString::from_str("")); // 消掉默认 "Button"
    btn.setImage(Some(&img));
    btn.setImagePosition(NSCellImagePosition::ImageAbove);
    btn.setTag(tag as isize);
    wire_action(&btn, delegate, sel!(switchSettingsTab:));
    btn.setFrame(frame);
    pane.addSubview(&btn);
    btn
}

/// 色块单选按钮:无边框、无标题,图片=该色 swatch(选中带环)。
pub(crate) fn add_swatch_button(
    pane: &Retained<NSView>,
    frame: NSRect,
    color: Color,
    tag: i64,
    delegate: &AppDelegate,
) -> Retained<NSButton> {
    let mtm = MainThreadMarker::new().expect("add_swatch_button 须主线程");
    let btn = NSButton::new(mtm);
    let img = swatch_image(color, SWATCH_D, false);
    btn.setBordered(false);
    btn.setTitle(&NSString::from_str("")); // 消掉默认 "Button"
    btn.setImage(Some(&img));
    btn.setImagePosition(NSCellImagePosition::ImageAbove);
    btn.setTag(tag as isize);
    wire_action(&btn, delegate, sel!(changeColor:));
    btn.setFrame(frame);
    pane.addSubview(&btn);
    btn
}

/// 单选按钮(radio):标题 + action。
pub(crate) fn add_radio_button(
    pane: &Retained<NSView>,
    frame: NSRect,
    title: &str,
    tag: i64,
    delegate: &AppDelegate,
    action: Sel,
) -> Retained<NSButton> {
    let mtm = MainThreadMarker::new().expect("add_radio_button 须主线程");
    let btn = NSButton::new(mtm);
    btn.setButtonType(NSButtonType::Radio);
    btn.setTitle(&NSString::from_str(title));
    btn.setTag(tag as isize);
    wire_action(&btn, delegate, action);
    btn.setFrame(frame);
    pane.addSubview(&btn);
    btn
}

pub(crate) fn add_text(
    pane: &Retained<NSView>,
    frame: NSRect,
    text: &str,
    center: bool,
    bold: bool,
) -> Retained<NSTextField> {
    // 标准 NSTextField(而非 labelWithString:——后者不响应 setAlignment,实测右对齐不生效)。
    let mtm = MainThreadMarker::new().expect("add_text 须主线程");
    let label = NSTextField::new(mtm);
    label.setStringValue(&NSString::from_str(text));
    label.setBezeled(false);
    label.setDrawsBackground(false);
    label.setEditable(false);
    label.setSelectable(false);
    label.setTextColor(Some(&NSColor::labelColor()));
    if bold {
        label.setFont(Some(&NSFont::boldSystemFontOfSize(14.0)));
    }
    if center {
        // NSTextAlignmentCenter=2(此版本 bindings 未导出 Center 常量,保留裸值)。
        label.setAlignment(NSTextAlignment(2));
    }
    // 文字垂直居中 frame:NSTextField 默认基线偏上,与同行控件(slider/switch 中心)不齐
    // (macOS 官方最强调的 label-控件垂直居中)。sizeToFit 取文字自然高,再让文字中心 =
    // frame 中心。调用方若需自定义几何(如 header title 再 sizeToFit+setFrame)会覆盖此处。
    label.sizeToFit();
    let text_h = label.frame().size.height;
    let mid = frame.origin.y + frame.size.height / 2.0;
    label.setFrame(NSRect::new(
        NSPoint::new(frame.origin.x, mid - text_h / 2.0),
        NSSize::new(frame.size.width, text_h),
    ));
    pane.addSubview(&label);
    label
}

pub(crate) fn add_slider(
    pane: &Retained<NSView>,
    frame: NSRect,
    min: f64,
    max: f64,
    val: f64,
    action: Sel,
    delegate: &AppDelegate,
) -> Retained<NSSlider> {
    let mtm = MainThreadMarker::new().expect("add_slider 须主线程");
    let slider = NSSlider::new(mtm);
    slider.setFrame(frame);
    slider.setMinValue(min);
    slider.setMaxValue(max);
    slider.setDoubleValue(val);
    slider.setContinuous(true);
    wire_action(&slider, delegate, action);
    pane.addSubview(&slider);
    slider
}

/// NSSwitch(现代滑动开关,原生)。用于「点击穿透 / 开机启动」等开关行。
pub(crate) fn add_switch(
    pane: &Retained<NSView>,
    frame: NSRect,
    on: bool,
    action: Sel,
    delegate: &AppDelegate,
) -> Retained<NSSwitch> {
    let mtm = MainThreadMarker::new().expect("NSSwitch 须主线程");
    let sw = NSSwitch::new(mtm);
    sw.setState(if on {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
    wire_action(&sw, delegate, action);
    sw.setFrame(frame);
    pane.addSubview(&sw);
    sw
}

pub(crate) fn add_popup(
    pane: &Retained<NSView>,
    frame: NSRect,
    items: &[&str],
    selected: usize,
    action: Sel,
    delegate: &AppDelegate,
    tag: i64,
) -> Retained<NSPopUpButton> {
    let mtm = MainThreadMarker::new().expect("add_popup 须主线程");
    // NSPopUpButton::new 默认 pullsDown=NO(标准 pop-up,非 pull-down),与原 initWithFrame_pullsDown(false) 等价。
    let pop = NSPopUpButton::new(mtm);
    pop.setFrame(frame);
    for it in items {
        pop.addItemWithTitle(&NSString::from_str(it));
    }
    pop.selectItemAtIndex(selected as isize);
    pop.setTag(tag as isize);
    wire_action(&pop, delegate, action);
    pane.addSubview(&pop);
    pop
}

/// header 图标:NSImageView + 单色(template)SF Symbol,contentTintColor=labelColor,随明暗。
pub(crate) fn add_header_icon(pane: &Retained<NSView>, frame: NSRect, symbol: &str) {
    let mtm = MainThreadMarker::new().expect("add_header_icon 须主线程");
    let img = sf_symbol(symbol);
    img.setTemplate(true);
    let iv = NSImageView::new(mtm);
    iv.setFrame(frame);
    iv.setImage(Some(&img));
    iv.setImageScaling(NSImageScaling::ScaleProportionallyDown);
    iv.setContentTintColor(Some(&NSColor::labelColor()));
    pane.addSubview(&iv);
}
