//! State pane 控件集合类型 + 按窗宽重排 + 按样式/Agent 状态刷新。

use objc2::msg_send;
use objc2::rc::Retained;
use objc2_app_kit::{NSBox, NSButton, NSSlider, NSTextField};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use agent_light_core::{Anim, StateStyle, StyleKey};

use crate::overlay::swatch_image;

use super::tags::{
    ANIM_ORDER, CARD_BOT_PAD, CARD_TOP_PAD, COLOR_GAP, COLOR_ORDER, CONTENT_PAD_X, H, HEADER_GAP,
    ROW_H, SPEED_MAX, SPEED_MIN, SWATCH_D, TOP_INSET, hz_of,
};

/// 一个状态 pane 的全部控件(类型化引用,便于 reset / 选择变更时批量刷新)。
pub struct StateControls {
    pub key: StyleKey,
    pub card: Retained<NSBox>,
    pub color: Vec<Retained<NSButton>>,
    pub color_lbl: Retained<NSTextField>,
    pub anim: Vec<Retained<NSButton>>,
    pub anim_lbl: Retained<NSTextField>,
    pub speed: Retained<NSSlider>,
    pub speed_lbl: Retained<NSTextField>,
    pub speed_label: Retained<NSTextField>,
    /// DoneNotif 专属:持续时间(秒)拉杆 + 标签 + 右侧 `xx s` 实时值。其余状态为 None。
    pub duration: Option<Retained<NSSlider>>,
    pub duration_lbl: Option<Retained<NSTextField>>,
    pub duration_label: Option<Retained<NSTextField>>,
}

/// 按 pane 宽度重排 state pane:card + 色块(固定间距 flow,行数随宽度)+ Anim/Speed/label。
/// build 与 windowDidResize 都调 —— 宽度变时色块自动换行 / 合并到 1 行,间距始终固定。
pub fn layout_state_pane(c: &StateControls, pane_w: CGFloat) {
    let col_w = pane_w - CONTENT_PAD_X * 2.0;
    let x0 = CONTENT_PAD_X;
    let lx = x0 + 16.0;
    let cx = x0 + 96.0;
    let cw = col_w - 96.0 - 16.0;
    let lw = cx - lx;
    let step = SWATCH_D + COLOR_GAP; // 色块固定间距(恒定,不随宽度变)
    // 每行可容纳数:首块 + 后续 (step) 量出;放不下就换行(每行数量可不同)。
    let per_row = (((cw + COLOR_GAP) / step).floor() as usize).max(1);
    let color_rows = COLOR_ORDER.len().div_ceil(per_row);
    let color_h = color_rows as CGFloat * step;
    let extra = if c.key == StyleKey::DoneNotif {
        ROW_H
    } else {
        0.0
    };
    let card_h = CARD_TOP_PAD + color_h + ROW_H * 2.0 + extra + CARD_BOT_PAD;
    let y_top = H - CONTENT_PAD_X - TOP_INSET - HEADER_GAP; // card 顶
    let color_top = y_top - CARD_TOP_PAD;
    let anim_top = color_top - color_h;
    let anim_mid = anim_top - ROW_H / 2.0;
    let speed_mid = anim_top - ROW_H - ROW_H / 2.0;
    c.card.setFrame(NSRect::new(
        NSPoint::new(x0, y_top - card_h),
        NSSize::new(col_w, card_h),
    ));
    c.color_lbl.setFrame(NSRect::new(
        NSPoint::new(lx, color_top - color_h / 2.0 - 10.0),
        NSSize::new(lw, 20.0),
    ));
    for (i, btn) in c.color.iter().enumerate() {
        let r = i / per_row;
        let cc = i % per_row;
        let sx = cx + cc as CGFloat * step;
        let row_mid = color_top - (r as CGFloat + 0.5) * step;
        btn.setFrame(NSRect::new(
            NSPoint::new(sx, row_mid - SWATCH_D / 2.0),
            NSSize::new(SWATCH_D, SWATCH_D),
        ));
    }
    c.anim_lbl.setFrame(NSRect::new(
        NSPoint::new(lx, anim_mid - 10.0),
        NSSize::new(lw, 20.0),
    ));
    for (i, btn) in c.anim.iter().enumerate() {
        btn.setFrame(NSRect::new(
            NSPoint::new(cx + i as CGFloat * 76.0, anim_mid - 11.0),
            NSSize::new(72.0, 22.0),
        ));
    }
    c.speed_lbl.setFrame(NSRect::new(
        NSPoint::new(lx, speed_mid - 10.0),
        NSSize::new(lw, 20.0),
    ));
    c.speed.setFrame(NSRect::new(
        NSPoint::new(cx, speed_mid - 11.0),
        NSSize::new(cw - 64.0, 22.0),
    ));
    c.speed_label.setFrame(NSRect::new(
        NSPoint::new(cx + cw - 56.0, speed_mid - 10.0),
        NSSize::new(56.0, 20.0),
    ));
    // DoneNotif:持续时间行(speed 下一行)。
    if let (Some(slider), Some(lbl), Some(vlabel)) =
        (&c.duration, &c.duration_lbl, &c.duration_label)
    {
        let dur_mid = anim_top - ROW_H * 2.0 - ROW_H / 2.0;
        lbl.setFrame(NSRect::new(
            NSPoint::new(lx, dur_mid - 10.0),
            NSSize::new(lw, 20.0),
        ));
        slider.setFrame(NSRect::new(
            NSPoint::new(cx, dur_mid - 11.0),
            NSSize::new(cw - 64.0, 22.0),
        ));
        vlabel.setFrame(NSRect::new(
            NSPoint::new(cx + cw - 56.0, dur_mid - 10.0),
            NSSize::new(56.0, 20.0),
        ));
    }
}

/// 按某状态当前样式,刷新其 pane 的色块(选中带环)/ radio 选中 / 速度滑块+标签。
pub fn refresh_state_controls(c: &StateControls, style: StateStyle) {
    let steady = style.anim == Anim::Steady;
    for (i, btn) in c.color.iter().enumerate() {
        let img = swatch_image(COLOR_ORDER[i], SWATCH_D, style.color == COLOR_ORDER[i]);
        btn.setImage(Some(&img));
    }
    for (i, btn) in c.anim.iter().enumerate() {
        let on = style.anim == ANIM_ORDER[i];
        // setState 取 NSControlStateValue enum;用裸值 1/0 表 on/off,保留 msg_send!。
        unsafe {
            let _: () = msg_send![btn, setState: if on { 1i64 } else { 0 }];
        }
    }
    let hz = if steady {
        1.0
    } else {
        hz_of(style.period_ms).clamp(SPEED_MIN, SPEED_MAX)
    };
    let text = if steady {
        "—".to_string()
    } else {
        format!("{:.1} Hz", hz)
    };
    c.speed.setEnabled(!steady);
    c.speed.setDoubleValue(hz);
    c.speed_label.setStringValue(&NSString::from_str(&text));
}

/// 刷新 DoneNotif 持续时间拉杆的值 + 右侧 `xx s` 标签(其余状态无 duration 控件,空操作)。
pub fn refresh_duration(c: &StateControls, secs: u32) {
    if let (Some(slider), Some(label)) = (&c.duration, &c.duration_label) {
        slider.setDoubleValue(secs as f64);
        label.setStringValue(&NSString::from_str(&format!("{} s", secs)));
    }
}
