//! 设置窗口的 helper 函数:几何计算(card_height/card_frame/row_center_y)、tag 解析
//! (parse_control_tag/tab_of_key/stylekey_of_tab)、数值转换(hz_of/poll_preset_index/theme_index)、
//! SF Symbol 图标(sf_symbol)、label 列宽测量(label_col_width)。不可变常量见 consts.rs。

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSFont, NSImage, NSTextField};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use agent_light_core::{StyleKey, Theme};

use super::consts::{
    CARD_BOT_PAD, CARD_TOP_PAD, COL_W, COLOR_GAP, COLOR_ORDER, POLL_PRESETS_MS, ROW_H, STATE_KEYS,
    SWATCH_STEP, TAB_GENERAL,
};

/// `rows` 行卡片的总高度。
pub(crate) fn card_height(rows: usize) -> CGFloat {
    CARD_TOP_PAD + rows as CGFloat * ROW_H + CARD_BOT_PAD
}

/// 卡片 frame:顶部边在 `top`、`rows` 行高(含上下留白)。
pub(crate) fn card_frame(x0: CGFloat, top: CGFloat, rows: usize) -> NSRect {
    let h = card_height(rows);
    NSRect::new(NSPoint::new(x0, top - h), NSSize::new(COL_W, h))
}

/// 第 i 行(0=最上)的垂直中心 y。所有 label 与控件都对齐到它(居中制,杜绝错位)。
pub(crate) fn row_center_y(top: CGFloat, i: usize) -> CGFloat {
    top - CARD_TOP_PAD - (i as CGFloat + 0.5) * ROW_H
}

/// 色块 flow 在控件区宽 `cw` 下的(每行可容纳数, 总高)。固定间距 step = SWATCH_D + COLOR_GAP、
/// 左对齐 flow:每行首块 + 后续按 step 量出,放不下换行(每行数量可不同)。layout(实排)与
/// pane_state(预估 content_h)共用此单一事实源,避免两处分写同一几何而漂移。
pub(crate) fn color_flow_metrics(cw: CGFloat) -> (usize, CGFloat) {
    let per_row = (((cw + COLOR_GAP) / SWATCH_STEP).floor() as usize).max(1);
    let color_rows = COLOR_ORDER.len().div_ceil(per_row);
    (per_row, color_rows as CGFloat * SWATCH_STEP)
}

pub fn stylekey_of_tab(tab: i64) -> Option<StyleKey> {
    STATE_KEYS.iter().find(|(t, _)| *t == tab).map(|(_, k)| *k)
}

pub(crate) fn tab_of_key(key: StyleKey) -> i64 {
    STATE_KEYS
        .iter()
        .find(|(_, k)| *k == key)
        .map(|(t, _)| *t)
        .unwrap_or(TAB_GENERAL)
}

/// 控件 tag → (StyleKey, sub)。
pub fn parse_control_tag(tag: i64) -> Option<(StyleKey, i64)> {
    stylekey_of_tab(tag / 1000).map(|k| (k, tag % 1000))
}

pub(crate) fn hz_of(period_ms: u32) -> f64 {
    if period_ms == 0 {
        0.0
    } else {
        1000.0 / period_ms as f64
    }
}

pub(crate) fn poll_preset_index(ms: u32) -> usize {
    POLL_PRESETS_MS.iter().position(|&p| p == ms).unwrap_or(2)
}

/// Theme 下拉的选中索引(FollowSystem=0 / Dark=1 / Light=2)。
pub(crate) fn theme_index(theme: Theme) -> usize {
    match theme {
        Theme::FollowSystem => 0,
        Theme::Dark => 1,
        Theme::Light => 2,
    }
}

/// 单色 SF Symbol 图标(底栏用,template 渲染跟随明暗)。
pub(crate) fn sf_symbol(name: &str) -> Retained<NSImage> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)
        .expect("SF Symbol not found")
}

/// 测量一组 label 文字的最宽渲染宽,作为 label 列宽(系统字体 13,与 `add_text` 同字体;
/// +8 padding)。建临时 NSTextField → sizeToFit → 取 max → 丢弃(不进视图树)。与项目既有
/// sizeToFit 习惯(pane_general 标题 / theme chip)一致。
///
/// 用法:General/State pane 在 build 时测一次,确定 cx(控件 x)与 cw(控件区宽)。
/// **注意**:只传 label 文字,**排除**非 label 控件(如 reset 按钮),否则列宽偏大。
pub(crate) fn label_col_width(labels: &[&str]) -> CGFloat {
    let mtm = MainThreadMarker::new().expect("label_col_width 须主线程");
    const PADDING: CGFloat = 8.0;
    let mut max_w: CGFloat = 0.0;
    for &s in labels {
        // 临时字段(系统字体 13,与 add_text 默认一致),测完丢弃(超出作用域自动释放)。
        let field = NSTextField::new(mtm);
        field.setStringValue(&NSString::from_str(s));
        field.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        field.sizeToFit();
        let f = field.frame();
        max_w = max_w.max(f.size.width);
    }
    max_w + PADDING
}
