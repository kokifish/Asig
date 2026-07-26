//! Phase 2:全局置顶、透明、默认点击穿透的"药丸"浮窗 + CoreAnimation 灯效。
//!
//! 渲染:自绘 NSView(NSBezierPath 圆角 + NSColor 填充)——绕开 CALayer 的 CGColor 依赖。
//! 灯效(全部交 render server 进程驱动 GPU 插值,本进程 CPU ~0%):
//!   - Steady 常亮 / Pulse 呼吸(快闪·慢闪·呼吸只是周期不同):动 layer "opacity";
//!   - Ripple 波纹:两个自绘环子视图错相扩散,动其 layer "transform"(绕圆心缩放的
//!     CATransform3D)+ "opacity",从中心扩散并淡出(环也自绘,故无需 CGColor)。
//!
//! 窗口固定大尺寸(120×120,透明 + 默认点击穿透),核心圆点按设置 `dot_size` 居中绘制、
//! 波纹环在其中扩散。改大小只重绘圆点,**不**改窗口尺寸 —— 避免运行时对窗口发
//! setFrame 结构体消息(此前 KVO 窗口 setFrame 曾崩)。
//! 浮窗位置跨启动记忆(见 `build` 的 `saved` 参数 + `app_delegate::persist_light_pos`)。

use std::cell::RefCell;
use std::ptr::NonNull;

use agent_light_core::{
    Color, GRADIENT_LAYERS_DEFAULT, GRADIENT_LAYERS_MAX, GRADIENT_LAYERS_MIN, LightAnim,
    LightPosition, Theme,
};
use block2::RcBlock;
use objc2::rc::{Allocated, Retained, autoreleasepool};
use objc2::runtime::Bool;
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
};
use objc2_app_kit::{
    NSAppearance, NSApplication, NSBackingStoreType, NSBezierPath, NSColor, NSImage, NSScreen,
    NSView, NSWindingRule, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRect, NSSize, NSString, NSValue};
use objc2_quartz_core::{
    CABasicAnimation, CAKeyframeAnimation, CALayer, CAMediaTiming, CATransform3D,
    NSValueCATransform3DAdditions,
};

/// 固定窗口尺寸(透明,容得下最大圆点 + 波纹扩散)。
const WIN: CGFloat = 120.0;

/// 该 NSAppearance 是否为深色(name 含 "Dark":darkAqua / vibrantDark / …)。
fn appearance_is_dark(appearance: &NSAppearance) -> bool {
    let name = appearance.name();
    autoreleasepool(|pool| unsafe { name.to_str(pool) }.contains("Dark"))
}

/// 当前 app 外观是否深色(读 `NSApp.effectiveAppearance`)。
pub fn is_dark_appearance() -> bool {
    let mtm = MainThreadMarker::new().expect("is_dark_appearance 须在主线程");
    let app = NSApplication::sharedApplication(mtm);
    let appearance = app.effectiveAppearance();
    appearance_is_dark(&appearance)
}

/// 据 Theme 设 `NSApp.appearance`(FollowSystem→nil 继承系统;Dark/Light→对应固定外观)。
pub fn apply_theme(theme: Theme) {
    let mtm = MainThreadMarker::new().expect("apply_theme 须在主线程");
    let app = NSApplication::sharedApplication(mtm);
    let appearance = match theme {
        Theme::FollowSystem => None,
        Theme::Dark => {
            NSAppearance::appearanceNamed(&NSString::from_str("NSAppearanceNameVibrantDark"))
        }
        Theme::Light => NSAppearance::appearanceNamed(&NSString::from_str("NSAppearanceNameAqua")),
    };
    app.setAppearance(appearance.as_deref());
}

/// `c` 色的**动态** NSColor:浮窗自绘 `drawRect` 每次重绘按当前绘图外观取浅/深档。
/// (栅格化场景如 swatch 位图请用 `swatch_solid_nscolor`,否则动态色会被冻结。)
pub fn nscolor(c: Color) -> Retained<NSColor> {
    let [light, dark] = c.rgb_pair();
    let block: RcBlock<dyn Fn(NonNull<NSAppearance>) -> NonNull<NSColor>> = RcBlock::new(
        move |appearance: NonNull<NSAppearance>| -> NonNull<NSColor> {
            let (r, g, b) = if appearance_is_dark(unsafe { appearance.as_ref() }) {
                dark
            } else {
                light
            };
            let color = NSColor::colorWithCalibratedRed_green_blue_alpha(r, g, b, 1.0);
            // block 返回约定 +1 retained:into_raw 转移所有权给调用方,不 release。
            unsafe { NonNull::new_unchecked(Retained::into_raw(color)) }
        },
    );
    unsafe { NSColor::colorWithName_dynamicProvider(None, &block) }
}

