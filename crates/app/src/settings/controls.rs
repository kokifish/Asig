//! 设置窗口控件工厂:卡片 / 文本 / 按钮 / 滑块 / 开关 / 下拉 / chip 等构造 helper。
//!
//! 这些函数被 pane_general / pane_state / pane_about / glass(mod.rs sidebar) 跨模块复用,
//! 故统一 pub(crate)。原文件内为私有,拆分后仅提升到 crate 内可见。

use objc2::rc::{Allocated, Retained};
use objc2::runtime::Sel;
use objc2::{MainThreadMarker, Message, class, msg_send, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBox, NSButton, NSColor, NSFont, NSImage, NSPopUpButton, NSSlider,
    NSSwitch, NSTextField, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use agent_light_core::Color;

use crate::app_delegate::AppDelegate;
use crate::overlay::swatch_image;

use super::tags::{SWATCH_D, sf_symbol};

/// 分组圆角卡片背景(NSBox custom:细边 + 圆角 + 浅填充),置于行后面。返回卡片引用(layout 重排用)。
pub(crate) fn add_card(pane: &Retained<NSView>, frame: NSRect) -> Retained<NSBox> {
    let mtm = MainThreadMarker::new().expect("add_card 须主线程");
    let b = NSBox::new(mtm);
    unsafe {
        let _: () = msg_send![&b, setBoxType: 4u64]; // NSBoxCustom(常量 feature 不确定,保留裸值)
    }
    b.setCornerRadius(10.0);
    b.setBorderWidth(0.0);
    b.setFillColor(&NSColor::quaternaryLabelColor());
    b.setTitle(&NSString::from_str(""));
    b.setFrame(frame);
    b.setWantsLayer(true);
    // CALayer.setCornerCurve: 常量在 feature 后,保留 msg_send。
    if let Some(layer) = b.layer() {
        unsafe {
            let _: () = msg_send![&layer, setCornerCurve: &*NSString::from_str("continuous")];
        }
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

pub(crate) fn set_tag<T: Message>(view: &Retained<T>, tag: i64) {
    unsafe {
        let _: () = msg_send![view, setTag: tag];
    }
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
    unsafe {
        let _: () = msg_send![&btn, setBezelStyle: 1u64]; // NSBezelStyleRounded(enum 常量在 feature 后,保留裸值)
    }
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

/// Agent 多选 chip:NSBox 容器(custom:圆角+边框,样式由 apply_chip_style 设)+ 内嵌 borderless
/// NSButton(文字 + action)。调用方给 chip 左下角 origin;宽按文字 sizeToFit + 左右 padding 自适应。
/// button 填满 chip、文字水平居中(NSButton 默认垂直居中)——避免手动按框中心居中导致的墨迹
/// 错位(NSButton 墨迹低于框中心,同 General 标题的坑)。返回 button(tag = AGENT_OFF + i)。
pub(crate) fn add_agent_chip(
    pane: &Retained<NSView>,
    origin: NSPoint,
    title: &str,
    tag: i64,
    delegate: &AppDelegate,
) -> Retained<NSButton> {
    let mtm = MainThreadMarker::new().expect("add_agent_chip 须主线程");
    let chip = NSBox::new(mtm);
    unsafe {
        let _: () = msg_send![&chip, setBoxType: 4u64]; // NSBoxCustom(常量 feature 不确定,保留裸值)
    }
    chip.setCornerRadius(8.0);
    chip.setBorderWidth(1.5);
    chip.setTitle(&NSString::from_str(""));
    pane.addSubview(&chip);
    let btn = NSButton::new(mtm);
    btn.setBordered(false);
    btn.setTitle(&NSString::from_str(title));
    btn.setTag(tag as isize);
    unsafe {
        btn.setTarget(Some(delegate));
        btn.setAction(Some(sel!(changeEnabledAgents:)));
    }
    btn.sizeToFit();
    let fit = btn.frame();
    const CHIP_PAD: CGFloat = 10.0;
    const CHIP_H: CGFloat = 22.0;
    let chip_w = fit.size.width + 2.0 * CHIP_PAD;
    chip.setFrame(NSRect::new(origin, NSSize::new(chip_w, CHIP_H)));
    // button 填满 chip(button 加到 NSBox.contentView,frame = chip bounds),文字由 alignment 居中。
    btn.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(chip_w, CHIP_H),
    ));
    unsafe {
        let _: () = msg_send![&btn, setAlignment: 2i64]; // NSTextAlignmentCenter(enum 常量在 feature 后)
    }
    chip.addSubview(&btn);
    btn
}

/// 取 chip 的 NSBox 容器。button 加到 NSBox.contentView,故 chip = button.superview.superview。
/// flow 布局与样式刷新都通过它定位/设样,避免每处重写 superview 链。
pub(crate) fn chip_of_button(button: &Retained<impl Message>) -> Retained<NSView> {
    let cv: Retained<NSView> = unsafe { msg_send![button, superview] };
    unsafe { msg_send![&cv, superview] }
}

/// 按 selected 设 agent chip(= button 的 superview 链上的 NSBox)样式:选中=强调色边框+浅强调底,
/// 未选=分隔线色细边框+透明底。用 NSBox 的 setBorderColor:/setFillColor:(NSColor 直传)——绕开
/// layer.borderColor 要 CGColor(NSDynamicSystemColor/_NSTaggedPointerColor 等多类 NSColor 不响应
/// cgColor,直取会运行时崩 NSInvalidArgumentException)。button 经 NSBox.contentView 承载,故
/// chip = button.superview.superview。
pub(crate) fn apply_chip_style(button: &Retained<impl Message>, selected: bool) {
    // chip = button.superview.superview(NSBox);NSBox 专属 setter 走 msg_send!(动态分发到 NSBox)。
    let chip = chip_of_button(button);
    let border = if selected {
        NSColor::controlAccentColor()
    } else {
        NSColor::separatorColor()
    };
    let fill = if selected {
        NSColor::controlAccentColor().colorWithAlphaComponent(0.15)
    } else {
        NSColor::clearColor()
    };
    unsafe {
        let _: () = msg_send![&chip, setBorderColor: &*border];
        let _: () = msg_send![&chip, setFillColor: &*fill];
    }
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
    unsafe {
        let _: () = msg_send![&btn, setAlignment: 0i64]; // NSTextAlignmentLeft(enum 常量在 feature 后,保留裸值)
    }
    btn.setTitle(&NSString::from_str(title));
    if let Some(img) = image {
        btn.setImage(Some(img));
        unsafe {
            let _: () = msg_send![&btn, setImagePosition: 2i64]; // image left(NSCellImagePosition 常量在 feature 后,保留裸值)
        }
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
    unsafe {
        let _: () = msg_send![&btn, setImagePosition: 5i64]; // image only(NSCellImagePosition 常量在 feature 后,保留裸值)
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
    unsafe {
        let _: () = msg_send![&btn, setImagePosition: 5i64]; // image only(NSCellImagePosition 常量在 feature 后,保留裸值)
    }
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
    unsafe {
        let _: () = msg_send![&btn, setButtonType: 4u64]; // NSButtonTypeRadio(enum 常量在 feature 后,保留裸值)
    }
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
        unsafe {
            let _: () = msg_send![&label, setAlignment: 2i64]; // NSTextAlignmentCenter(enum 常量在 feature 后,保留裸值)
        }
    }
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
    unsafe {
        // setState 取 NSControlStateValue enum;用裸值 1/0 表 on/off,保留 msg_send!。
        let _: () = msg_send![&sw, setState: if on { 1i64 } else { 0 }];
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
    let img = sf_symbol(symbol);
    img.setTemplate(true);
    // NSImageView 的 cargo feature 未开(只裸 msg_send! 用 class! 透传),保留 msg_send! 构造,
    // 以免为单一图标控件引入 feature。其余 setter 同理走 msg_send!。
    unsafe {
        let iv: Retained<NSView> = msg_send![class!(NSImageView), new];
        let _: () = msg_send![&iv, setFrame: frame];
        let _: () = msg_send![&iv, setImage: &*img];
        let _: () = msg_send![&iv, setImageScaling: 0i64]; // scaleProportionallyDown
        let _: () = msg_send![&iv, setContentTintColor: &*NSColor::labelColor()];
        let _: () = msg_send![&**pane, addSubview: &*iv];
    }
}
