# CLAUDE.md — Asig 操作手册

> 怎么在 Asig 干活 + 踩坑方向。**设计/规格看 [DEV.md](./DEV.md)**(语义冲突以其为准,文件级架构看其「Code Map」);故障排查看 [FIX.md](./FIX.md)。

Asig = macOS 多 Agent 状态监控灯(菜单栏灯 + 全局置顶浮窗 + 弹窗/设置面板)。Rust workspace:`crates/core`(可移植内核,零 AppKit)+ `crates/app`(objc2/AppKit 壳)。

## 构建 / 运行 / 测试

- `cargo run -p agent-light` / `cargo build --release -p agent-light`(release:`opt-level=z`+LTO+strip,契合 <60MB/<1% CPU)。
- 内核单测(最快):`cargo test -p agent-light-core`;全量(含 AppKit):`cargo test -p agent-light`。
- 打包:`./scripts/make-app.sh` → `build/Asig.app`。
- **坑①:源码改动不会自动到运行中的 app** —— 必须 `make-app.sh`(重编 + 拷进 bundle)**再重启进程**才生效。看到「没变化」先想这一条。
- **坑②:非交互 shell 里 `cargo` 不在 PATH** —— 先 `source ~/.cargo/env`。

## objc2 / AppKit 约定(改 AppKit 代码必读)

- 最新发布线 `objc2` 0.6 + `-foundation`/`-app-kit`/`-quartz-core` 0.3;升版先看 objc2 CHANGELOG(宏/API 会漂,0.5 `declare_class!` → 0.6 `define_class!` 就是大改)。
- 用 0.6 `define_class!` 宏(属性式:`#[unsafe(super(...))]`/`#[thread_kind = MainThreadOnly]`/`#[name=]`/`#[ivars=]`;方法 `#[unsafe(method(sel:))]`)。新增 ObjC 类照着 `AppDelegate`/`PillView`/`RingView` 抄;`.ivars()` 来自 `DefinedClass`(用到就 import)。
- 类型/协议藏在 cargo feature 后;编译报某类型 "not found" → 八成漏开 feature(如 `NSWindow`/`NSScreen`/`CATransform3D`)。
- `msg_send!` 统管对象/基本类型返回(`msg_send_id!` 已废弃);多参数选择子参数间用**逗号**(`addAnimation:x, forKey:y`)。
- 0.6 起 `CGFloat`/CG 类型搬到 `objc2-core-foundation`;`NSRect`/`NSPoint`/`NSSize` 在 `objc2-foundation`(NSGeometry feature)。框架自带方法(`NSBezierPath::...`/`path.fill()`)多**安全**,别再套 `unsafe {}`(clippy 报 `unused_unsafe`)。
- 纯 `NSView` 只有 `tag()`/`viewWithTag:`,没 `setTag:`(打 tag 会 debug panic);`NSControl` 子类(`NSSlider`/`NSTextField`/`NSPopUpButton`/`NSButton`)才有。纯 `NSView`(如设置窗 pane)按 `Vec` 索引切、别打 tag。
- 各 agent 状态判定(NeedsDeci/Working/Done 等)见 DEV.md「Code Map」各 source 条目(claude.rs / openclaw / hermes),统一走内核 sticky `transition`。

## 陷阱方向(遇到这类问题往哪想)

- **accessory app 的窗口/菜单**:涉及可切换窗口 / 标准快捷键(⌘W/⌘Q)→ 先想 activation policy(`.regular` 才进 Cmd+Tab/Dock)+ 主菜单(快捷键靠菜单项提供)。运行时 accessory↔regular 切换的 Cmd+Tab 排序末尾是 WindowServer MRU,无 API 可控(非 bug)。
- **pid 探测权限**:`kill(pid,0)` 对系统/非同用户进程(launchd pid 1)返回 EPERM → 误判不活。gateway pid 探测用同用户活进程;测试构造 pid 别用 1。
- **合成操作打不到的目标**:菜单栏 `NSStatusItem` 点击、accessory 窗口关闭(⌘W/AX)—— 合成点击/keystroke 打不到 → 靠逻辑论证(delegate 链路 + 对称 API)+ 真人验证,别指望脚本复现。
- **layer-backed NSView 的 `anchorPoint`/`position` 由 AppKit 托管**,运行时改会被重置 → 绕中心缩放改用 `CATransform3D`(别动 anchorPoint)。
- **运行时对已显示窗口发 `setFrame:` 等结构体消息**可能触发 KVO 崩 → 改浮窗位置走持久化 `light_pos`。
- **状态判定不靠 UI**:emoji/小元素图像分析器常误判(相近色 🟡/🟠、popover 多行截断)→ 以代码逻辑/单测/probe 输出为准,视觉只做布局几何确认。
- **截图玻璃/材质**:全屏 `screencapture` 再按窗口 bounds 裁窗;单窗口 `-l<wid>` 不合桌面背景,液态玻璃/vibrancy 退化纯色(只能看几何)。
- **配置向后兼容**:新 `AgentKind` 不自动进老用户 `enabled_agents`(serde default 只对缺失 key 触发)→ 加新配置项要决定老用户手动启用还是迁移。

## 开发钩子(仅 dev,生产不设这些环境变量)

- `ASIG_PANEL=1`:启动 0.5s 后自动开 Drop-down Panel。
- `ASIG_SETTINGS=1`:自动开 Settings Panel。
- `ASIG_NO_HIDE=1`:关掉 Drop-down 的「失焦自动关」,便于截图。
- `ASIG_TAB=<1..=7>`:直接开到指定 pane(1=DoneNotif/2=Done/3=Working/4=NeedsDeci/5=Error/6=Offline/7=About;不设=General)。
- `ASIG_PREVIEW=1`:跳过轮询,循环展示各状态默认灯效(便于动画截图)。
- `ASIG_HERMES_ROOT=<dir>`:HermesSource 指向测试用 hermes 目录(构造小 `state.db` + `gateway_state.json` 做端到端三态验证,见 `hermes/tests.rs` 的 `#[ignore] probe_env`)。
- 用法:`ASIG_SETTINGS=1 ./build/Asig.app/Contents/MacOS/agent-light`(`open` 不透传 env)。

## 供应链检查(cargo-deny)

- 配置在 `deny.toml`:许可证白名单(宽松许可证 + MPL-2.0)+ RustSec 漏洞 + 重复版本/来源(仅警告)。
- CI 跑 `cargo deny check`(全量,联网拉 RustSec);cargo-deny advisories 已覆盖 cargo-audit,没再单跑。
- 本地无网:`cargo deny check licenses bans sources`。加新依赖 license 被拒 → 确认是宽松许可证就加进 `[licenses] allow`(写理由);强 copyleft(GPL/AGPL)不引入。

## 治理

- **完成必自测**:`cargo fmt --all -- --check` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace`;UI/运行时改动还要 `make-app.sh` + 全屏截图按窗口 bounds 裁窗(别用 `-l<wid>`,别只靠目测/分析器)+ `open build/Asig.app` 启动最新版核对。
- **DEV.md 是唯一权威手册,未经用户明确允许不可改**:未明确先问;与代码脱节给修改方案并问用户,不自作主张改 DEV.md 或代码。
- 设置在 `~/Library/Application Support/Asig/settings.json`(serde,`StyleKey` 作键,缺省回退内置默认);字段改动带 serde alias / `#[serde(default)]`。
- **提交规则**:`commit`/`push` 须用户明确指令(`add` 可自行);提交信息全英文、正文 ≤8 行、不带测试结果、**不带 Claude / Co-Authored-By**。
- README.md:DEV.md 的简化版,面向用户。
