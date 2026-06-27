# CLAUDE.md — Asig 的 agent 操作手册

> 只放「怎么在 Asig 里干活」的操作要点和踩过的坑。**设计与规格看 [DEV.md](./DEV.md)**,语义冲突以 DEV.md 为准。文件级架构看 DEV.md「Code Map」。

Asig = macOS 多 Agent 状态监控灯(菜单栏灯 + 全局置顶浮窗 + 弹窗/设置面板)。Rust workspace:`crates/core`(可移植内核,零 AppKit)+ `crates/app`(objc2/AppKit 壳)。

## 构建 / 运行 / 测试
- `cargo run -p agent-light` / `cargo build --release -p agent-light`(release:`opt-level=z`+LTO+strip,契合 <60MB/<1% CPU)。
- 内核单测(纯 Rust,最快):`cargo test -p agent-light-core`。全量(含 AppKit 编译):`cargo test -p agent-light`。
- 打包 `.app`:`./scripts/make-app.sh` → `build/Asig.app`。
- **坑①:源码改动不会自动到运行中的 app** —— 必须 `make-app.sh`(重编 + 拷进 bundle)**再重启进程**才生效。改完代码看到「没变化」,先想这一条。
- **坑②:非交互 shell 里 `cargo` 不在 PATH** —— 先 `source ~/.cargo/env` 再调 cargo。

## objc2 生态(改前必读)
- 当前在**最新发布线**:`objc2` 0.6 + `-foundation`/`-app-kit`/`-quartz-core` 0.3。升版前先看 objc2 CHANGELOG —— 宏和 API 会漂(从 0.5 `declare_class!` 升 0.6 `define_class!` 就是大改)。
- 用 **0.6 的 `define_class!` 宏**(属性式:`#[unsafe(super(...))]`、`#[thread_kind = MainThreadOnly]`、`#[name = "..."]`、`#[ivars = ...]`;方法标注 `#[unsafe(method(sel:))]` / `#[unsafe(method_id(sel:))]`)。新增 ObjC 类照着 `AppDelegate` / `PillView` / `KeyPanel` 抄。`.ivars()` 来自 `DefinedClass` trait(用到它的文件都要 import)。
- **类型/协议默认藏在 cargo feature 后**:`Cargo.toml` 按需开(如 `NSWindow`、`NSScreen`、`CATransform3D`)。编译报某类型 "not found" → 八成是漏开 feature。
- `msg_send!` 在 0.6 统管对象/基本类型返回(`msg_send_id!` 已废弃)。多参数选择子参数间要**逗号**:`addAnimation:x, forKey:y`。
- 0.6 起 **`CGFloat` / CG 类型搬到 `objc2-core-foundation`**(已加为依赖);`NSRect`/`NSPoint`/`NSSize` 仍在 `objc2-foundation`(NSGeometry feature)。框架自带方法(如 `NSBezierPath::...`、`path.fill()`)在 0.6 多为**安全**调用,别再套 `unsafe {}`(clippy 会报 `unused_unsafe`)。

## macOS / AppKit 坑(都踩过)
- **layer-backed NSView 的 `anchorPoint`/`position` 由 AppKit 托管,运行时改会被重置**。要「绕中心缩放」别动 anchorPoint,改用绕圆心的 `CATransform3D`(`overlay::scale_about`)做 `transform` 动画。波纹居中就是这么修的(曾因改 anchorPoint 无效、环偏到圆点左下角)。
- **合成/程序化点击打不到菜单栏 `NSStatusItem`**,也触发不了真失焦 → 「点别处自动关」「菜单栏点击」这类只能真人交互验证。
- 别在运行时对已显示的窗口乱发 `setFrame:` 等结构体消息(曾因 KVO setFrame 崩)。改浮窗位置走持久化的 `light_pos`。
- 视觉改动(灯效/布局/颜色)**尽量像素级实测**,别只靠目测或图像分析器(分析器对小元素常看走眼 —— 这次波纹就误判过一次)。

## 开发 / 截图钩子(仅 dev,生产不设这些环境变量)
- `ASIG_PANEL=1`:启动 0.5s 后自动开 Drop-down Panel。
- `ASIG_SETTINGS=1`:自动开 Settings Panel。
- `ASIG_NO_HIDE=1`:关掉 Drop-down 的「失焦自动关」,便于截图。
- 用法:`ASIG_SETTINGS=1 ./build/Asig.app/Contents/MacOS/agent-light`(`open` 不透传 env)。

## 供应链检查(cargo-deny)
- 配置在 `deny.toml`:许可证白名单(只放行宽松许可证 + MPL-2.0)+ RustSec 漏洞 + 禁用/重复版本/来源。
- CI 里跑 `cargo deny check`(全量,联网拉 RustSec 库)。**cargo-deny 的 advisories 已覆盖 cargo-audit 的职责**(同源),所以没再单跑 cargo-audit。
- 本地:`cargo install cargo-deny` 后 `cargo deny check`;若拉不到 RustSec 库(无网),可只跑不需联网的部分:`cargo deny check licenses bans sources`。
- 加新依赖后若 license 被拒:看报错的 SPDX id,确认是宽松许可证就加进 `deny.toml` 的 `[licenses] allow`(并写明理由);是强 copyleft(GPL/AGPL)则不要引入。

## 治理
- **DEV.md 是唯一权威手册,未经用户明确允许不可改**;README 次之,以 DEV 为准。改了设计要同步 README。
- Asig 自身设置在 `~/Library/Application Support/Asig/settings.json`(serde,`StyleKey` 作键,缺省回退内置默认)。向后兼容字段改动要带 serde alias / `#[serde(default)]`。
- **提交规则**:未经用户同意不 `git commit` / `git push`;提交信息**不带任何 Claude / Co-Authored-By 字样**(只留 koki)。
