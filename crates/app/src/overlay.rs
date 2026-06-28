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

use agent_light_core::{Color, LightAnim, LightPosition};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{Bool, NSObject};
use objc2::{ClassType, DefinedClass, MainThreadOnly, class, define_class, msg_send};
use objc2_app_kit::{NSBezierPath, NSColor, NSImage, NSScreen, NSView, NSWindow};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{
    NSArray, NSDictionary, NSNumber, NSPoint, NSRect, NSSize, NSString, NSValue,
};
use objc2_quartz_core::{CABasicAnimation, CALayer, CATransform3D, NSValueCATransform3DAdditions};

/// 固定窗口尺寸(透明,容得下最大圆点 + 波纹扩散)。
const WIN: CGFloat = 120.0;

// ---- Color -> NSColor ----
pub fn nscolor(c: Color) -> Retained<NSColor> {
    let (r, g, b): (CGFloat, CGFloat, CGFloat) = match c {
        Color::Green => (0.20, 0.80, 0.30),     // Done
        Color::LightBlue => (0.30, 0.64, 0.96), // Done Notification(浅蓝)
        Color::Yellow => (0.95, 0.80, 0.15),    // Working
        Color::Amber => (0.95, 0.55, 0.10),     // NeedsDeci(橙)
        Color::Red => (0.92, 0.22, 0.22),       // Error
        Color::Purple => (0.62, 0.36, 0.90),    // Offline
    };
    NSColor::colorWithCalibratedRed_green_blue_alpha(r, g, b, 1.0)
}

/// 画一个 `c` 色的实心圆 NSImage(菜单栏图标 / 设置页色块用)。`selected` 时描一圈
/// `controlAccentColor` 外环表示选中。`setTemplate:NO` 保留真彩(否则菜单栏/按钮按
/// 模板渲染成单色)。
pub fn swatch_image(c: Color, diameter: CGFloat, selected: bool) -> Retained<NSImage> {
    let alloc: Allocated<NSImage> = unsafe { msg_send![class!(NSImage), alloc] };
    let img: Retained<NSImage> =
        unsafe { msg_send![alloc, initWithSize: NSSize::new(diameter, diameter)] };
    unsafe {
        let _: () = msg_send![&img, setTemplate: Bool::NO];
        let _: () = msg_send![&img, lockFocus];
        // 实心填充圆
        let inset: CGFloat = if selected { 3.0 } else { 2.0 };
        let d = diameter - inset * 2.0;
        let fill_rect = NSRect::new(NSPoint::new(inset, inset), NSSize::new(d, d));
        let fill_path: Retained<NSBezierPath> =
            msg_send![class!(NSBezierPath), bezierPathWithOvalInRect: fill_rect];
        let fill = nscolor(c);
        let _: () = msg_send![&fill, set];
        fill_path.fill();
        // 选中:外环
        if selected {
            let lw: CGFloat = 2.0;
            let ring_rect = NSRect::new(
                NSPoint::new(lw / 2.0, lw / 2.0),
                NSSize::new(diameter - lw, diameter - lw),
            );
            let ring: Retained<NSBezierPath> =
                msg_send![class!(NSBezierPath), bezierPathWithOvalInRect: ring_rect];
            let accent: Retained<NSColor> = msg_send![class!(NSColor), controlAccentColor];
            let _: () = msg_send![&ring, setLineWidth: lw];
            let _: () = msg_send![&accent, set];
            let _: () = msg_send![&ring, stroke];
        }
        let _: () = msg_send![&img, unlockFocus];
    }
    img
}

fn anim_color(a: LightAnim) -> Color {
    match a {
        LightAnim::Steady { color } => color,
        LightAnim::Pulse { color, .. } => color,
        LightAnim::Ripple { color, .. } => color,
    }
}

/// 圆点在窗口内居中的左下角 origin。
fn dot_origin(dot: CGFloat) -> CGFloat {
    (WIN - dot) / 2.0
}

// ---- 屏幕几何:用于浮窗位置的记忆 / 恢复(含多屏) ----

