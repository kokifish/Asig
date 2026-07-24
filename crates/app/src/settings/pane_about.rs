//! About pane(关于卡):应用名 + 版本号 + 仓库链接。

use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize};

use super::consts::{
    COL_W, CONTENT_HEADER_H, CONTENT_PAD_X, CONTENT_W, GITHUB_URL, HEADER_GAP, TOP_INSET,
};
use super::controls::{add_card, add_text, new_view};
use super::strings::Strings;
use super::tags::{card_frame, card_height, row_center_y};

pub(crate) fn build_about_pane(st: &Strings) -> (Retained<NSView>, CGFloat) {
    // pane 实际内容高:header + gap + 3 行卡片 + 顶/底留白。
    let content_h = TOP_INSET + CONTENT_PAD_X + HEADER_GAP + card_height(3) + CONTENT_PAD_X;
    let pane = new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, content_h),
    ));
    let x0 = CONTENT_PAD_X;
    let mut y = content_h - CONTENT_PAD_X - TOP_INSET;
    add_text(
        &pane,
        NSRect::new(NSPoint::new(x0, y), NSSize::new(COL_W, CONTENT_HEADER_H)),
        st.about,
        false,
        true,
    );
    y -= HEADER_GAP;
    add_card(&pane, card_frame(x0, y, 3));
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(x0 + 18.0, row_center_y(y, 0) - 10.0),
            NSSize::new(COL_W - 36.0, 20.0),
        ),
        "Asig",
        true,
        true,
    );
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(x0 + 18.0, row_center_y(y, 1) - 10.0),
            NSSize::new(COL_W - 36.0, 20.0),
        ),
        &format!("{}{}", st.version, env!("CARGO_PKG_VERSION")),
        true,
        false,
    );
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(x0 + 18.0, row_center_y(y, 2) - 10.0),
            NSSize::new(COL_W - 36.0, 20.0),
        ),
        GITHUB_URL,
        true,
        false,
    );
    (pane, content_h)
}
