//! Phase 2:全局置顶、透明、默认点击穿透的"药丸"浮窗 + CoreAnimation 灯效。
//!
//! 渲染:自绘 NSView(NSBezierPath 圆角 + NSColor 填充)——绕开 CALayer 的 CGColor 依赖。
//! 灯效(全部交 render server 进程驱动 GPU 插值,本进程 CPU ~0%):
//!   - Steady 常亮 / Pulse 呼吸 / Blink 明灭:动 layer "opacity";
//!   - Ripple 波纹:一个自绘环子视图,动其 layer "transform.scale" + "opacity",
//!     从中心扩散并淡出(环也自绘,故无需 CGColor)。
//!
//! 窗口比核心圆点大一圈(48×48 内放 16×16 圆点),给波纹环留出扩散空间。

use std::cell::RefCell;

use agent_light_core::{Color, LightAnim};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::Bool;
use objc2::{class, declare_class, msg_send, msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_app_kit::{NSBezierPath, NSColor, NSView, NSWindow};
use objc2_foundation::{CGFloat, NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{CABasicAnimation, CALayer};

// 窗口与核心圆点的几何。
const WIN: CGFloat = 48.0;
const DOT: CGFloat = 16.0;
const DOT_ORIGIN: CGFloat = (WIN - DOT) / 2.0; // 圆点左下角,使其居中

// ---- 灯效规格(供 animation 模块 / 后续 Phase 用)----
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct CoreAnimSpec {
    pub color: Color,
    pub kind: AnimKind,
    pub period_ms: u32,
}
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum AnimKind {
    Steady,
    Pulse,
    Blink,
    Ripple,
    Gif,
}
impl CoreAnimSpec {
    pub fn from(a: LightAnim) -> Self {
        match a {
            LightAnim::Steady { color } => Self { color, kind: AnimKind::Steady, period_ms: 0 },
            LightAnim::Pulse { color, period_ms } => Self { color, kind: AnimKind::Pulse, period_ms },
            LightAnim::Blink { color, period_ms } => Self { color, kind: AnimKind::Blink, period_ms },
            LightAnim::Ripple { color, period_ms } => Self { color, kind: AnimKind::Ripple, period_ms },
        }
    }
}

// ---- Color -> NSColor ----
pub fn nscolor(c: Color) -> Retained<NSColor> {
    let (r, g, b): (CGFloat, CGFloat, CGFloat) = match c {
        Color::Green => (0.20, 0.80, 0.30),     // Done
        Color::DarkGreen => (0.02, 0.45, 0.20), // Done Notification(深绿)
        Color::Yellow => (0.95, 0.80, 0.15),    // Working
        Color::Amber => (0.95, 0.55, 0.10),     // NeedsDeci(橙)
        Color::Red => (0.92, 0.22, 0.22),       // Error
        Color::Purple => (0.62, 0.36, 0.90),    // Offline
    };
    unsafe { NSColor::colorWithCalibratedRed_green_blue_alpha(r, g, b, 1.0) }
}

fn anim_color(a: LightAnim) -> Color {
    match a {
        LightAnim::Steady { color } => color,
        LightAnim::Pulse { color, .. } => color,
        LightAnim::Blink { color, .. } => color,
        LightAnim::Ripple { color, .. } => color,
    }
}

// ---- PillView:自绘圆点 + 持有可选的波纹环 ----
pub struct PillState {
    pub color: Retained<NSColor>,
    pub ring: Option<Retained<RingView>>,
}

declare_class!(
    pub struct PillView;

    unsafe impl ClassType for PillView {
        type Super = NSView;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "PillView";
    }

    impl DeclaredClass for PillView {
        type Ivars = RefCell<PillState>;
    }

    #[allow(non_snake_case)]
    unsafe impl PillView {
        /// 允许点击药丸拖动无边框窗口(配合 window movableByWindowBackground)。
        /// 仅在「关闭点击穿透」时窗口才接收鼠标事件,故只在那时生效。
        #[method(mouseDownCanMoveWindow)]
        fn mouse_down_can_move_window(&self) -> Bool {
            Bool::YES
        }

        #[method(drawRect:)]
        fn draw_rect(&self, _dirty: NSRect) {
            let b = self.ivars().borrow();
            let color: &NSColor = &*b.color;
            let dot = NSRect::new(NSPoint::new(DOT_ORIGIN, DOT_ORIGIN), NSSize::new(DOT, DOT));
            let radius = DOT / 2.0;
            let path: Retained<NSBezierPath> = unsafe {
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(dot, radius, radius)
            };
            let _: () = unsafe { msg_send![color, set] };
            unsafe { path.fill() };
        }
    }
);

impl PillView {
    fn new(color: Retained<NSColor>, frame: NSRect) -> Retained<Self> {
        let allocated: Allocated<Self> = unsafe { msg_send_id![Self::class(), alloc] };
        let partial = allocated.set_ivars(RefCell::new(PillState { color, ring: None }));
        unsafe { msg_send_id![super(partial), initWithFrame: frame] }
    }

    fn rust_set_color(&self, color: Retained<NSColor>) {
        self.ivars().borrow_mut().color = color;
        let _: () = unsafe { msg_send![self, setNeedsDisplay: Bool::YES] };
    }
}

// ---- RingView:波纹环(自绘描边圆,故无需 CGColor)----
declare_class!(
    pub struct RingView;

    unsafe impl ClassType for RingView {
        type Super = NSView;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "RingView";
    }

    impl DeclaredClass for RingView {
        type Ivars = RefCell<Retained<NSColor>>;
    }

    #[allow(non_snake_case)]
    unsafe impl RingView {
        #[method(drawRect:)]
        fn draw_rect(&self, _dirty: NSRect) {
            let b = self.ivars().borrow();
            let color: &NSColor = &*b;
            let bounds: NSRect = unsafe { msg_send![self, bounds] };
            let lw: CGFloat = 1.5;
            let inset = NSRect::new(
                NSPoint::new(lw / 2.0, lw / 2.0),
                NSSize::new(bounds.size.width - lw, bounds.size.height - lw),
            );
            let r = inset.size.height / 2.0;
            let path: Retained<NSBezierPath> = unsafe {
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(inset, r, r)
            };
            let _: () = unsafe { msg_send![&path, setLineWidth: lw] };
            let _: () = unsafe { msg_send![color, set] };
            let _: () = unsafe { msg_send![&path, stroke] };
        }
    }
);

impl RingView {
    fn new(color: Retained<NSColor>, frame: NSRect) -> Retained<Self> {
        let allocated: Allocated<Self> = unsafe { msg_send_id![Self::class(), alloc] };
        let partial = allocated.set_ivars(RefCell::new(color));
        unsafe { msg_send_id![super(partial), initWithFrame: frame] }
    }
}

// ---- 构建浮窗 ----
pub fn build() -> (Retained<NSWindow>, Retained<PillView>) {
    let frame = NSRect::new(NSPoint::new(220.0, 820.0), NSSize::new(WIN, WIN));

    let alloc: Allocated<NSWindow> = unsafe { msg_send_id![class!(NSWindow), alloc] };
    let window: Retained<NSWindow> = unsafe {
        msg_send_id![
            alloc,
            initWithContentRect: frame,
            styleMask: 0u64, // NSWindowStyleMaskBorderless
            backing: 2u64,   // NSBackingStoreBuffered
            defer: Bool::NO,
        ]
    };

    let clear = unsafe { NSColor::clearColor() };
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

    let view = PillView::new(nscolor(Color::Purple), NSRect::new(NSPoint::new(0.0, 0.0), frame.size));
    let _: () = unsafe { msg_send![&view, setWantsLayer: Bool::YES] };
    let _: () = unsafe { msg_send![&window, setContentView: &*view] };
    let _: () = unsafe { msg_send![&window, orderFrontRegardless] };
    (window, view)
}

/// 切换浮窗是否点击穿透。on=true → 忽略鼠标(穿透);on=false → 接收鼠标,可拖动。
pub fn set_click_through(window: &NSWindow, on: bool) {
    let _: () = unsafe { msg_send![window, setIgnoresMouseEvents: Bool::new(on)] };
}

// ---- 按灯效更新颜色 + 动画 ----
pub fn set_light(view: &PillView, anim: LightAnim) {
    view.rust_set_color(nscolor(anim_color(anim)));

    let layer: Retained<CALayer> = unsafe { msg_send_id![view, layer] };
    // 先清掉旧的:opacity 动画 + 波纹环子视图。
    let _: () = unsafe { msg_send![&layer, removeAnimationForKey: &*NSString::from_str("pulse")] };
    let _: () = unsafe { msg_send![&layer, setOpacity: 1.0f32] };
    {
        let mut st = view.ivars().borrow_mut();
        if let Some(ring) = st.ring.take() {
            let _: () = unsafe { msg_send![&*ring, removeFromSuperview] };
        }
    }

    match anim {
        LightAnim::Blink { period_ms, .. } => add_pulse(&layer, 0.0, period_ms),
        LightAnim::Pulse { period_ms, .. } => add_pulse(&layer, 0.4, period_ms),
        LightAnim::Ripple { color, period_ms } => add_ripple(view, color, period_ms),
        LightAnim::Steady { .. } => {}
    }
}

/// opacity 在 [from, 1.0] 间往复。from=0 → 明灭(Blink);from=0.4 → 呼吸(Pulse)。
fn add_pulse(layer: &CALayer, from: f64, period_ms: u32) {
    let basic: Retained<CABasicAnimation> = unsafe {
        msg_send_id![class!(CABasicAnimation), animationWithKeyPath: &*NSString::from_str("opacity")]
    };
    let from_n: Retained<NSNumber> = unsafe { msg_send_id![class!(NSNumber), numberWithDouble: from] };
    let to_n: Retained<NSNumber> = unsafe { msg_send_id![class!(NSNumber), numberWithDouble: 1.0f64] };
    // autoreverses 下 duration 是半周期;period_ms 为完整周期。
    let duration = period_ms as f64 / 1000.0 / 2.0;
    unsafe {
        let _: () = msg_send![&basic, setFromValue: &*from_n];
        let _: () = msg_send![&basic, setToValue: &*to_n];
        let _: () = msg_send![&basic, setDuration: duration];
        let _: () = msg_send![&basic, setAutoreverses: Bool::YES];
        let _: () = msg_send![&basic, setRepeatCount: f32::INFINITY];
        let _: () = msg_send![layer, addAnimation: &*basic forKey: &*NSString::from_str("pulse")];
    }
}

/// 波纹:一个自绘环子视图,transform.scale 从 1.0 扩到 2.6、opacity 从 0.85 淡到 0,
/// 单向循环(末尾近乎透明,故回弹不可见,视觉上即连续扩散的环)。
fn add_ripple(view: &PillView, color: Color, period_ms: u32) {
    let ring_frame = NSRect::new(NSPoint::new(DOT_ORIGIN, DOT_ORIGIN), NSSize::new(DOT, DOT));
    let ring = RingView::new(nscolor(color), ring_frame);
    unsafe {
        let _: () = msg_send![&ring, setWantsLayer: Bool::YES];
        let _: () = msg_send![view, addSubview: &*ring];
    }
    let layer: Retained<CALayer> = unsafe { msg_send_id![&ring, layer] };
    let duration = period_ms as f64 / 1000.0;

    let scale: Retained<CABasicAnimation> = unsafe {
        msg_send_id![class!(CABasicAnimation), animationWithKeyPath: &*NSString::from_str("transform.scale")]
    };
    let opacity: Retained<CABasicAnimation> = unsafe {
        msg_send_id![class!(CABasicAnimation), animationWithKeyPath: &*NSString::from_str("opacity")]
    };
    unsafe {
        let from1: Retained<NSNumber> = msg_send_id![class!(NSNumber), numberWithDouble: 1.0f64];
        let to1: Retained<NSNumber> = msg_send_id![class!(NSNumber), numberWithDouble: 2.6f64];
        let _: () = msg_send![&scale, setFromValue: &*from1];
        let _: () = msg_send![&scale, setToValue: &*to1];
        let _: () = msg_send![&scale, setDuration: duration];
        let _: () = msg_send![&scale, setRepeatCount: f32::INFINITY];
        let _: () = msg_send![&layer, addAnimation: &*scale forKey: &*NSString::from_str("rippleScale")];

        let from2: Retained<NSNumber> = msg_send_id![class!(NSNumber), numberWithDouble: 0.85f64];
        let to2: Retained<NSNumber> = msg_send_id![class!(NSNumber), numberWithDouble: 0.0f64];
        let _: () = msg_send![&opacity, setFromValue: &*from2];
        let _: () = msg_send![&opacity, setToValue: &*to2];
        let _: () = msg_send![&opacity, setDuration: duration];
        let _: () = msg_send![&opacity, setRepeatCount: f32::INFINITY];
        let _: () = msg_send![&layer, addAnimation: &*opacity forKey: &*NSString::from_str("rippleOpacity")];
    }

    view.ivars().borrow_mut().ring = Some(ring);
}