/// 当前所有屏幕(screens[0] 是主屏 / 菜单栏所在屏)。
fn screens() -> Vec<Retained<NSScreen>> {
    let arr: Retained<NSArray<NSScreen>> = unsafe { msg_send![class!(NSScreen), screens] };
    let n: usize = unsafe { msg_send![&arr, count] };
    (0..n)
        .map(|i| unsafe { msg_send![&arr, objectAtIndex: i] })
        .collect()
}

/// 屏幕的 CGDirectDisplayID(经 deviceDescription[@"NSScreenNumber"]);取不到返回 0。
fn screen_device_id(screen: &NSScreen) -> u32 {
    let dict: Retained<NSDictionary<NSString, NSObject>> =
        unsafe { msg_send![screen, deviceDescription] };
    let num: Retained<NSNumber> =
        unsafe { msg_send![&dict, objectForKey: &*NSString::from_str("NSScreenNumber")] };
    let v: i64 = unsafe { msg_send![&num, integerValue] };
    v as u32
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
        let fr: NSRect = unsafe { msg_send![&s, frame] };
        if point_in_rect(fr, pt) {
            return screen_device_id(&s);
        }
    }
    0
}

/// 按 display id 找屏;id=0 或屏已断开返回 None。
fn screen_with_id(id: u32) -> Option<Retained<NSScreen>> {
    if id == 0 {
        return None;
    }
    screens().into_iter().find(|s| screen_device_id(s) == id)
}

/// 主屏(screens[0])左上角的默认 origin:borderless 浮窗贴可见区(visibleFrame,
/// 已排除菜单栏 / Dock)左上,留小边距,大致落在窗口红黄绿按钮那一行。
fn default_origin(win: CGFloat) -> NSPoint {
    let vf: NSRect = match screens().into_iter().next() {
        Some(s) => unsafe { msg_send![&s, visibleFrame] },
        None => {
            let main: Retained<NSScreen> = unsafe { msg_send![class!(NSScreen), mainScreen] };
            unsafe { msg_send![&main, visibleFrame] }
        }
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
    let Some(s) = screen_with_id(p.screen_id) else {
        return default_origin(win);
    };
    let vf: NSRect = unsafe { msg_send![&s, visibleFrame] };
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
            let o = dot_origin(dot);
            let rect = NSRect::new(NSPoint::new(o, o), NSSize::new(dot, dot));
            let path: Retained<NSBezierPath> = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, dot / 2.0, dot / 2.0);
            let _: () = unsafe { msg_send![color, set] };
            path.fill();
        }
    }
);

