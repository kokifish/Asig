//! 设置窗口控件工厂:卡片 / 文本 / 按钮 / 滑块 / 开关 / 下拉 / chip 等构造 helper。
//!
//! 这些函数被 pane_general / pane_state / pane_about / glass(mod.rs sidebar) 跨模块复用,
//! 故统一 pub(crate)。原文件内为私有,拆分后仅提升到 crate 内可见。

use objc2::rc::{Allocated, Retained};
use objc2::runtime::Sel;
use objc2::{MainThreadMarker, class, msg_send, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBezelStyle, NSBox, NSBoxType, NSButton, NSButtonType,
    NSCellImagePosition, NSColor, NSControlStateValueOff, NSControlStateValueOn, NSFont, NSImage,
    NSImageScaling, NSImageView, NSPopUpButton, NSSlider, NSSwitch, NSTextAlignment, NSTextField,
    NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use agent_light_core::Color;

use crate::app_delegate::AppDelegate;
use crate::overlay::swatch_image;

use super::tags::{SWATCH_D, sf_symbol};

/// 分组圆角卡片背景(NSBox custom:细边 + 圆角 + 浅填充),置于行后面。返回卡片引用(layout 重排用)。
pub(crate) fn add_card(pane: &Retained<NSView>, frame: NSRect) -> Retained<NSBox> {
    let mtm = MainThreadMarker::new().expect("add_card 须主线程");
    let b = NSBox::new(mtm);
    b.setBoxType(NSBoxType::Custom);
    b.setCornerRadius(10.0);
    b.setBorderWidth(0.0);
    b.setFillColor(&NSColor::quaternaryLabelColor());
    b.setTitle(&NSString::from_str(""));
    b.setFrame(frame);
    b.setWantsLayer(true);
    if let Some(layer) = b.layer() {
        layer.setCornerCurve(&NSString::from_str("continuous"));
    }
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
    unsafe {
        btn.setTarget(Some(delegate));
        btn.setAction(Some(action));
    }
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
    unsafe {
        btn.setTarget(Some(delegate));
        btn.setAction(Some(action));
    }
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
    unsafe {
        btn.setTarget(Some(delegate));
        btn.setAction(Some(sel!(switchSettingsTab:)));
    }
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
    unsafe {
        btn.setTarget(Some(delegate));
        btn.setAction(Some(sel!(switchSettingsTab:)));
    }
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
    unsafe {
        btn.setTarget(Some(delegate));
        btn.setAction(Some(sel!(changeColor:)));
    }
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
    unsafe {
        btn.setTarget(Some(delegate));
        btn.setAction(Some(action));
    }
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
    // 用 alloc/initWithFrame 构造(而非 labelWithString:)—— 后者创建的 label 不响应
    // setAlignment(实测右对齐不生效),标准 NSTextField 才能可靠设对齐。
    let alloc: Allocated<NSTextField> = unsafe { msg_send![class!(NSTextField), alloc] };
    let label = NSTextField::initWithFrame(alloc, frame);
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
    let alloc: Allocated<NSSlider> = unsafe { msg_send![class!(NSSlider), alloc] };
    let slider = NSSlider::initWithFrame(alloc, frame);
    slider.setMinValue(min);
    slider.setMaxValue(max);
    slider.setDoubleValue(val);
    slider.setContinuous(true);
    unsafe {
        slider.setTarget(Some(delegate));
        slider.setAction(Some(action));
    }
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
    unsafe {
        sw.setTarget(Some(delegate));
        sw.setAction(Some(action));
    }
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
    let alloc: Allocated<NSPopUpButton> = unsafe { msg_send![class!(NSPopUpButton), alloc] };
    let pop = NSPopUpButton::initWithFrame_pullsDown(alloc, frame, false);
    for it in items {
        pop.addItemWithTitle(&NSString::from_str(it));
    }
    pop.selectItemAtIndex(selected as isize);
    pop.setTag(tag as isize);
    unsafe {
        pop.setTarget(Some(delegate));
        pop.setAction(Some(action));
    }
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
