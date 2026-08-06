//! 颜色 + 外观工具(原 overlay.rs 抽出:`nscolor`/`swatch_image` 等被 overlay / settings /
//! tray / app_delegate 跨用,独立成模块解耦,overlay 回归纯浮窗职责)。
//!
//! - 动态 / 静态 NSColor 构造、swatch 位图栅格化、Theme 应用、外观 / 无障碍偏好读取。

use std::ptr::NonNull;

use agent_light_core::{Color, Theme};
use block2::RcBlock;
use objc2::rc::{Allocated, Retained, autoreleasepool};
use objc2::{MainThreadMarker, class, msg_send};
use objc2_app_kit::{NSAppearance, NSApplication, NSBezierPath, NSColor, NSImage, NSWorkspace};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

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
            // block 返回约定 +1 retained:into_raw 转移所有权给调用方,不释放。
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