/// `c` 色的**当前外观**静态 NSColor —— 给 swatch 位图栅格化用(`lockFocus` 会冻结
/// dynamicProvider,故色块 / 菜单栏图标必须取当下具体值;外观变化时由上层重生成)。
pub fn swatch_solid_nscolor(c: Color) -> Retained<NSColor> {
    let [light, dark] = c.rgb_pair();
    let (r, g, b) = if is_dark_appearance() { dark } else { light };
    NSColor::colorWithCalibratedRed_green_blue_alpha(r, g, b, 1.0)
}

/// 画一个 `c` 色的实心圆 NSImage(菜单栏图标 / 设置页色块用)。`selected` 时描一圈
/// `controlAccentColor` 外环表示选中。`setTemplate:NO` 保留真彩(否则菜单栏/按钮按
/// 模板渲染成单色)。
#[allow(deprecated)] // lockFocus/unlockFocus 栅格化(换 imageWithSize:flipped:drawingHandler: 收益不值)
pub fn swatch_image(c: Color, diameter: CGFloat, selected: bool) -> Retained<NSImage> {
    let alloc: Allocated<NSImage> = unsafe { msg_send![class!(NSImage), alloc] };
    let img = NSImage::initWithSize(alloc, NSSize::new(diameter, diameter));
    img.setTemplate(false);
    img.lockFocus();
    // 实心填充圆
    let inset: CGFloat = if selected { 3.0 } else { 2.0 };
    let d = diameter - inset * 2.0;
    let fill_rect = NSRect::new(NSPoint::new(inset, inset), NSSize::new(d, d));
    let fill_path = NSBezierPath::bezierPathWithOvalInRect(fill_rect);
    swatch_solid_nscolor(c).set();
    fill_path.fill();
    // 选中:外环
    if selected {
        let lw: CGFloat = 2.0;
        let ring_rect = NSRect::new(
            NSPoint::new(lw / 2.0, lw / 2.0),
            NSSize::new(diameter - lw, diameter - lw),
        );
        let ring = NSBezierPath::bezierPathWithOvalInRect(ring_rect);
        let accent = NSColor::controlAccentColor();
        ring.setLineWidth(lw);
        accent.set();
        ring.stroke();
    }
    img.unlockFocus();
    img
}

/// 系统「Reduce Motion」是否开启(无障碍 → Display)。开启时浮窗动画降级为常亮,
/// 状态仍由颜色区分 —— 避免对晕动症用户持续脉冲/扩散。
pub fn reduce_motion_on() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// 系统「Reduce Transparency」是否开启(无障碍 → Display)。开启时液态玻璃退化不透明
/// (走 NSVisualEffectView,其自动在 Reduce Transparency 下变实色),保证内容可读。
pub fn reduce_transparency_on() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
}

/// 圆点在窗口内居中的左下角 origin。
fn dot_origin(dot: CGFloat) -> CGFloat {
    (WIN - dot) / 2.0
}

// ---- 屏幕几何:用于浮窗位置的记忆 / 恢复(含多屏) ----

/// 当前所有屏幕(screens[0] 是主屏 / 菜单栏所在屏)。
fn screens() -> Vec<Retained<NSScreen>> {
    let mtm = MainThreadMarker::new().expect("screens 须在主线程");
    NSScreen::screens(mtm).to_vec()
}

/// 屏幕的 CGDirectDisplayID(经 deviceDescription[@"NSScreenNumber"]);取不到返回 0。
fn screen_device_id(screen: &NSScreen) -> u32 {
    let dict = screen.deviceDescription();
    let key = NSString::from_str("NSScreenNumber");
    dict.objectForKey(&key)
        .and_then(|o| o.downcast::<NSNumber>().ok())
        .map(|n| n.integerValue() as u32)
        .unwrap_or(0)
}

