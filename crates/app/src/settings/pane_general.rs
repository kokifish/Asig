//! General pane(常规设置卡):语言 / 灯大小 / 点击穿透 / 轮询 / 开机启动 / Agent chip / Theme / Reset。

use objc2::DefinedClass;
use objc2::rc::Retained;
use objc2::{msg_send, sel};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize};

use agent_light_core::{DOT_SIZE_MAX_PX, DOT_SIZE_MIN_PX, Lang};

use crate::app_delegate::AppDelegate;

use super::controls::{
    add_agent_chip, add_card, add_header_icon, add_plain_button, add_popup, add_radio_button,
    add_slider, add_switch, add_text, apply_chip_style, set_tag,
};
use super::strings::Strings;
use super::tags::{
    AGENT_KIND_ORDER, AGENT_OFF, COL_W, CONTENT_HEADER_H, CONTENT_PAD_X, CONTENT_W, H, HEADER_GAP,
    LANG_EN_TAG, LANG_ZH_TAG, SIZE_LABEL_TAG, THEME_OFF, TOP_INSET, card_frame, card_height,
    poll_preset_index, row_center_y, theme_index,
};

pub(crate) fn build_general_pane(delegate: &AppDelegate, st: &Strings) -> Retained<NSView> {
    let pane = super::controls::new_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_W, H),
    ));
    let x0 = CONTENT_PAD_X;
    let lx = x0 + 16.0; // 标签 x
    let cx = x0 + 200.0; // 控件 x
    let cw = COL_W - 200.0 - 16.0; // 控件区宽
    let lw = cx - lx; // 标签列宽(容纳最长标签,不裁剪)
    let mut y = H - CONTENT_PAD_X - TOP_INSET;

    // header:齿轮图标 + 标题(DEV.md General Settings Card 的 icon + Name)。
    // 关键:按「墨迹中心」而非「框中心」对齐——NSTextField 在偏高的框里会按基线把文字画到
    // 下部(墨迹低于框中心 ~6px),而 NSImageView 几何居中其 image;若只把两者框中心对齐,
    // 文字会读作比齿轮低(实测低 ~4px)。故标题先 sizeToFit 取文字自然高,再把 tight 框与
    // 齿轮框都居中到同一条 band_center,让两者的墨迹中心落到同一水平线。
    let band_center = y + CONTENT_HEADER_H / 2.0;
    let gear_s = 20.0;
    add_header_icon(
        &pane,
        NSRect::new(
            NSPoint::new(x0, band_center - gear_s / 2.0),
            NSSize::new(gear_s, gear_s),
        ),
        "gearshape",
    );
    let title = add_text(
        &pane,
        NSRect::new(
            NSPoint::new(x0 + 28.0, y),
            NSSize::new(COL_W - 28.0, CONTENT_HEADER_H),
        ),
        st.general,
        false,
        true,
    );
    title.sizeToFit();
    let fit = title.frame();
    let fit_h = fit.size.height;
    title.setFrame(NSRect::new(
        NSPoint::new(x0 + 28.0, band_center - fit_h / 2.0),
        NSSize::new(COL_W - 28.0, fit_h),
    ));
    y -= HEADER_GAP;

    // —— Group-1:语言 + 重置所有(DEV.md「Group 不带名称,仅分组」,顺序即从上至下)——
    add_card(&pane, card_frame(x0, y, 2));
    // Language(标签 + English / 中文 单选;默认中文)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 0) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.language,
        false,
        false,
    );
    add_radio_button(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_center_y(y, 0) - 11.0),
            NSSize::new(90.0, 22.0),
        ),
        "English",
        LANG_EN_TAG,
        delegate,
        sel!(changeLanguage:),
    );
    add_radio_button(
        &pane,
        NSRect::new(
            NSPoint::new(cx + 100.0, row_center_y(y, 0) - 11.0),
            NSSize::new(90.0, 22.0),
        ),
        "中文",
        LANG_ZH_TAG,
        delegate,
        sel!(changeLanguage:),
    );
    let lang = delegate.ivars().settings.borrow().lang;
    let want_tag = if lang == Lang::En {
        LANG_EN_TAG
    } else {
        LANG_ZH_TAG
    };
    for t in [LANG_EN_TAG, LANG_ZH_TAG] {
        if let Some(b) = super::view_with_tag(&pane, t) {
            // setState 取 NSControlStateValue enum;此处用裸值 1/0 表 on/off,保留 msg_send!。
            unsafe {
                let _: () = msg_send![&b, setState: if t == want_tag { 1i64 } else { 0 }];
            }
        }
    }
    // Reset All(按钮 → 确认对话框 → 重置全部自定义:语言 + 各状态灯效)
    let _ = add_plain_button(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 1) - 14.0),
            NSSize::new(130.0, 28.0),
        ),
        st.reset_all,
        0,
        sel!(resetAll:),
        delegate,
    );
    y -= card_height(2) + super::tags::CARD_GAP;

    // —— Group-2:浮窗灯大小 / 点击穿透 / Agent状态轮询间隔 / 开机自启动 ——
    let group2 = add_card(&pane, card_frame(x0, y, 6));
    // Light size(标签 + 滑块 + 右侧 `xx px` 实时标签)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 0) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.light_size,
        false,
        false,
    );
    let dot = delegate.ivars().settings.borrow().dot_size;
    let size_slider = add_slider(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_center_y(y, 0) - 11.0),
            NSSize::new(cw - 60.0, 22.0),
        ),
        DOT_SIZE_MIN_PX as f64,
        DOT_SIZE_MAX_PX as f64,
        dot as f64,
        sel!(changeSize:),
        delegate,
    );
    let size_label = add_text(
        &pane,
        NSRect::new(
            NSPoint::new(cx + cw - 52.0, row_center_y(y, 0) - 10.0),
            NSSize::new(52.0, 20.0),
        ),
        &format!("{} px", dot),
        false,
        false,
    );
    set_tag(&size_label, SIZE_LABEL_TAG);
    // 滑块宽度随 pane 拉伸,右侧 `xx px` 标签贴右(MinXMargin);两者间距恒定。
    size_slider.setAutoresizingMask(NSAutoresizingMaskOptions(2));
    size_label.setAutoresizingMask(NSAutoresizingMaskOptions(1));
    // Click-through(标签 + 开关;与 Drop-down「锁定」同步同一开关)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 1) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.click_through,
        false,
        false,
    );
    add_switch(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_center_y(y, 1) - 11.0),
            NSSize::new(40.0, 22.0),
        ),
        *delegate.ivars().click_through.borrow(),
        sel!(toggleClickThrough:),
        delegate,
    );
    // Agent poll interval(标签 + 下拉;1/2/3/5/10/15 秒)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 2) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.poll_interval,
        false,
        false,
    );
    let poll_ms = delegate.ivars().settings.borrow().poll_interval_ms;
    add_popup(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_center_y(y, 2) - 13.0),
            NSSize::new(120.0, 26.0),
        ),
        &st.poll_opts,
        poll_preset_index(poll_ms),
        sel!(changePollInterval:),
        delegate,
        0,
    );
    // Agent to monitor(标签 + 多选 chip:Claude Code / CodeBuddy / OpenClaw;选中=监控,点击 toggle)。
    // chip=圆角块(选中=强调色边框+浅底,未选=细边框),宽按文字 sizeToFit 自适应;控件区放不下换行。
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 3) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.agent_monitor,
        false,
        false,
    );
    let enabled = delegate.ivars().settings.borrow().enabled_agents.clone();
    // chip flow:cx 起、固定间距,超出控件区右边界(cx+cw)换行。各 chip 宽不同(按文字自适应),
    // 故按实际宽累计(不等宽 flow),放不下换到下一行。
    const CHIP_GAP: CGFloat = 10.0;
    const CHIP_VGAP: CGFloat = 6.0;
    const CHIP_H: CGFloat = 22.0;
    let chip_max_x = cx + cw;
    let row0_center = row_center_y(y, 3);
    let mut ax = cx;
    let mut chip_row: usize = 0;
    for (i, &name) in st.agent_opts.iter().enumerate() {
        // 先以临时 origin 建好 chip 拿宽度,再按 flow 定位。
        let btn = add_agent_chip(
            &pane,
            NSPoint::new(0.0, 0.0),
            name,
            AGENT_OFF + i as i64,
            delegate,
        );
        let chip = super::controls::chip_of_button(&btn);
        let chip_w = chip.frame().size.width;
        if ax + chip_w > chip_max_x && ax > cx {
            chip_row += 1;
            ax = cx;
        }
        let row_center = row0_center - chip_row as CGFloat * (CHIP_H + CHIP_VGAP);
        chip.setFrameOrigin(NSPoint::new(ax, row_center - CHIP_H / 2.0));
        ax += chip_w + CHIP_GAP;
        apply_chip_style(&btn, enabled.contains(&AGENT_KIND_ORDER[i]));
    }
    // agent chip 占 chip_row+1 行;超出 1 行(extra)让后续行(开机/主题)+ Group-2 卡片高度下移。
    let extra = chip_row;
    if extra > 0 {
        group2.setFrame(card_frame(x0, y, 6 + extra));
    }
    // Launch at login(标签 + 开关,占位禁用)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 4 + extra) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.launch_login,
        false,
        false,
    );
    let launch = add_switch(
        &pane,
        NSRect::new(
            NSPoint::new(cx, row_center_y(y, 4 + extra) - 11.0),
            NSSize::new(40.0, 22.0),
        ),
        false,
        sel!(noop:),
        delegate,
    );
    launch.setEnabled(false);
    // Theme(标签 + 下拉:跟随系统 / 深色 / 浅色)
    add_text(
        &pane,
        NSRect::new(
            NSPoint::new(lx, row_center_y(y, 5 + extra) - 10.0),
            NSSize::new(lw, 20.0),
        ),
        st.theme,
        false,
        false,
    );
    // Theme(标签 + 横向 radio:跟随系统 / 深色 / 浅色;与「效果」同款单选)。
    // radio 宽度按标题 sizeToFit 自适应并横向累计,避免长标题(「跟随系统」)被截断。
    let theme_idx = theme_index(delegate.ivars().settings.borrow().theme);
    let mut rx = cx;
    for (i, &opt) in st.theme_opts.iter().enumerate() {
        let btn = add_radio_button(
            &pane,
            NSRect::new(
                NSPoint::new(rx, row_center_y(y, 5 + extra) - 11.0),
                NSSize::new(100.0, 22.0),
            ),
            opt,
            THEME_OFF + i as i64,
            delegate,
            sel!(changeTheme:),
        );
        // sizeToFit 返回 void(就地改 frame),不是返回自适应尺寸——直接当 NSSize 读会拿到
        // 垃圾值,算出错误的按钮宽,标题被裁掉(主题三个 radio 只见圆点不见名称的根因)。
        // 正确做法:调完 sizeToFit 再读 frame 拿自适应宽。
        btn.sizeToFit();
        let fitted = btn.frame();
        let w = fitted.size.width + 2.0;
        btn.setFrameSize(NSSize::new(w, 22.0));
        if i == theme_idx {
            // setState 取 NSControlStateValue enum;用裸值 1 表 on,保留 msg_send!。
            unsafe {
                let _: () = msg_send![&btn, setState: 1i64];
            }
        }
        rx += w + 28.0;
    }

    pane
}