impl PillView {
    fn new(color: Retained<NSColor>, frame: NSRect, dot: CGFloat) -> Retained<Self> {
        let allocated: Allocated<Self> = unsafe { msg_send![Self::class(), alloc] };
        let partial = allocated.set_ivars(RefCell::new(PillState {
            color,
            rings: Vec::new(),
            dot,
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
            let bounds: NSRect = unsafe { msg_send![self, bounds] };
            let lw: CGFloat = 1.5;
            let inset = NSRect::new(
                NSPoint::new(lw / 2.0, lw / 2.0),
                NSSize::new(bounds.size.width - lw, bounds.size.height - lw),
            );
            let r = inset.size.height / 2.0;
            let path: Retained<NSBezierPath> = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(inset, r, r);
            let _: () = unsafe { msg_send![&path, setLineWidth: lw] };
            let _: () = unsafe { msg_send![color, set] };
            let _: () = unsafe { msg_send![&path, stroke] };
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
) -> (Retained<NSWindow>, Retained<PillView>) {
    let origin = resolve_origin(saved, WIN);
    let frame = NSRect::new(origin, NSSize::new(WIN, WIN));

    let alloc: Allocated<NSWindow> = unsafe { msg_send![class!(NSWindow), alloc] };
    let window: Retained<NSWindow> = unsafe {
        msg_send![
            alloc,
            initWithContentRect: frame,
            styleMask: 0u64, // NSWindowStyleMaskBorderless
            backing: 2u64,   // NSBackingStoreBuffered
            defer: Bool::NO,
        ]
    };

    let clear = NSColor::clearColor();
    unsafe {
        let _: () = msg_send![&window, setOpaque: Bool::NO];
        let _: () = msg_send![&window, setBackgroundColor: &*clear];
        let _: () = msg_send![&window, setHasShadow: Bool::NO];
        let _: () = msg_send![&window, setIgnoresMouseEvents: Bool::YES]; // 默认点击穿透
        let _: () = msg_send![&window, setMovableByWindowBackground: Bool::YES]; // 关穿透时可拖
        let _: () = msg_send![&window, setLevel: 3i64]; // NSFloatingWindowLevel
        let _: () = msg_send![&window, setCollectionBehavior: 1u64]; // canJoinAllSpaces
        let _: () = msg_send![&window, setReleasedWhenClosed: Bool::NO];
    }

    let dot = dot_size as CGFloat;
    let view = PillView::new(
        nscolor(Color::Purple),
        NSRect::new(NSPoint::new(0.0, 0.0), frame.size),
        dot,
    );
    let _: () = unsafe { msg_send![&view, setWantsLayer: Bool::YES] };
    let _: () = unsafe { msg_send![&window, setContentView: &*view] };
    let _: () = unsafe { msg_send![&window, orderFrontRegardless] };
    (window, view)
}

/// 切换浮窗是否点击穿透。on=true → 忽略鼠标(穿透);on=false → 接收鼠标,可拖动。
pub fn set_click_through(window: &NSWindow, on: bool) {
    let _: () = unsafe { msg_send![window, setIgnoresMouseEvents: Bool::new(on)] };
}

/// 改圆点大小:更新 dot、拆掉按旧尺寸建的波纹环(下次 set_light 重建)、重绘。
pub fn set_size(view: &PillView, dot_size: u32) {
    {
        let mut st = view.ivars().borrow_mut();
        st.dot = dot_size as CGFloat;
        for ring in st.rings.drain(..) {
            let _: () = unsafe { msg_send![&*ring, removeFromSuperview] };
        }
    }
    let _: () = unsafe { msg_send![view, setNeedsDisplay: Bool::YES] };
}

// ---- 按灯效更新颜色 + 动画 ----
pub fn set_light(view: &PillView, anim: LightAnim) {
    view.rust_set_color(nscolor(anim_color(anim)));

    let layer: Retained<CALayer> = unsafe { msg_send![view, layer] };
    // 先清掉旧的:opacity 动画 + 波纹环子视图。
    let _: () = unsafe { msg_send![&layer, removeAnimationForKey: &*NSString::from_str("pulse")] };
    let _: () = unsafe { msg_send![&layer, setOpacity: 1.0f32] };
    {
        let mut st = view.ivars().borrow_mut();
        for ring in st.rings.drain(..) {
            let _: () = unsafe { msg_send![&*ring, removeFromSuperview] };
        }
    }

    match anim {
        LightAnim::Pulse { period_ms, .. } => add_pulse(&layer, period_ms),
        LightAnim::Ripple { color, period_ms } => add_ripple(view, color, period_ms),
        LightAnim::Steady { .. } => {}
    }
}

impl PillView {
    fn rust_set_color(&self, color: Retained<NSColor>) {
        self.ivars().borrow_mut().color = color;
        let _: () = unsafe { msg_send![self, setNeedsDisplay: Bool::YES] };
    }
}

/// 呼吸:opacity 在 [0.2, 1.0] 间往复。周期越短视觉上越「闪」(快闪/慢闪/呼吸)。
fn add_pulse(layer: &CALayer, period_ms: u32) {
    const FLOOR: f64 = 0.2;
    let basic: Retained<CABasicAnimation> = unsafe {
        msg_send![class!(CABasicAnimation), animationWithKeyPath: &*NSString::from_str("opacity")]
    };
    let from_n: Retained<NSNumber> =
        unsafe { msg_send![class!(NSNumber), numberWithDouble: FLOOR] };
    let to_n: Retained<NSNumber> = unsafe { msg_send![class!(NSNumber), numberWithDouble: 1.0f64] };
    // autoreverses 下 duration 是半周期;period_ms 为完整周期。
    let duration = period_ms as f64 / 1000.0 / 2.0;
    unsafe {
        let _: () = msg_send![&basic, setFromValue: &*from_n];
        let _: () = msg_send![&basic, setToValue: &*to_n];
        let _: () = msg_send![&basic, setDuration: duration];
        let _: () = msg_send![&basic, setAutoreverses: Bool::YES];
        let _: () = msg_send![&basic, setRepeatCount: f32::INFINITY];
        let _: () = msg_send![layer, addAnimation: &*basic, forKey: &*NSString::from_str("pulse")];
    }
}

/// 波纹环数量。两环错相半个周期 → 视觉上连续扩散。
const RIPPLE_RINGS: usize = 2;

/// 波纹:N 个自绘环子视图错相扩散。每个环缩放从 1.0 扩到 MAX_SCALE、opacity 从 0.85 淡到 0,
/// 单向循环(末尾近乎透明,故回弹不可见,视觉上即连续扩散)。多环用 timeOffset 错开相位,
/// 环以更密节奏接连出现。
///
/// 居中关键:layer-backed NSView 的 anchorPoint/position 由 AppKit 托管、运行时改会被
/// 重置(故早先「改 anchorPoint 到中心」无效,环仍从左下角缩放、圆心偏离圆点)。这里
/// 不动锚点,改用一个「绕环自身圆心缩放」的 CATransform3D 作动画
/// (translate(+c)·scale·translate(-c)),无论 anchorPoint 在哪,环都在缩放时圆心始终
/// 对齐圆点圆心,对称向外扩散。
fn add_ripple(view: &PillView, color: Color, period_ms: u32) {
    let dot = view.ivars().borrow().dot;
    let o = dot_origin(dot);
    let ring_frame = NSRect::new(NSPoint::new(o, o), NSSize::new(dot, dot));
    let duration = period_ms as f64 / 1000.0;

    // 环视图自身坐标里的圆心 = (dot/2, dot/2)(环描边内切于 dot×dot bounds)。
    let c = dot / 2.0;
    let from_t = scale_about(c, c, 1.0);
    let to_t = scale_about(c, c, MAX_SCALE);

    let mut rings = Vec::with_capacity(RIPPLE_RINGS);
    for i in 0..RIPPLE_RINGS {
        let ring = RingView::new(nscolor(color), ring_frame);
        unsafe {
            let _: () = msg_send![&ring, setWantsLayer: Bool::YES];
            let _: () = msg_send![view, addSubview: &*ring];
        }
        let layer: Retained<CALayer> = unsafe { msg_send![&ring, layer] };
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
    let scale: Retained<CABasicAnimation> = unsafe {
        msg_send![class!(CABasicAnimation), animationWithKeyPath: &*NSString::from_str("transform")]
    };
    let opacity: Retained<CABasicAnimation> = unsafe {
        msg_send![class!(CABasicAnimation), animationWithKeyPath: &*NSString::from_str("opacity")]
    };
    unsafe {
        let from_v: Retained<NSValue> = NSValue::valueWithCATransform3D(from_t);
        let to_v: Retained<NSValue> = NSValue::valueWithCATransform3D(to_t);
        let _: () = msg_send![&scale, setFromValue: &*from_v];
        let _: () = msg_send![&scale, setToValue: &*to_v];
        let _: () = msg_send![&scale, setDuration: duration];
        let _: () = msg_send![&scale, setTimeOffset: phase];
        let _: () = msg_send![&scale, setRepeatCount: f32::INFINITY];
        let _: () =
            msg_send![layer, addAnimation: &*scale, forKey: &*NSString::from_str("rippleScale")];

        let from2: Retained<NSNumber> = msg_send![class!(NSNumber), numberWithDouble: 0.85f64];
        let to2: Retained<NSNumber> = msg_send![class!(NSNumber), numberWithDouble: 0.0f64];
        let _: () = msg_send![&opacity, setFromValue: &*from2];
        let _: () = msg_send![&opacity, setToValue: &*to2];
        let _: () = msg_send![&opacity, setDuration: duration];
        let _: () = msg_send![&opacity, setTimeOffset: phase];
        let _: () = msg_send![&opacity, setRepeatCount: f32::INFINITY];
        let _: () = msg_send![layer, addAnimation: &*opacity, forKey: &*NSString::from_str("rippleOpacity")];
    }
}

/// 波纹环最大缩放倍数(扩到 2.6× 圆点直径,仍在 120px 窗口内不裁切)。
const MAX_SCALE: CGFloat = 2.6;

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