fn point_in_rect(r: NSRect, p: NSPoint) -> bool {
    p.x >= r.origin.x
        && p.x <= r.origin.x + r.size.width
        && p.y >= r.origin.y
        && p.y <= r.origin.y + r.size.height
}

/// 点所在的屏的 display id;不在任何屏内返回 0(用于存「上次所在屏」)。
pub fn screen_id_at(pt: NSPoint) -> u32 {
    for s in screens() {
        if point_in_rect(s.frame(), pt) {
            return screen_device_id(&s);
        }
    }
    0
}

/// 主屏(screens[0])左上角的默认 origin:borderless 浮窗贴可见区(visibleFrame,
/// 已排除菜单栏 / Dock)左上,留小边距,大致落在窗口红黄绿按钮那一行。
fn default_origin(win: CGFloat) -> NSPoint {
    let mtm = MainThreadMarker::new().expect("default_origin 须在主线程");
    let vf: NSRect = match screens().into_iter().next() {
        Some(s) => s.visibleFrame(),
        None => NSScreen::mainScreen(mtm).expect("无屏幕").visibleFrame(),
    };
    const GAP: CGFloat = 8.0;
    NSPoint::new(vf.origin.x + GAP, vf.origin.y + vf.size.height - win - GAP)
}

/// 把 saved 位置解析成实际 origin:
/// - saved 所在屏仍在 → 贴该屏恢复,并夹到其可见区内(防分辨率变化跑出屏外);
/// - 屏已断开 / saved=None → 主屏左上角默认。
fn resolve_origin(saved: Option<LightPosition>, win: CGFloat) -> NSPoint {
    let Some(p) = saved else {
        return default_origin(win);
    };
    // 按坐标点找它实际所在的屏来 clamp,而非存的 screen_id:persist_light_pos 按窗口「中心」判屏却
    // 存「原点」(x,y),浮窗跨屏边界时(原点在 A 屏、中心越过接缝到 B 屏)会存成 (origin=A 屏坐标,
    // screen_id=B 屏)——若按 screen_id clamp,原点被推到 B 屏边缘的接缝里,浮窗落到两屏之间不可见。
    // 直接按坐标定位:落在哪屏就 clamp 到哪屏;screens()[0] 是主屏(Apple 保证),接缝上的点归主屏,
    // 避免落到副屏边缘。点不在任何屏(屏断开 / 坐标过期)→ 默认主屏左上角。
    let pt = NSPoint::new(p.x, p.y);
    let Some(s) = screens().into_iter().find(|s| point_in_rect(s.frame(), pt)) else {
        return default_origin(win);
    };
    let vf = s.visibleFrame();
    let max_x = (vf.origin.x + vf.size.width - win).max(vf.origin.x);
    let max_y = (vf.origin.y + vf.size.height - win).max(vf.origin.y);
    NSPoint::new(p.x.clamp(vf.origin.x, max_x), p.y.clamp(vf.origin.y, max_y))
}

// ---- PillView:自绘圆点 + 持有可选的波纹环 ----
pub struct PillState {
    pub color: Retained<NSColor>,
    /// 波纹环(2 个,错相扩散)。无波纹时为空。
    pub rings: Vec<Retained<RingView>>,
    pub dot: CGFloat,
    /// 渐变层数(slider 值 0..=4)。drawRect 据此画 layers+1 同心环;0=纯色单层。
    pub layers: u8,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PillView"]
    #[ivars = RefCell<PillState>]
    pub struct PillView;

