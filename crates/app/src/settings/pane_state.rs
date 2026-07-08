//! State pane(状态设置卡):标题 + Reset + 色块 / 动画 / 速度(+ DoneNotif 持续时间)。

use objc2::DefinedClass;
use objc2::rc::Retained;
use objc2::sel;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSButton, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use agent_light_core::{DONE_NOTIF_DURATION_MAX_S, DONE_NOTIF_DURATION_MIN_S, StyleKey};

use crate::app_delegate::AppDelegate;

use super::controls::{
    add_card, add_plain_button, add_radio_button, add_slider, add_swatch_button, add_text,
    new_view, set_tag,
};
use super::layout::{StateControls, layout_state_pane, refresh_duration, refresh_state_controls};
use super::strings::Strings;
use super::tags::{
    ANIM_OFF, COL_W, COLOR_OFF, COLOR_ORDER, CONTENT_HEADER_H, CONTENT_PAD_X, CONTENT_W, H,
    RESET_OFF, SPEED_LABEL_OFF, SPEED_MAX, SPEED_MIN, SPEED_OFF, SWATCH_D, TOP_INSET, tab_of_key,
};

pub(crate) fn build_state_pane(
    delegate: &AppDelegate,
    key: StyleKey,
    name: &str,
    st: &Strings,
) -> (Retained<NSView>, StateControls) {
    let pane = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    let base = tab_of_key(key) * 1000;
    let y_hdr = H - CONTENT_PAD_X - TOP_INSET;

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
    let speed_lbl = add_text(
        &pane,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
        st.speed,
        false,
        false,
    );
    let speed = add_slider(
        &pane,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
        SPEED_MIN,
        SPEED_MAX,
        1.0,
        sel!(changeSpeed:),
        delegate,
    );
    set_tag(&speed, base + SPEED_OFF);
    let speed_label = add_text(
        &pane,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
        "—",
        false,
        false,
    );
    set_tag(&speed_label, base + SPEED_LABEL_OFF);

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
        card,
        color: color_btns,
        color_lbl,
        anim: anim_btns,
        anim_lbl,
        speed,
        speed_lbl,
        speed_label,
        duration,
        duration_lbl,
        duration_label,
    };
    layout_state_pane(&controls, CONTENT_W); // 初始布局(默认宽度)
    let style = delegate.ivars().settings.borrow().style_for(key);
    refresh_state_controls(&controls, style);
    if key == StyleKey::DoneNotif {
        let secs = delegate.ivars().settings.borrow().done_notif_duration_s;
        refresh_duration(&controls, secs);
    }
    (pane, controls)
}
