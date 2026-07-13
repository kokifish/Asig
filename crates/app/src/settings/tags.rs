//! 设置窗口的几何/布局常量、tag 编码与几何 helper。
//!
//! 常量保持原 visibility(pub const 维持 pub,其余私有);helper 多为模块间复用 → pub(crate)。

use objc2::rc::{Allocated, Retained};
use objc2::{MainThreadMarker, class, msg_send};
use objc2_app_kit::{NSFont, NSImage, NSTextField};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use agent_light_core::{AgentKind, AgentStatus, Anim, Color, StyleKey, Theme};

// 窗口最小宽度。加大到 780 让 General pane 的「监控的 Agent」(3 chip)与「状态通知」(5 chip)
// 默认单行排开不换行 → Group-2 card 只 7 行(content_h≈436 < H=460),首屏完整显示,
// Theme 行不再因 card 被窗口底截断而看似越界。
pub(crate) const W: CGFloat = 780.0;
pub(crate) const H: CGFloat = 460.0;
pub(crate) const SIDEBAR_W: CGFloat = 160.0;
pub const CONTENT_W: CGFloat = W - SIDEBAR_W;
pub(crate) const CONTENT_PAD_X: CGFloat = 22.0;
pub(crate) const CONTENT_HEADER_H: CGFloat = 26.0;
/// 标题(下方不再有横线)到首张卡片的间距。
pub(crate) const HEADER_GAP: CGFloat = 16.0;
/// 标题栏高度。窗口 fullSizeContentView + 透明标题栏(主液态玻璃渗透到顶),但 pane 内的「内容」
/// (tab / 标题 / 卡片)必须从标题栏下方开始,否则会压在标题栏下/与红黄绿重叠。距顶锚点扣除本值。
pub(crate) const TOP_INSET: CGFloat = 28.0;
/// 浮动侧栏玻璃面板:距窗边留白(左/下/右),面板顶到标题栏下、底到窗底留白。
pub(crate) const SIDEBAR_INSET: CGFloat = 10.0;
pub(crate) const SIDEBAR_PANE_W: CGFloat = SIDEBAR_W - 2.0 * SIDEBAR_INSET;
pub(crate) const SIDEBAR_PANE_H: CGFloat = H - TOP_INSET - SIDEBAR_INSET;

/// 关于页显示的仓库链接(占位,改成真实仓库)。
pub(crate) const GITHUB_URL: &str = "https://github.com/koki/Asig";

pub const ANIM_ORDER: [Anim; 3] = [Anim::Steady, Anim::Pulse, Anim::Ripple];
pub const COLOR_ORDER: [Color; 12] = [
    Color::LightBlue,
    Color::Green,
    Color::Yellow,
    Color::Amber,
    Color::Red,
    Color::Purple,
    // —— 个性化扩展色(Tailwind,无默认状态映射)——
    Color::Blue,
    Color::Indigo,
    Color::Teal,
    Color::Cyan,
    Color::Orange,
    Color::Pink,
];
/// 轮询间隔下拉的可选项(ms)。index ↔ 选中项。
pub const POLL_PRESETS_MS: [u32; 6] = [1000, 2000, 3000, 5000, 10000, 15000];

/// General pane「监控的 Agent」多选 chip 的 tag 基数(+0/1/2 = Claude/CodeBuddy/OpenClaw)。
/// 避让 LANG 50x / SIZE_LABEL 503 / THEME 600。
pub const AGENT_OFF: i64 = 700;
/// Agent chip 顺序;引用 core 的 `AgentKind::IMPLEMENTED` 单一事实源(= 默认启用列表)。
pub const AGENT_KIND_ORDER: [AgentKind; 3] = AgentKind::IMPLEMENTED;

/// General pane「状态通知」多选 chip 的 tag 基数(+0..4 = 5 个 AgentStatus)。
/// 避让 AGENT_OFF 700 / THEME 600。
pub const NOTIFY_OFF: i64 = 800;
/// 状态通知 chip 顺序(Done/Working/NeedsDeci/Error/Offline);与 `strings::notify_opts` 同序。
pub const NOTIFY_STATUS_ORDER: [AgentStatus; 5] = [
    AgentStatus::Done,
    AgentStatus::Working,
    AgentStatus::NeedsDeci,
    AgentStatus::Error,
    AgentStatus::Offline,
];

pub const TAB_GENERAL: i64 = 0;
pub const TAB_ABOUT: i64 = 7;

/// 状态 tab 顺序(DEV.md「Left Side Tabs」)。label 由 `Strings.state` 按本地化填。
pub(crate) const STATE_KEYS: [(i64, StyleKey); 6] = [
    (1, StyleKey::DoneNotif),
    (2, StyleKey::Done),
    (3, StyleKey::Working),
    (4, StyleKey::NeedsDeci),
    (5, StyleKey::Error),
    (6, StyleKey::Offline),
];

// 状态控件 tag sub 偏移(base = tab_id*1000)。
pub const COLOR_OFF: i64 = 10;
pub const ANIM_OFF: i64 = 20;
pub const SPEED_OFF: i64 = 30;
pub const SPEED_LABEL_OFF: i64 = 31;
pub const RESET_OFF: i64 = 40;
// State pane「渐变层数」滑块 sub offset(整数拉杆 0..=4,仅作用于浮窗圆点本体)。
pub const GRADIENT_OFF: i64 = 50;
pub const GRADIENT_LABEL_OFF: i64 = 51;
// General pane 语言单选 tag。
pub const LANG_EN_TAG: i64 = 501;
pub const LANG_ZH_TAG: i64 = 502;
// General pane「浮窗灯大小」右侧 `xx px` 实时标签 tag(changeSize 时按它刷新)。
pub const SIZE_LABEL_TAG: i64 = 503;
// General pane「Theme」radio tag 基数(+0/1/2 = 跟随系统/深色/浅色)。
pub const THEME_OFF: i64 = 600;

pub const SPEED_MIN: f64 = 0.2;
pub const SPEED_MAX: f64 = 5.0;
pub(crate) const SWATCH_D: CGFloat = 28.0;
/// 相邻色块之间的固定像素间距(恒定,不随宽度变);色块按此间距左对齐 flow,
/// 放不下则换行(每行数量可不同),窗口拉到很宽时合并为 1 行。
pub(crate) const COLOR_GAP: CGFloat = 15.0;

// 右区内容布局:标题属于 content panel 的 header;卡片与标题左边缘对齐。
pub(crate) const COL_W: CGFloat = CONTENT_W - CONTENT_PAD_X * 2.0;
pub(crate) const ROW_H: CGFloat = 32.0;
/// 卡片内顶部/底部留白;行间距 = ROW_H。所有行内容垂直居中对齐到 row_center_y。
pub(crate) const CARD_TOP_PAD: CGFloat = 10.0;
pub(crate) const CARD_BOT_PAD: CGFloat = 10.0;
/// 卡片之间的统一间距。
pub(crate) const CARD_GAP: CGFloat = 20.0;

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
        let alloc: Allocated<NSTextField> = unsafe { msg_send![class!(NSTextField), alloc] };
        let field = NSTextField::initWithFrame(
            alloc,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
        );
        field.setStringValue(&NSString::from_str(s));
        field.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        field.sizeToFit();
        let f = field.frame();
        max_w = max_w.max(f.size.width);
        let _ = mtm; // 主线程标记已用(构造须主线程)
    }
    max_w + PADDING
}
