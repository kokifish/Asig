# Asig 设计与开发

> Asig 简洁的、最权威的开发维护手册，语义冲突时以本文档为准，包括README.md和源码。
> 没有明确允许，Agent 不可修改本文档。

- Troubleshooting: 通用故障排查与修复经验沉淀在 [FIX.md](./FIX.md)。

Asig = macOS 多 Agent 状态监控灯。菜单栏灯 + 全局置顶动态药丸浮窗。
监控 Claude Code / CodeBuddy / OpenClaw，Trae 待支持。

## Principals

- 结构清晰，逻辑简单，高内聚低耦合，提高代码复用率，降低总代码量和行数
- 不过度设计，避免不必要的薄封装
- 保持组件、工具链、依赖等保持release版本最新，非必要不兼容旧版
- 在保持美观、功能符合要求的前提下，尽可能降低 CPU、Memory 占用

## Tech Overview

- **内核**: Rust workspace `crates/core`，零 AppKit 依赖 — 可移植，Windows 壳可直接复用
- **UI 壳**: objc2 / AppKit 纯 Rust，无 WebView — 常驻灯 <60MB，CoreAnimation 交 render server，CPU ~0%
- **跨平台**: 暂只 macOS，留口子（内核可移植，UI 壳按平台另写）

### Code Map

文件级架构（一句话/文件）。分层：内核 `crates/core`（可移植，零 AppKit）→ UI 壳 `crates/app`（objc2/AppKit），壳只调 `Monitor::poll()` 拿 `Snapshot` 驱动灯。

**内核 `crates/core`：**

- `source.rs` — `AgentSource` trait + `AgentSession` / `AgentKind`（每个工具实现一个 source）
- `claude.rs` — `ClaudeLikeSource`：Claude / CodeBuddy 共用（参数化根目录）；读 session 文件 + pid 存活判定做 Offline 检测；busy 时读 transcript 尾部 `stop_reason`（`end_turn`→NeedsDeci / `tool_use`→Working）做可靠的待决策检测
- `openclaw.rs` — `OpenClawSource`：SQLite 状态库（Phase 3 补全，当前占位）
- `aggregate.rs` — `global_status()`：N 个会话压成最高优先级的全局灯态
- `status.rs` — `AgentStatus` + `Color` + `LightAnim` + sticky 状态机 `transition()` + `AgentStatus::light()`（默认灯效的单一事实源）
- `config.rs` — `Settings` / `StyleKey` / `StateStyle` / `LightPosition`：可配置灯效 + 浮窗位置，serde 持久化
- `lib.rs` — `Monitor`（轮询编排 → `Snapshot`，含 DoneNotif 边沿检测）

**UI 壳 `crates/app`（objc2/AppKit，纯 Rust，无 WebView）：**

- `main.rs` — 入口：加载设置 → 建浮窗 → 建 `AppDelegate` → 状态栏 + tick 定时器
- `app_delegate.rs` — `AppDelegate`（declare_class）：tick 轮询 / 渲染分发、popover 与 settings 生命周期、点击穿透、样式改动落盘、浮窗位置记忆的枢纽
- `tray.rs` — 菜单栏 Signal Icon（`NSStatusItem` + 自绘彩色圆点按钮；点击弹 Drop-down）+ tick 定时器
- `overlay.rs` — Signal Light 浮窗：自绘圆点 `PillView` + 波纹环 `RingView` + CoreAnimation 灯效 + 多屏位置几何
- `panel.rs` — Drop-down Panel：圆角卡片 `CardView` + 三按钮（设置/锁定/退出）+ 会话列表；定位在图标左下方
- `settings.rs` — Settings Panel：左侧栏导航（常规 + 各状态 tab + 底部图标行）+ 右侧 pane 切换；状态 pane = 颜色 / 动画 / 速度(Hz)
- `palette.rs` — 颜色→NSColor/中文名、动效中文名、状态 emoji(下拉面板用)映射