    #[allow(non_snake_case)]
    impl PillView {
        /// 允许点击药丸拖动无边框窗口(配合 window movableByWindowBackground)。
        /// 仅在「关闭点击穿透」时窗口才接收鼠标事件,故只在那时生效。
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> Bool {
            Bool::YES
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let b = self.ivars().borrow();
            let color: &NSColor = &b.color;
            let dot = b.dot;
            // 渐变层数 layers(slider 值 0..=4) → 实际层数 L=layers+1。第 k 层(k=0 中心)透明度
            // α=1−k/L,按半径等距分段 [k/L·R, (k+1)/L·R](R=dot/2)。每段画 even-odd 环(外圆+内圆)
            // 各自独立 α、互不重叠 —— 避免 source-over 合成使中间层 α 累加(否则「中 2/3」会
            // 渗入外层色)。layers=0 → L=1 → 单个实心圆(等价历史纯色圆点)。
            let l = b.layers as usize + 1;
            let r = dot / 2.0;
            let c = dot_origin(dot) + r; // 圆心(正方形圆点,cx=cy=c)
            for k in 0..l {
                let frac_in = k as CGFloat / l as CGFloat;
                let r_out = (k as CGFloat + 1.0) / l as CGFloat * r;
                let outer = NSRect::new(
                    NSPoint::new(c - r_out, c - r_out),
                    NSSize::new(2.0 * r_out, 2.0 * r_out),
                );
                let path = NSBezierPath::bezierPathWithOvalInRect(outer);
                if k > 0 {
                    let r_in = frac_in * r;
                    let inner = NSRect::new(
                        NSPoint::new(c - r_in, c - r_in),
                        NSSize::new(2.0 * r_in, 2.0 * r_in),
                    );
                    path.appendBezierPathWithOvalInRect(inner);
                    path.setWindingRule(NSWindingRule::EvenOdd);
                }
                color.colorWithAlphaComponent(1.0 - frac_in).set();
                path.fill();
            }
        }

        /// 外观变化 → 重绘(drawRect 按当前外观重新解析动态色)。
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            self.setNeedsDisplay(true);
        }
    }
);

impl PillView {
    fn new(color: Retained<NSColor>, frame: NSRect, dot: CGFloat, layers: u8) -> Retained<Self> {
        let allocated: Allocated<Self> = unsafe { msg_send![Self::class(), alloc] };
        let partial = allocated.set_ivars(RefCell::new(PillState {
            color,
            rings: Vec::new(),
            dot,
            layers,
        }));
        unsafe { msg_send![super(partial), initWithFrame: frame] }
    }
}

// ---- RingView:波纹环(自绘描边圆,故无需 CGColor)----
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "RingView"]
    #[ivars = RefCell<Retained<NSColor>>]
    pub struct RingView;

    #[allow(non_snake_case)]
    impl RingView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let b = self.ivars().borrow();
            let color: &NSColor = &b;
            let bounds = self.bounds();
            let lw: CGFloat = 1.5;
            let inset = NSRect::new(
                NSPoint::new(lw / 2.0, lw / 2.0),
                NSSize::new(bounds.size.width - lw, bounds.size.height - lw),
            );
            let r = inset.size.height / 2.0;
            let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(inset, r, r);
            path.setLineWidth(lw);
            color.set();
            path.stroke();
        }

        /// 外观变化 → 重绘(描边色按当前外观重新解析动态色)。
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            self.setNeedsDisplay(true);
        }
    }
);

impl RingView {
    fn new(color: Retained<NSColor>, frame: NSRect) -> Retained<Self> {
        let allocated: Allocated<Self> = unsafe { msg_send![Self::class(), alloc] };
        let partial = allocated.set_ivars(RefCell::new(color));
        unsafe { msg_send![super(partial), initWithFrame: frame] }
    }
}

// ---- 构建浮窗 ----
/// `saved` = 上次记忆的位置(含所在屏 id);None 或该屏已断开 → 主屏左上角默认。
pub fn build(
    dot_size: u32,
    saved: Option<LightPosition>,
    hide_in_fullscreen: bool,
) -> (Retained<NSWindow>, Retained<PillView>) {
    let origin = resolve_origin(saved, WIN);
    let frame = NSRect::new(origin, NSSize::new(WIN, WIN));

    let alloc: Allocated<NSWindow> = unsafe { msg_send![class!(NSWindow), alloc] };
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            alloc,
            frame,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };

    let clear = NSColor::clearColor();
    window.setOpaque(false);
    window.setBackgroundColor(Some(&clear));
    window.setHasShadow(false);
    window.setIgnoresMouseEvents(true); // 默认点击穿透
    window.setMovableByWindowBackground(true); // 关穿透时可拖
    window.setLevel(objc2_app_kit::NSFloatingWindowLevel); // 浮窗置顶
    // hide_in_fullscreen=true → Managed(不进全屏 app 的 Space:全屏自动消失 + 不打断菜单栏 / Dock
    // 自动隐藏);false → CanJoinAllSpaces(浮窗跨所有 Space 显示,含全屏)。toggleHideInFullscreen 切换。
    set_hide_in_fullscreen(&window, hide_in_fullscreen);
    unsafe {
        // ARC 下手动 retain,需 unsafe。
        window.setReleasedWhenClosed(false);
    }

    let dot = dot_size as CGFloat;
    let view = PillView::new(
        nscolor(Color::Purple),
        NSRect::new(NSPoint::new(0.0, 0.0), frame.size),
        dot,
        GRADIENT_LAYERS_DEFAULT,
    );
    view.setWantsLayer(true);
    window.setContentView(Some(&view));
    window.orderFrontRegardless();
    (window, view)
}

