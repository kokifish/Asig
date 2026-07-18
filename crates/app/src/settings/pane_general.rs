//! General pane(常规设置卡):语言 / 灯大小 / 点击穿透 / 轮询 / 开机启动 / Agent chip / Theme / Reset。
//! `build_general_pane` 只编排 header → Group-1 → Group-2 三段(各成子函数)+ 高度收尾。

use objc2::DefinedClass;
use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::sel;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSControlStateValueOff, NSControlStateValueOn, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize};

use agent_light_core::{DOT_SIZE_MAX_PX, DOT_SIZE_MIN_PX, Lang};

use crate::app_delegate::AppDelegate;

use super::consts::{
    AGENT_KIND_ORDER, AGENT_OFF, CARD_GAP, COL_W, CONTENT_HEADER_H, CONTENT_PAD_X, CONTENT_W, H,
    HEADER_GAP, LANG_EN_TAG, LANG_ZH_TAG, NOTIFY_OFF, NOTIFY_STATUS_ORDER, SIZE_LABEL_TAG,
    THEME_OFF, TOP_INSET,
};
use super::controls::{
    add_card, add_header_icon, add_plain_button, add_popup, add_radio_button, add_slider,
    add_switch, add_text, add_toggle_chip,
};
use super::strings::Strings;
use super::tags::{
    card_frame, card_height, label_col_width, poll_preset_index, row_center_y, theme_index,
};

/// NSSwitch frame 比 alignmentRect 宽(左侧 inset),origin 左移此值让 switch 与其他控件左对齐。
const SWITCH_INSET: CGFloat = 5.0;
/// chip 之间的水平间距(flow 布局)。
const CHIP_GAP: CGFloat = 10.0;
/// chip 换行之间的垂直间距。
const CHIP_VGAP: CGFloat = 6.0;
/// theme 三个 radio 之间的固定水平间距(紧凑成组,不填满控件区)。
const THEME_GAP: CGFloat = 20.0;

/// pane 几何:卡片 x / 标签 x / 控件 x / 控件区宽 / 标签列宽(由 label_col_width 测,三段共用)。
struct Geom {
    x0: CGFloat,
    lx: CGFloat,
    cx: CGFloat,
    cw: CGFloat,
    lw: CGFloat,
}