## Build and Run

```bash
cargo run -p agent-light                 # 跑(debug)
cargo build --release -p agent-light     # 发布版
cargo build -p agent-light-core          # 只验内核(纯 Rust,快)
```

Performance budget: 运行内存 < 60MB，CPU 平均 < 1%

## Design

- 需要轮询的，默认3s轮询一次

### Signal Color and State Priority

一个 `AgentStatus` 同时决定**灯的颜色 + 灯效(动画)**,UI 层只消费 `status.light()`。

| 优先级 | 状态 | 灯 | 默认动效 | 含义 |
|:---:|---|:---:|---|---|
| 5 | `Error` | 🔴 红 | 快闪 | agent 报错且无法自动恢复 |
| 4 | `NeedsDeci` | 🟠 琥珀 | 慢闪 | 待决策（要权限 / 要输入） |
| 3 | `Offline` | 🟣 紫 | 常亮 | 异常 / 卡住 / 进程没了 / 未知 |
| 2 | `Working` | 🟡 黄 | 呼吸-慢速 | 正在跑 |
| 1 | `Done` | 🟢 绿 | 波纹 | 完成 / 空闲 / 初始默认态 |
| 0 | `DoneNotif` | 🔵 浅蓝 | 快速呼吸 | 其他状态转入Done状态 |