/// 切换浮窗是否点击穿透。on=true → 忽略鼠标(穿透);on=false → 接收鼠标,可拖动。
pub fn set_click_through(window: &NSWindow, on: bool) {
    window.setIgnoresMouseEvents(on);
}

/// 切换全屏自动隐藏:on=true → Managed(不进全屏 app 的 Space,全屏自动消失 + 不打断菜单栏 / Dock
/// 自动隐藏);on=false → CanJoinAllSpaces(跨所有 Space 显示,含全屏)。build 与 toggle 共用此单一入口。
pub fn set_hide_in_fullscreen(window: &NSWindow, on: bool) {
    let b = if on {
        NSWindowCollectionBehavior::Managed
    } else {
        NSWindowCollectionBehavior::CanJoinAllSpaces
    };
    window.setCollectionBehavior(b);
}

/// 改圆点大小:更新 dot、拆掉按旧尺寸建的波纹环(下次 set_light 重建)、重绘。
pub fn set_size(view: &PillView, dot_size: u32) {
    view.ivars().borrow_mut().dot = dot_size as CGFloat;
    drain_rings(view);
    view.setNeedsDisplay(true);
}

// ---- 按灯效更新颜色 + 动画 ----
pub fn set_light(view: &PillView, anim: LightAnim, layers: u8) {
    // Reduce Motion 开启:动画降级为常亮(保留颜色),不脉冲/不扩散。渐变层数是正交参数,不受影响。
    let anim = if reduce_motion_on() {
        LightAnim::Steady {
            color: anim.color(),
        }
    } else {
        anim
    };
    view.rust_set_color(nscolor(anim.color()));
    view.rust_set_layers(layers);

    let layer = view.layer().expect("PillView 须 layer-backed");
    // 先清掉旧的:opacity 动画 + 波纹环子视图。
    layer.removeAnimationForKey(&NSString::from_str("pulse"));
    layer.setOpacity(1.0);
    drain_rings(view);

    match anim {
        LightAnim::Pulse { period_ms, .. } => add_pulse(&layer, period_ms),
        LightAnim::Ripple {
            color, period_ms, ..
        } => add_ripple(view, color, period_ms, layers),
        LightAnim::Steady { .. } => {}
    }
}

impl PillView {
    fn rust_set_color(&self, color: Retained<NSColor>) {
        self.ivars().borrow_mut().color = color;
        self.setNeedsDisplay(true);
    }

    /// 改渐变层数(仅变化时重绘)。
    fn rust_set_layers(&self, layers: u8) {
        let layers = layers.clamp(GRADIENT_LAYERS_MIN, GRADIENT_LAYERS_MAX);
        let changed = self.ivars().borrow().layers != layers;
        if changed {
            self.ivars().borrow_mut().layers = layers;
            self.setNeedsDisplay(true);
        }
    }
}

/// 拆掉所有波纹环子视图(set_size 改尺寸 / set_light 换灯效时清旧环),两处共用。
fn drain_rings(view: &PillView) {
    for ring in view.ivars().borrow_mut().rings.drain(..) {
        ring.removeFromSuperview();
    }
}

/// 呼吸:opacity 在 [0.2, 1.0] 间往复。周期越短视觉上越「闪」(快闪/慢闪/呼吸)。
fn add_pulse(layer: &CALayer, period_ms: u32) {
    const FLOOR: f64 = 0.2;
    let basic = CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str("opacity")));
    let from_n = NSNumber::numberWithDouble(FLOOR);
    let to_n = NSNumber::numberWithDouble(1.0);
    // autoreverses 下 duration 是半周期;period_ms 为完整周期。
    let duration = period_ms as f64 / 1000.0 / 2.0;
    // setFromValue/setToValue 强类型但仍 unsafe(AnyObject 类型校验留给调用方)。
    unsafe {
        basic.setFromValue(Some(&from_n));
        basic.setToValue(Some(&to_n));
    }
    basic.setDuration(duration); // CABediaTiming trait
    basic.setAutoreverses(true);
    basic.setRepeatCount(f32::INFINITY);
    layer.addAnimation_forKey(&basic, Some(&NSString::from_str("pulse")));
}

