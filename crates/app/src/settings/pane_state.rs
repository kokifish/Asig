//! State pane(状态设置卡):标题 + Reset + 色块 / 动画 / 速度(+ DoneNotif 持续时间)。

use objc2::DefinedClass;
use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::sel;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSButton, NSSlider, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize};

use agent_light_core::{
    DONE_NOTIF_DURATION_MAX_S, DONE_NOTIF_DURATION_MIN_S, GRADIENT_LAYERS_MAX, GRADIENT_LAYERS_MIN,
    StyleKey,
};

use crate::app_delegate::AppDelegate;

use super::controls::{
    add_card, add_plain_button, add_radio_button, add_slider, add_swatch_button, add_text, new_view,
};
use super::layout::{StateControls, layout_state_pane, refresh_duration, refresh_state_controls};
use super::strings::Strings;
use super::tags::{
    ANIM_OFF, CARD_BOT_PAD, CARD_TOP_PAD, COL_W, COLOR_GAP, COLOR_OFF, COLOR_ORDER,
    CONTENT_HEADER_H, CONTENT_PAD_X, CONTENT_W, GRADIENT_LABEL_OFF, GRADIENT_OFF, H, HEADER_GAP,
    RESET_OFF, ROW_H, SPEED_LABEL_OFF, SPEED_MAX, SPEED_MIN, SPEED_OFF, SWATCH_D, TOP_INSET,
    label_col_width, tab_of_key,
};

/// State pane 一行「name 标签 + 滑块 + 右侧值标签」:三控件先占位零尺寸(frame 由 `layout_state_pane`
/// 后设),slider/value 打 tag(base + off)。收口 speed/gradient 两处「slider + set_tag + 文本」样板。
#[allow(clippy::too_many_arguments)]
fn add_state_slider(
    pane: &Retained<NSView>,
    delegate: &AppDelegate,
    base: i64,
    name: &str,
    slider_off: i64,
    label_off: i64,
    min: f64,
    max: f64,
    val: f64,
    action: Sel,
    value_text: &str,
) -> (
    Retained<NSSlider>,
    Retained<NSTextField>,
    Retained<NSTextField>,
) {
    let zero = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
    let name_lbl = add_text(pane, zero, name, false, false);
    let slider = add_slider(pane, zero, min, max, val, action, delegate);
    slider.setTag((base + slider_off) as isize);
    let value_lbl = add_text(pane, zero, value_text, false, false);
    value_lbl.setTag((base + label_off) as isize);
    (slider, value_lbl, name_lbl)
}