- **Done Notification**: 在别的状态转入`Done`时，默认持续 30s 的 DoneNotif (Done-Notification)，用浅蓝色表示，默认动效为快速呼吸
- **Aggregation**：同一个Agent多个会话同时存在时，全局灯取**优先级最高**的那一个（`AgentStatus::priority()`，数字大者覆盖）。排序：红 > 琥珀 > 紫 > 黄 > 绿。
- **Sticky state**：`NeedsDeci` / `Error` / `Offline` 一旦进入即**锁定**——只有观测到明确的 `Working`（恢复）或 `Done`（结束）才解锁（`transition()`）。不因超时自动清，锁定态之间也**不互相覆盖**（先到先得，避免抖动闪烁）；`Done` / `Working` 可自由接受任意新观测。
- **Animation types**：`Steady`（常亮）/ `Pulse`（呼吸）/ `Ripple`（波纹），共 3 种（详见 [Light Animations](#light-animations)）。**快闪 / 慢闪 / 呼吸都是 `Pulse`，只是周期不同**，无独立的明灭（Blink）动效。全部交 CoreAnimation 在 render server 上跑，app 进程 ~0% CPU。
- **Color enum**：颜色与状态一一对应，定义在内核、平台无关；app 层翻译成具体 RGB

### Light Animations

灯效 = 颜色 + 动画（`LightAnim`）。一个 `AgentStatus` → 一套默认灯效（见上表），用户可在 Settings Panel 覆盖（动效种类 / 颜色 / 周期）。

**全部交 CoreAnimation 在 render server 上驱动 GPU 插值，app 进程 ~0% CPU。**

| 动效 | 英文 | 视觉 | 涉及的属性 |
|---|---|---|---|---|
| 常亮 | Steady | 不变，纯色常亮 | 无周期，period_ms 置 0 |
| 呼吸 | Pulse | 透明度 ~0.2↔1 往复（周期越短越「闪」） | `opacity`，可定义频率 |
| 波纹 | Ripple | 两圈环以圆点为圆心、错相(半周期)对称扩散并淡出 | `transform`（绕圆心缩放的 `CATransform3D`）+ `opacity`（2 个错相 `RingView`），单程一次扩散 |

- Default period：`Error`=350（快闪）/ `NeedsDeci`=1000（慢闪）/ `Working`=1800（呼吸）/ `Done`=1600（波纹）/ `DoneNotif`=450（快速呼吸）。**快闪 / 慢闪 / 呼吸都是 `Pulse`，只是周期不同**（数字越小越快），不是不同动效。
- **Done Notification**：别的态刚转 `Done` 的窗口期内，用 `Pulse`（LightBlue，450ms）覆盖全局态。
- Configurable：Settings 里每状态独立改 动效 + 颜色 + 周期（`StateStyle`）；缺省回退内置 `AgentStatus::light()`。
- Carrier：Signal Light 浮窗——圆点本体做 Steady/Pulse，波纹用两个错相 `RingView` 子视图扩散（动画用绕圆心缩放的 `CATransform3D`——不动 layer-backed 视图会被 AppKit 重置的 `anchorPoint`，故环从圆点对称扩散）；Signal Icon（菜单栏）无动效，只显示自绘彩色圆点（`overlay::swatch_image`，`setTemplate:NO` 保留真彩），不可设动效。
- 速度（周期）以 **Hz** 呈现给用户（`period_ms = 1000 / Hz`）；常亮（Steady）无周期、速度不可设。

### Accessibility（Reduce Motion / Reduce Transparency）

遵循 macOS 无障碍开关（System Settings → Accessibility → Display），读 `NSWorkspace.shared` 的两个布尔：

- **Reduce Motion 开启**：Signal Light 的 `Pulse`/`Ripple` 一律**降级为 `Steady`**（保留颜色、不动）—— 状态仍由颜色区分，只是不再脉冲/扩散，避免对晕动症用户不适。降级在 `overlay::set_light` 入口处据 `reduce_motion_on()` 完成；用户切该开关时，tick 把 `reduce_motion` 并入渲染签名 → 签名变化 → 立即重渲染（无需常驻渲染，不损 CPU）。Signal Icon（菜单栏）本就无动效，不受影响。
- **Reduce Transparency 开启**：Settings/Drop-down 的液态玻璃退化不透明。Drop-down 的 `NSPopover` 由系统自动处理；Settings 在 `glass_pane` 里**跳过 `NSGlassEffectView`**、改用 `NSVisualEffectView`（其在 Reduce Transparency 下自动变实色），保证文字可读（设置窗在(重)开时取最新值）。

### Signal Light

- Def: 在桌面上的可以配置动效、大小的叫 Signal Light
- Default Position: 初始位置在主屏幕的左上角（红黄绿按钮下方一行）。**Position memory**：拖动后记住位置，下次启动自动恢复到上次位置（含所在屏幕，按 `CGDirectDisplayID` 匹配）；若该屏已断开则回退主屏左上角。记忆持久化在 `settings.json` 的 `light_pos` 字段。

### Signal Icon

- Def: 在菜单栏上的，无动效且不可设置动效的叫 Signal Icon

### Drop-down Panel

- Def: 单击菜单栏图标后的弹窗
- Position: 菜单栏单击后在图标右下方弹出菜单栏弹窗，菜单栏弹窗左侧和菜单栏Asig图标左侧对齐，但如果右侧空间不足，则右侧贴屏幕边缘。不可拖动不可自定义大小
- Upper Button: 从左至右分别为`设置`-用于打开 Settings Panel 的最左侧按钮，`锁定`-用于快速设置是否可以拖动圆角单选按钮（与 Settings Panel「浮窗点击穿透」同步同一开关），`退出`-用于退出Asig的最右侧按钮
- 材质：`NSPopover`（SDK 26+ 链接即自动获得液态玻璃，无需手动 vibrancy）。

### Settings Panel

- Def: 点击 Drop-down Panel 的设置按钮后的用于配置显示效果的面板
- Position: 默认在屏幕中央，可以拖动
- Navigation: 左侧栏（顶部 tab 列表 + 底部图标行）+ 右侧 pane 切换。点 tab / 「关于」图标切换右侧 pane。
- 材质：真·液态玻璃（macOS 26+ `NSGlassEffectView`，UI 必须放进其 `contentView`；旧系统回退 `NSVisualEffectView` vibrancy）。窗口 = 一整片主玻璃（透明标题栏，玻璃贯穿顶部）；**左侧栏是浮动玻璃面板**——独立一块 `NSGlassEffectView` 叠在主玻璃上，二次模糊自然更不透明，读作浮于内容之上的圆角玻璃块。刻意**不用** `NSGlassEffectContainerView`：它会合并重叠/相邻的玻璃成一次模糊，反而让浮动侧栏与主玻璃融为一体、失去「浮动」层次。**右侧内容区无外框、标题下无横线**；靠极淡连续圆角卡片（`quaternaryLabelColor`）分组（stats.app 式编排），用层级而非厚重描边区分。
- Content:
  - 右侧内容区有自己的 **header**：标题固定在右侧内容区的左上方（State pane 的 Reset 按钮对齐到该 header 右侧），而不是漂在卡片列中央；标题下方不再有分隔线。
  - General pane: 浮窗大小（滑块）、浮窗点击穿透（勾选；与 Drop-down「锁定」同步同一开关）、轮询间隔（下拉；改完即时重排 tick 定时器）、开机启动（占位，待实现）。详见 General Settings Card。
  - State pane(每状态一个): 颜色（横向单选色块）/ 动画（单选）/ 速度(Hz，`period_ms = 1000/Hz`；常亮时速度禁用)。详见 State Settings Card。
  - About pane: 版本号 + GitHub 链接（纯展示）。
  - 各状态可独立改 动画 + 颜色 + 周期（`StateStyle`）；缺省回退内置 `AgentStatus::light()`。
- Left Side Tabs(in order): General, DoneNotif, Done, Working, NeedsDeci, Error, Offline. Left aligned。状态 tab = 当前色圆点 + 英文简称；其余黑白风（macOS 默认暗色）。
  - Color: 除状态色圆点外，其余均黑白风 / macOS 默认暗色，不用彩色。
  - 选中态：选中 tab = 实心强调色圆角块（`controlAccentColor`，cornerRadius 8，连续圆角 squircle），选中文字转白；状态色圆点保持彩色。**不用文字前缀（无 ▸ 三角形）**，与 stats.app 一致（玻璃/vibrancy 材质的选中态在玻璃侧栏上不可辨，故用实心强调色）。
- Left Side Buttons: 关于(About)、访问官网、调试、捐赠、退出Asig（左→右）。除「关于」外均为占位禁用按钮(留待实现)。
  - Color: 均黑白风 / macOS 默认暗色（单色 SF Symbol 图标），不用彩色。

#### General Settings Card

- Name/名称: General Settings/常规设置
- icon/图标: 常见的齿轮形状的macos纯色图标

> Group不带名称，仅用于分组，以下描述顺序也是卡片内选项的从上至下的顺序

- Group-1:
  - Language/语言: 单行单选列表: English, 中文。默认中文
  - Reset All/重置所有: 按钮，点击后会弹出确认对话框。重制为默认值，包括语言和状态显示的配置，全部自定义内容都恢复为默认值。在该group下居中
- Group-2:
  - Light size/浮窗灯大小: 左右方向的调整拉杆，右侧显示 `xx px`。范围5-50px，默认25px
  - Click-through/点击穿透(取消则可拖动): 开关
  - Agent poll interval/Agent状态轮询间隔: 单选栏，1/2/3/5/10/15 秒。默认3秒
  - Launch at login/开机自启动(待实现): 开关
  - Theme/主题: 单选"跟随系统", "深色", "浅色"。默认"跟随系统"

#### State Settings Card

- Reset/重置: 右上角"reset"按钮可以将这个State的所有配置恢复为默认值
- Color/颜色: "颜色"为横向的单选按钮，按钮中间为颜色展示，选中时外圈有选中的效果。"颜色: " + 颜色单选按钮 总共占据一行
- Animation/效果: "效果"单选，选中效果同"Color"。总共占据一行
- Speed/速度: "速度"调整。波纹/呼吸 支持自定义速度，范围为0.2Hz - 5Hz。总共占据一行