/// 波纹环数量。两环错相半个周期 → 视觉上连续扩散。
const RIPPLE_RINGS: usize = 2;

/// 波纹:N 个自绘环子视图错相扩散;transform 从 1.0 扩到 l(终态直径 = dot),opacity keyframe
/// 中段完全不透明(硬边)、末 15% 短淡出(掩盖 scale 单程回弹跳变)。
///
/// 居中:layer-backed NSView 的 anchorPoint/position 由 AppKit 托管(改了会被重置),故不动锚点,
/// 改用「绕环圆心缩放」的 CATransform3D(translate·scale·translate),圆心始终对齐圆点。
fn add_ripple(view: &PillView, color: Color, period_ms: u32, layers: u8) {
    let dot = view.ivars().borrow().dot;
    // 波纹从「最内层」(同心圆中心实心圆)外缘起扩散,而非整个圆点中心 —— 这样 layers>0
    // 时环从中心实心圆边缘出现、向外穿过半透明外层,视觉读作「从最内层扩散出去」
    // (最内层与环同色、重叠处本就不可辨,故起点贴其外缘)。layers=0 → L=1 → 最内层即整个
    // 圆点(起点=终点、scale=1,退化为静态环淡入淡出;默认 layers=1 正常扩散)。
    // 扩散终值:波纹环扩到灯边缘(终态直径 = dot),随当前 dot 大小成正比、永不超过窗口。
    let l = layers as CGFloat + 1.0;
    let inner_d = dot / l; // 最内层直径
    let o = dot_origin(dot) + (dot - inner_d) / 2.0; // 最内层在圆点内居中
    let ring_frame = NSRect::new(NSPoint::new(o, o), NSSize::new(inner_d, inner_d));
    let duration = period_ms as f64 / 1000.0;

    // 环视图自身坐标圆心 = (inner_d/2, inner_d/2)(环描边内切于 inner_d×inner_d bounds)。
    let c = inner_d / 2.0;
    let from_t = scale_about(c, c, 1.0);
    // 扩散终值:波纹环扩到灯边缘(终态直径 = dot)。scale = 终态直径 / inner_d = dot/(dot/l) = l。
    let to_t = scale_about(c, c, l);

    let mut rings = Vec::with_capacity(RIPPLE_RINGS);
    for i in 0..RIPPLE_RINGS {
        let ring = RingView::new(nscolor(color), ring_frame);
        ring.setWantsLayer(true);
        view.addSubview(&ring);
        let layer = ring.layer().expect("RingView 须 layer-backed");
        // 第 i 环偏移 i/N 个周期 → 多环均匀错相。
        let phase = i as f64 * duration / RIPPLE_RINGS as f64;
        ripple_anims(&layer, from_t, to_t, duration, phase);
        rings.push(ring);
    }
    view.ivars().borrow_mut().rings = rings;
}