pub(crate) fn build_state_pane(
    delegate: &AppDelegate,
    key: StyleKey,
    name: &str,
    st: &Strings,
) -> (Retained<NSView>, StateControls, CGFloat) {
    let pane = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, H), // 临时高度,稍后按 content_h 重设
    ));
    let base = tab_of_key(key) * 1000;
    // label 列宽:测 state 的 5 个 label(颜色/效果/速度/渐变层数/持续时间;DoneNotif 持续时间
    // 仅其 pane 有,但列宽统一用同一组测量 —— 非该 pane 的 duration label 不存在,测量仍含它,
    // 保证所有 state pane 列宽一致,切 pane 时不会跳动)。
    let lw = label_col_width(&[st.color, st.animation, st.speed, st.gradient, st.duration]);
    // pane 实际内容高(动态):按默认宽度算 card_h,得 content_h = 顶/底留白 + header gap + card_h。
    // pane 高不随窗变(autoresizing=2 只宽);宽度变时 card 行数变 → card_h 变,但 pane 高固定,
    // 若 card 超出则出滚动条(符合每页独立滚动语义)。先算 content_h,header 据它定位。
    let col_w0 = CONTENT_W - CONTENT_PAD_X * 2.0;
    let cw0 = col_w0 - 16.0 - lw;
    let step0 = SWATCH_D + COLOR_GAP;
    let per_row0 = (((cw0 + COLOR_GAP) / step0).floor() as usize).max(1);
    let color_rows0 = COLOR_ORDER.len().div_ceil(per_row0);
    let color_h0 = color_rows0 as CGFloat * step0;
    let extra0 = if key == StyleKey::DoneNotif {
        ROW_H
    } else {
        0.0
    };
    let card_h0 = CARD_TOP_PAD + color_h0 + ROW_H * 3.0 + extra0 + CARD_BOT_PAD;
    let content_h = TOP_INSET + CONTENT_PAD_X + HEADER_GAP + card_h0 + CONTENT_PAD_X;
    let y_hdr = content_h - CONTENT_PAD_X - TOP_INSET;

    // header:标题(宽随 pane autoresizing)+ Reset(贴右 autoresizing)。
    let title = add_text(
        &pane,
        NSRect::new(
            NSPoint::new(CONTENT_PAD_X, y_hdr),
            NSSize::new(COL_W, CONTENT_HEADER_H),
        ),
        name,
        false,
        true,
    );
    let reset = add_plain_button(
        &pane,
        NSRect::new(
            NSPoint::new(CONTENT_W - CONTENT_PAD_X - 70.0, y_hdr + 1.0),
            NSSize::new(70.0, 24.0),
        ),
        st.reset,
        base + RESET_OFF,
        sel!(resetStateStyle:),
        delegate,
    );
    title.setAutoresizingMask(NSAutoresizingMaskOptions(2)); // width 随 pane
    reset.setAutoresizingMask(NSAutoresizingMaskOptions(1)); // 贴右(MinXMargin)

    // card + 控件:先占位创建(frame 由 layout_state_pane 按 pane 宽设)。
    let card = add_card(
        &pane,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
    );
    let color_lbl = add_text(
        &pane,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
        st.color,
        false,
        false,
    );
    let mut color_btns: Vec<Retained<NSButton>> = Vec::with_capacity(COLOR_ORDER.len());
    for (i, &color) in COLOR_ORDER.iter().enumerate() {
        color_btns.push(add_swatch_button(
            &pane,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(SWATCH_D, SWATCH_D)),
            color,
            base + COLOR_OFF + i as i64,
            delegate,
        ));
    }
    let anim_lbl = add_text(
        &pane,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
        st.animation,
        false,
        false,
    );
    let mut anim_btns: Vec<Retained<NSButton>> = Vec::with_capacity(3);
    for (i, &nm) in st.anim.iter().enumerate() {
        anim_btns.push(add_radio_button(
            &pane,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(72.0, 22.0)),
            nm,
            base + ANIM_OFF + i as i64,
            delegate,
            sel!(changeAnim:),
        ));
    }
    let (speed, speed_label, speed_lbl) = add_state_slider(
        &pane,
        delegate,
        base,
        st.speed,
        SPEED_OFF,
        SPEED_LABEL_OFF,
        SPEED_MIN,
        SPEED_MAX,
        1.0,
        sel!(changeSpeed:),
        "—",
    );

    // 渐变层数(整数拉杆 0..=4,仅作用于浮窗圆点本体)+ 标签 + 右侧 slider 值。
    let layers = delegate
        .ivars()
        .settings
        .borrow()
        .style_for(key)
        .gradient_layers;
    let (gradient, gradient_label, gradient_lbl) = add_state_slider(
        &pane,
        delegate,
        base,
        st.gradient,
        GRADIENT_OFF,
        GRADIENT_LABEL_OFF,
        GRADIENT_LAYERS_MIN as f64,
        GRADIENT_LAYERS_MAX as f64,
        layers as f64,
        sel!(changeGradient:),
        &format!("{}", layers),
    );
    // 整数滑块:5 刻度(0..=4)吸附,旋钮只停整数位(speed 是连续 Hz 不吸附)。
    gradient.setNumberOfTickMarks((GRADIENT_LAYERS_MAX - GRADIENT_LAYERS_MIN + 1) as isize);
    gradient.setAllowsTickMarkValuesOnly(true);

    // DoneNotif 专属:持续时间(秒)拉杆 + 标签 + 右侧 `xx s` 实时值。其余状态 None。
    let (duration, duration_lbl, duration_label) = if key == StyleKey::DoneNotif {
        let dlbl = add_text(
            &pane,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
            st.duration,
            false,
            false,
        );
        let secs = delegate.ivars().settings.borrow().done_notif_duration_s;
        let dur = add_slider(
            &pane,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
            DONE_NOTIF_DURATION_MIN_S as f64,
            DONE_NOTIF_DURATION_MAX_S as f64,
            secs as f64,
            sel!(changeDuration:),
            delegate,
        );
        let dval = add_text(
            &pane,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
            &format!("{} s", secs),
            false,
            false,
        );
        (Some(dur), Some(dlbl), Some(dval))
    } else {
        (None, None, None)
    };

    let controls = StateControls {
        key,
        lw,
        pane_h: content_h,
        card,
        color: color_btns,
        color_lbl,
        anim: anim_btns,
        anim_lbl,
        speed,
        speed_lbl,
        speed_label,
        gradient,
        gradient_lbl,
        gradient_label,
        duration,
        duration_lbl,
        duration_label,
    };
    pane.setFrameSize(NSSize::new(CONTENT_W, content_h));
    layout_state_pane(&controls, CONTENT_W); // 初始布局(默认宽度)
    let style = delegate.ivars().settings.borrow().style_for(key);
    refresh_state_controls(&controls, style);
    if key == StyleKey::DoneNotif {
        let secs = delegate.ivars().settings.borrow().done_notif_duration_s;
        refresh_duration(&controls, secs);
    }
    (pane, controls, content_h)
}