/// 多选 chip flow:sizeToFit 自适应宽 + 固定间距,超出控件区右边界换行。返回换行行数。
/// agent chip(T=AgentKind)与 notify chip(T=AgentStatus)共用,消除两段近乎一致的重复。
#[allow(clippy::too_many_arguments)]
fn flow_chips<T: PartialEq>(
    pane: &Retained<NSView>,
    delegate: &AppDelegate,
    items: &[&str],
    tag_off: i64,
    action: Sel,
    order: &[T],
    enabled: &[T],
    cx: CGFloat,
    chip_max_x: CGFloat,
    row0_center: CGFloat,
    chip_gap: CGFloat,
    chip_vgap: CGFloat,
) -> usize {
    let mut x = cx;
    let mut row_count: usize = 0;
    let mut h: CGFloat = 22.0;
    for (i, &name) in items.iter().enumerate() {
        // 先以临时 origin 建好 chip 拿 sizeToFit 尺寸,再按 flow 定位 + 设初始选中态。
        let btn = add_toggle_chip(
            pane,
            NSPoint::new(0.0, 0.0),
            name,
            tag_off + i as i64,
            delegate,
            action,
        );
        let bf = btn.frame();
        h = h.max(bf.size.height);
        let on = enabled.contains(&order[i]);
        btn.setState(if on {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        let bw = bf.size.width;
        if x + bw > chip_max_x && x > cx {
            row_count += 1;
            x = cx;
        }
        let row_center = row0_center - row_count as CGFloat * (h + chip_vgap);
        btn.setFrameOrigin(NSPoint::new(x, row_center - bf.size.height / 2.0));
        x += bw + chip_gap;
    }
    row_count
}

pub(crate) fn build_general_pane(
    delegate: &AppDelegate,
    st: &Strings,
) -> (Retained<NSView>, CGFloat) {
    // label 列宽:测 9 个 label(系统字体 13,排除 reset 按钮)取最宽 + padding。
    let labels: [&str; 9] = [
        st.language,
        st.reset_all,
        st.light_size,
        st.click_through,
        st.poll_interval,
        st.agent_monitor,
        st.notify,
        st.launch_login,
        st.theme,
    ];
    let lw = label_col_width(&labels);
    let pane = super::controls::new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    let x0 = CONTENT_PAD_X;
    let g = Geom {
        x0,
        lx: x0 + 16.0,
        cx: x0 + 16.0 + lw,
        cw: COL_W - 16.0 - lw - 16.0,
        lw,
    };
    let mut y = H - CONTENT_PAD_X - TOP_INSET;

    build_header(&pane, st, delegate, &g, y);
    y -= HEADER_GAP;
    build_group1(&pane, st, delegate, &g, y);
    y -= card_height(2) + CARD_GAP;
    let g2_rows = build_group2(&pane, st, delegate, &g, y);

    // pane 实际内容高:Group-2 底 = y − card_height(g2_rows),加底部留白得 content_h。
    let content_h = (H - y) + card_height(g2_rows) + CONTENT_PAD_X;
    // 内容按 H 布置但 pane 实际高 content_h 可能 ≠ H;整体平移 dy = content_h − H 让上下留白对称。
    let dy = content_h - H;
    for sv in pane.subviews().iter() {
        let f = sv.frame();
        sv.setFrameOrigin(NSPoint::new(f.origin.x, f.origin.y + dy));
    }
    pane.setFrameSize(NSSize::new(CONTENT_W, content_h));
    (pane, content_h)
}

/// header:齿轮图标 + 标题 + 标题右侧「重置」按钮。
fn build_header(
    pane: &Retained<NSView>,
    st: &Strings,
    delegate: &AppDelegate,
    g: &Geom,
    y: CGFloat,
) {
    // 按「墨迹中心」对齐:NSTextField 文字墨迹低于框中心 ~6px,标题 sizeToFit 取自然高后
    // 与齿轮都居中到 band_center,让两者墨迹(而非框)同高。
    let band_center = y + CONTENT_HEADER_H / 2.0;
    let gear_s = 20.0;
    add_header_icon(
        pane,
        NSRect::new(
            NSPoint::new(g.x0, band_center - gear_s / 2.0),
            NSSize::new(gear_s, gear_s),
        ),
        "gearshape",
    );
    let title = add_text(
        pane,
        NSRect::new(
            NSPoint::new(g.x0 + 28.0, y),
            NSSize::new(COL_W - 28.0, CONTENT_HEADER_H),
        ),
        st.general,
        false,
        true,
    );
    title.sizeToFit();
    let fit_h = title.frame().size.height;
    title.setFrame(NSRect::new(
        NSPoint::new(g.x0 + 28.0, band_center - fit_h / 2.0),
        NSSize::new(COL_W - 28.0, fit_h),
    ));
    // 标题右侧「重置」按钮(重置本页 General 字段,不含语言/状态样式;与 state pane 一致)。
    let reset = add_plain_button(
        pane,
        NSRect::new(
            NSPoint::new(CONTENT_W - CONTENT_PAD_X - 70.0, band_center - 12.0),
            NSSize::new(70.0, 24.0),
        ),
        st.reset,
        0,
        sel!(resetGeneral:),
        delegate,
    );
    reset.setAutoresizingMask(NSAutoresizingMaskOptions(1)); // 贴右(MinXMargin)
}

/// Group-1:语言单选(English / 中文)+ 「重置所有」按钮(弹确认 → 重置全部自定义)。
fn build_group1(
    pane: &Retained<NSView>,
    st: &Strings,
    delegate: &AppDelegate,
    g: &Geom,
    y: CGFloat,
) {
    add_card(pane, card_frame(g.x0, y, 2));
    // Language(标签 + English / 中文 单选;默认中文)
    add_text(
        pane,
        NSRect::new(
            NSPoint::new(g.lx, row_center_y(y, 0) - 10.0),
            NSSize::new(g.lw, 20.0),
        ),
        st.language,
        false,
        false,
    );
    let lang = delegate.ivars().settings.borrow().lang;
    let en_btn = add_radio_button(
        pane,
        NSRect::new(
            NSPoint::new(g.cx, row_center_y(y, 0) - 11.0),
            NSSize::new(90.0, 22.0),
        ),
        "English",
        LANG_EN_TAG,
        delegate,
        sel!(changeLanguage:),
    );
    let zh_btn = add_radio_button(
        pane,
        NSRect::new(
            NSPoint::new(g.cx + 100.0, row_center_y(y, 0) - 11.0),
            NSSize::new(90.0, 22.0),
        ),
        "中文",
        LANG_ZH_TAG,
        delegate,
        sel!(changeLanguage:),
    );
    en_btn.setState(if lang == Lang::En {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
    zh_btn.setState(if lang == Lang::Zh {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
    // Reset All(按钮 → 确认对话框 → 重置全部自定义:语言 + 各状态灯效)
    let _ = add_plain_button(
        pane,
        NSRect::new(
            NSPoint::new(g.lx, row_center_y(y, 1) - 14.0),
            NSSize::new(130.0, 28.0),
        ),
        st.reset_all,
        0,
        sel!(resetAll:),
        delegate,
    );
}

/// Group-2:灯大小 / 点击穿透 / 轮询 / Agent chip / 通知 chip / 开机启动 / Theme / 全屏隐藏。
/// 行号用游标 `row` 递增(替代硬编码 + offset 链;chip 占多行时跳过换行数)。返回实际行数。
fn build_group2(
    pane: &Retained<NSView>,
    st: &Strings,
    delegate: &AppDelegate,
    g: &Geom,
    y: CGFloat,
) -> usize {
    let group2 = add_card(pane, card_frame(g.x0, y, 6));
    let chip_max_x = g.cx + g.cw;
    let mut row: usize = 0;

    // Light size(标签 + 滑块 + 右侧 `xx px` 实时标签)
    add_text(
        pane,
        NSRect::new(
            NSPoint::new(g.lx, row_center_y(y, row) - 10.0),
            NSSize::new(g.lw, 20.0),
        ),
        st.light_size,
        false,
        false,
    );
    let dot = delegate.ivars().settings.borrow().dot_size;
    let size_slider = add_slider(
        pane,
        NSRect::new(
            NSPoint::new(g.cx, row_center_y(y, row) - 11.0),
            NSSize::new(g.cw - 60.0, 22.0),
        ),
        DOT_SIZE_MIN_PX as f64,
        DOT_SIZE_MAX_PX as f64,
        dot as f64,
        sel!(changeSize:),
        delegate,
    );
    let size_label = add_text(
        pane,
        NSRect::new(
            NSPoint::new(g.cx + g.cw - 52.0, row_center_y(y, row) - 10.0),
            NSSize::new(52.0, 20.0),
        ),
        &format!("{} px", dot),
        false,
        false,
    );
    size_label.setTag(SIZE_LABEL_TAG as isize);
    // 滑块宽随 pane 拉伸,右侧 `xx px` 标签贴右(MinXMargin);两者间距恒定。
    size_slider.setAutoresizingMask(NSAutoresizingMaskOptions(2));
    size_label.setAutoresizingMask(NSAutoresizingMaskOptions(1));
    row += 1;

    // Click-through(开关;与 Drop-down「锁定」同步同一开关)
    add_text(
        pane,
        NSRect::new(
            NSPoint::new(g.lx, row_center_y(y, row) - 10.0),
            NSSize::new(g.lw, 20.0),
        ),
        st.click_through,
        false,
        false,
    );
    add_switch(
        pane,
        NSRect::new(
            NSPoint::new(g.cx - SWITCH_INSET, row_center_y(y, row) - 11.0),
            NSSize::new(40.0, 22.0),
        ),
        *delegate.ivars().click_through.borrow(),
        sel!(toggleClickThrough:),
        delegate,
    );
    row += 1;

    // Agent poll interval(标签 + 下拉;1/2/3/5/10/15 秒)
    add_text(
        pane,
        NSRect::new(
            NSPoint::new(g.lx, row_center_y(y, row) - 10.0),
            NSSize::new(g.lw, 20.0),
        ),
        st.poll_interval,
        false,
        false,
    );
    let poll_ms = delegate.ivars().settings.borrow().poll_interval_ms;
    add_popup(
        pane,
        NSRect::new(
            NSPoint::new(g.cx, row_center_y(y, row) - 13.0),
            NSSize::new(120.0, 26.0),
        ),
        &st.poll_opts,
        poll_preset_index(poll_ms),
        sel!(changePollInterval:),
        delegate,
        0,
    );
    row += 1;

    // Agent to monitor(标签 + 多选 chip:Claude Code / CodeBuddy / OpenClaw;选中=监控,放不下换行)
    add_text(
        pane,
        NSRect::new(
            NSPoint::new(g.lx, row_center_y(y, row) - 10.0),
            NSSize::new(g.lw, 20.0),
        ),
        st.agent_monitor,
        false,
        false,
    );
    let enabled = delegate.ivars().settings.borrow().enabled_agents.clone();
    let agent_extra = flow_chips(
        pane,
        delegate,
        &st.agent_opts,
        AGENT_OFF,
        sel!(changeEnabledAgents:),
        &AGENT_KIND_ORDER,
        &enabled,
        g.cx,
        chip_max_x,
        row_center_y(y, row),
        CHIP_GAP,
        CHIP_VGAP,
    );
    row += 1 + agent_extra;

    // Status notifications(标签 + 5 个 AgentStatus chip;选中=转入该状态时弹系统通知,默认 [NeedsDeci, Error])
    add_text(
        pane,
        NSRect::new(
            NSPoint::new(g.lx, row_center_y(y, row) - 10.0),
            NSSize::new(g.lw, 20.0),
        ),
        st.notify,
        false,
        false,
    );
    let notify_on = delegate.ivars().settings.borrow().notify_on.clone();
    let notify_extra = flow_chips(
        pane,
        delegate,
        &st.notify_opts,
        NOTIFY_OFF,
        sel!(changeNotifyOn:),
        &NOTIFY_STATUS_ORDER,
        &notify_on,
        g.cx,
        chip_max_x,
        row_center_y(y, row),
        CHIP_GAP,
        CHIP_VGAP,
    );
    row += 1 + notify_extra;

    // Launch at login(标签 + 开关,占位禁用)
    add_text(
        pane,
        NSRect::new(
            NSPoint::new(g.lx, row_center_y(y, row) - 10.0),
            NSSize::new(g.lw, 20.0),
        ),
        st.launch_login,
        false,
        false,
    );
    let launch = add_switch(
        pane,
        NSRect::new(
            NSPoint::new(g.cx - SWITCH_INSET, row_center_y(y, row) - 11.0),
            NSSize::new(40.0, 22.0),
        ),
        false,
        sel!(noop:),
        delegate,
    );
    launch.setEnabled(false);
    row += 1;

    // Theme(标签 + 横向 radio:跟随系统 / 深色 / 浅色)。固定 gap 紧凑成组(不填满 cw):
    // W 加大后 cw 变宽,按 (cw-total_w)/2 自适应会把三个 radio 均匀撑满控件区、间距过宽读作离散。
    add_text(
        pane,
        NSRect::new(
            NSPoint::new(g.lx, row_center_y(y, row) - 10.0),
            NSSize::new(g.lw, 20.0),
        ),
        st.theme,
        false,
        false,
    );
    let theme_idx = theme_index(delegate.ivars().settings.borrow().theme);
    let theme_row_y = row_center_y(y, row) - 11.0;
    let mut rx = g.cx;
    for (i, &opt) in st.theme_opts.iter().enumerate() {
        let btn = add_radio_button(
            pane,
            NSRect::new(NSPoint::new(g.cx, theme_row_y), NSSize::new(100.0, 22.0)),
            opt,
            THEME_OFF + i as i64,
            delegate,
            sel!(changeTheme:),
        );
        // sizeToFit 就地改 frame,调完再读拿自适应宽(否则拿垃圾值裁标题);+2 留呼吸。
        btn.sizeToFit();
        let w = btn.frame().size.width + 2.0;
        btn.setFrameSize(NSSize::new(w, 22.0));
        btn.setFrameOrigin(NSPoint::new(rx, theme_row_y));
        btn.setState(if i == theme_idx {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        rx += w + THEME_GAP;
    }
    row += 1;

    // Hide in fullscreen(开关;默认开)。与「点击穿透」同属浮窗行为开关。
    add_text(
        pane,
        NSRect::new(
            NSPoint::new(g.lx, row_center_y(y, row) - 10.0),
            NSSize::new(g.lw, 20.0),
        ),
        st.hide_in_fullscreen,
        false,
        false,
    );
    add_switch(
        pane,
        NSRect::new(
            NSPoint::new(g.cx - SWITCH_INSET, row_center_y(y, row) - 11.0),
            NSSize::new(40.0, 22.0),
        ),
        delegate.ivars().settings.borrow().hide_in_fullscreen,
        sel!(toggleHideInFullscreen:),
        delegate,
    );
    row += 1;

    // Group-2 实际行数 = 游标最终值(= 8 + agent_extra + notify_extra)。据此设 card 高。
    group2.setFrame(card_frame(g.x0, y, row));
    row
}