/// 给一个环的 layer 装上 scale + opacity 动画(均单向无限循环;`phase` 用作 timeOffset 错相)。
fn ripple_anims(
    layer: &CALayer,
    from_t: CATransform3D,
    to_t: CATransform3D,
    duration: f64,
    phase: f64,
) {
    let scale = CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str("transform")));
    let opacity = CAKeyframeAnimation::animationWithKeyPath(Some(&NSString::from_str("opacity")));
    // valueWithCATransform3D 仍 unsafe(Additions trait 的 unsafe 约定)。
    let from_v = unsafe { NSValue::valueWithCATransform3D(from_t) };
    let to_v = unsafe { NSValue::valueWithCATransform3D(to_t) };
    // setFromValue/setToValue 强类型但仍 unsafe(AnyObject 类型校验留给调用方)。
    unsafe {
        scale.setFromValue(Some(&from_v));
        scale.setToValue(Some(&to_v));
    }
    scale.setDuration(duration);
    scale.setTimeOffset(phase);
    scale.setRepeatCount(f32::INFINITY);
    layer.addAnimation_forKey(&scale, Some(&NSString::from_str("rippleScale")));

    // opacity keyframe:前 12% 淡入 → 中段保持完全不透明(硬边)→ 末 15% 短淡出。
    // 全程不透明主体让环边缘锐利;末尾淡到 0 掩盖 scale 单程回弹的瞬间跳变(无可见重置)。
    let vals = NSArray::from_slice(&[
        &*NSNumber::numberWithFloat(0.0),
        &*NSNumber::numberWithFloat(1.0),
        &*NSNumber::numberWithFloat(1.0),
        &*NSNumber::numberWithFloat(0.0),
    ]);
    let times = NSArray::from_slice(&[
        &*NSNumber::numberWithFloat(0.0),
        &*NSNumber::numberWithFloat(0.12),
        &*NSNumber::numberWithFloat(0.85),
        &*NSNumber::numberWithFloat(1.0),
    ]);
    unsafe {
        // setValues 接 NSArray<NSObject>;此处是 NSArray<NSNumber>,经 msg_send! 绕过编译期泛型
        // (运行时 NSNumber 即 NSObject 子类,正确);setKeyTimes 类型匹配直接调。
        let _: () = msg_send![&opacity, setValues: &*vals];
        opacity.setKeyTimes(Some(&times));
    }
    opacity.setDuration(duration);
    opacity.setTimeOffset(phase);
    opacity.setRepeatCount(f32::INFINITY);
    layer.addAnimation_forKey(&opacity, Some(&NSString::from_str("rippleOpacity")));
}

/// 构造「绕点 (cx, cy) 缩放 s 倍」的 2D 仿射 CATransform3D(s=1 即单位矩阵)。
/// 不依赖 layer 的 anchorPoint,故对 layer-backed NSView 也稳定有效。
fn scale_about(cx: CGFloat, cy: CGFloat, s: CGFloat) -> CATransform3D {
    CATransform3D {
        m11: s,
        m12: 0.0,
        m13: 0.0,
        m14: 0.0,
        m21: 0.0,
        m22: s,
        m23: 0.0,
        m24: 0.0,
        m31: 0.0,
        m32: 0.0,
        m33: 1.0,
        m34: 0.0,
        m41: cx * (1.0 - s),
        m42: cy * (1.0 - s),
        m43: 0.0,
        m44: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::scale_about;
    use objc2_core_foundation::CGFloat;
    use objc2_quartz_core::CATransform3D;

    /// 把 2D 仿射 CATransform3D 作用到点 (x,y)(只用 m11/m21/m41 与 m12/m22/m42)。
    fn apply2d(t: &CATransform3D, x: CGFloat, y: CGFloat) -> (CGFloat, CGFloat) {
        (t.m11 * x + t.m21 * y + t.m41, t.m12 * x + t.m22 * y + t.m42)
    }

    #[test]
    fn scale_about_is_identity_at_one() {
        let t = scale_about(20.0, 20.0, 1.0);
        assert!((t.m11 - 1.0).abs() < 1e-9 && (t.m22 - 1.0).abs() < 1e-9);
        assert!(t.m41.abs() < 1e-9 && t.m42.abs() < 1e-9); // 无平移
        assert!((t.m33 - 1.0).abs() < 1e-9 && (t.m44 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn scale_about_fixes_center_point() {
        // 波纹居中的几何不变量:无论缩放多少倍,圆心 (c,c) 经变换后仍在原位 ——
        // 这正是「环以圆点为圆心对称扩散」、不因 anchorPoint 偏移的数学保证。
        for &c in &[10.0_f64, 20.0, 40.0] {
            for &s in &[1.3, 1.77, 2.0, 2.6] {
                let (x, y) = apply2d(&scale_about(c, c, s), c, c);
                assert!((x - c).abs() < 1e-9, "c={c} s={s}: x={x} != {c}");
                assert!((y - c).abs() < 1e-9, "c={c} s={s}: y={y} != {c}");
            }
        }
    }

    #[test]
    fn scale_about_scales_radius_about_center() {
        // 距圆心 r 的点,缩放后距圆心 s*r(环半径随 s 线性扩大,圆心不动)。
        let (c, r, s) = (20.0, 15.0, 2.0);
        let (x, y) = apply2d(&scale_about(c, c, s), c + r, c);
        let dist = ((x - c).powi(2) + (y - c).powi(2)).sqrt();
        assert!(
            (dist - s * r).abs() < 1e-9,
            "dist={dist} expected={}",
            s * r
        );
    }
}
