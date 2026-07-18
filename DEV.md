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
- `jsonl_tail.rs` — 只读 jsonl 尾部的取数工具（claude/openclaw 共用）
- `claude.rs` — `ClaudeLikeSource`：Claude / CodeBuddy 共用（参数化根目录）。读 session 文件（camelCase 字段：`sessionId`/`kind`/`status`…，`rename_all`）+ pid 存活：
  - **按 cwd 聚合** —— 同目录的多个 session（用户手开 interactive + claude `--fork-session` 派发的后台子 claude `kind:"bg"`）合并为**一个**会话：interactive 作主，bg 不单独显示但 busy 活跃度合并进主会话状态（否则 fork 任务到后台跑时主进程 idle 成 shell 会被误判不在运行），纯 bg 无 interactive 的目录整组跳过（避免与 OpenClaw source 重叠）
  - **状态判定**（status 层，优先于 transcript）：`waiting`（Claude 等用户输入/授权，如工具 permission）→NeedsDeci；busy+transcript 尾部信号（`end_turn`→NeedsDeci；`user`（用户刚输入、Claude 处理中）/`tool_use`→Working；`end_turn` 后若已有 `user` 判 Working，不被残留 `end_turn` 误判）；idle/shell（空闲）→Done；pid 死→Offline
- `openclaw/` — `OpenClawSource`（子模块：`db` 只读 sqlite 归并 / `sessions` jsonl 尾部信号 / `probe` CLI 诊断 DTO）。两套数据源：
  - ① 只读 `~/.openclaw/state/openclaw.sqlite`（单一事实源、升级迁移目标），按 `agent_databases` 聚合 task/flow/subagent runs（ended_at NULL→Working、blocked 且 ended_at NULL→NeedsDeci、近期 failed→Error）
  - ② 交互式会话 `agents/<id>/sessions/*.jsonl` 尾部 `message.stopReason`（toolUse/user/toolResult→Working）
  - **协调后台子 agent**：主 agent `sessions_yield` 让出 + 文件以 `leaf` 结尾 = 协调后台子 agent——子 agent 在跑 → Working，子 agent 全 ended 或协调态超 30min → 卡死 → Error。子 agent 走 sessions 机制（`.trajectory.jsonl`）不进 `subagent_runs` 表，靠 leaf+yield 信号识别（GLM 下 yield 期间尾部 `assistant stop="stop"` 否则误判 Done）—— **不进主库，故单读**
  - 否则 Done
- `aggregate.rs` — `global_status()`：N 个会话压成最高优先级的全局灯态
- `status.rs` — `AgentStatus` + `Color` + `LightAnim` + sticky 状态机 `transition()` + `AgentStatus::light()`（默认灯效的单一事实源）
- `config.rs` — `Settings` / `StyleKey` / `StateStyle` / `LightPosition`：可配置灯效 + 浮窗位置，serde 持久化（`load`/`save` 失败可见不静默：无文件静默默认、IO 错提示、JSON 损坏备份成 `settings.json.bad`，均回退默认绝不 panic）
- `lib.rs` — `Monitor`（轮询编排 → `Snapshot`，含 DoneNotif 边沿检测）；`Snapshot::signature()`（指纹，app tick 据此跳过无变化 render）

**UI 壳 `crates/app`（objc2/AppKit，纯 Rust，无 WebView）：**

- `main.rs` — 入口：加载设置 → 建浮窗 → 建 `AppDelegate` → 状态栏 + tick 定时器
- `cli.rs` — CLI 子命令（`probe-openclaw`：打印各 agent 诊断 + status，判定走 `openclaw::probe`）
- `app_delegate.rs` — `AppDelegate`（`define_class!`）：tick 轮询 / 渲染分发、popover 与 settings 生命周期、点击穿透、样式改动落盘、浮窗位置记忆的枢纽（`persist_light_pos` 改字段与落盘拆两个独立 borrow scope，避免 RefCell 重入 panic）
- `tray.rs` — 菜单栏 Signal Icon（`NSStatusItem` + 自绘彩色圆点按钮；点击弹 Drop-down）+ tick 定时器
- `overlay.rs` — Signal Light 浮窗（`collectionBehavior` 据设置 `hide_in_fullscreen`:on → `Managed` 不进全屏 app 的 Space → 全屏自动消失 + 不打断菜单栏/Dock 自动隐藏;off → `CanJoinAllSpaces` 跨 Space 显示,含全屏）：自绘圆点 `PillView` + 波纹环 `RingView` + CoreAnimation 灯效 + 多屏位置几何
- `panel.rs` — Drop-down Panel：圆角卡片 `CardView` + 三按钮（设置/锁定/退出）+ 会话列表；定位在图标左下方
- `menu.rs` — 最小主菜单：仅切 regular（开设置窗）时显示。App 菜单留空（系统补 Quit ⌘Q 等）+ File 菜单 Close ⌘W（`performClose:` 走 responder chain 关设置窗 → 触发 `windowWillClose:` 切回 accessory）
- `settings/` — Settings Panel（10 子模块）。左侧栏导航 + 右侧 pane 切换；状态 pane = 颜色 / 动画 / 速度(Hz)。子模块：
  - `mod` — 装配 build/show/view_with_tag + pub use 外部 API
  - `strings` — 本地化文案
  - `consts` — 不可变常量（几何 / tag 编码 / 业务顺序 / 数值范围）
  - `tags` — helper（card_height / card_frame / row_center_y / parse_control_tag / hz_of / poll_preset_index / theme_index / sf_symbol / label_col_width）
  - `controls` — 控件工厂 add_*
  - `glass` — 液态玻璃 GlassPane + 选中态药丸
  - `layout` — StateControls + layout/refresh_*
  - `pane_general` / `pane_state` / `pane_about` — 各 pane builder
- `palette.rs` — 下拉面板会话列表用的状态 emoji(`status_emoji`)
- `notify.rs` — macOS 系统通知（UserNotifications framework：授权 + 发送）
- `logger.rs` — 极简 log 实现（`SimpleLogger`,core 的 `log::warn!` 经此输出 warn 到 stderr）

## Build and Run

```bash
cargo run -p agent-light                 # 跑(debug)
cargo build --release -p agent-light     # 发布版
cargo build -p agent-light-core          # 只验内核(纯 Rust,快)
```

Performance budget: 运行内存 < 60MB，CPU 平均 < 1%

## 测试

**代码单测（针对分析逻辑）**：`cargo test -p agent-light-core openclaw` —— 覆盖 `OpenClawSource` 的状态归并（task/flow/subagent 跨表 + 交互式 `message.stopReason`）、字段解析、边界（ended/未 ended、近期 failed 窗口、toolUse 跳闸门等）。改 `openclaw.rs` 后必跑；改判定逻辑时同步改对应单测断言。

**真实 openclaw 实测（针对本机最新版）**：`./scripts/probe-openclaw.sh` 是薄壳，判定走 `agent-light probe-openclaw` 子命令（即 `openclaw.rs` 单一事实源），对真实状态库跑同款判定、打印每 agent 应判状态。配合 `openclaw agent --agent <id> -m "..."` 触发各场景：

| 目标状态 | prompt | 期望 |
|---|---|---|
| Working（工具链）| `用 bash 执行 'ls ~/git_space/Asig/crates' 然后逐个解释` | 🟡，toolUse 期间不抖 |
| 长工具（>30s）| `用 bash 执行 'sleep 40' 然后说完成` | 🟡 持续，不闪蓝 |
| 完成 | （上一条跑完）| 转 🟢，转绿瞬间闪浅蓝（完成通知）|

跑法：`watch -n2 ./scripts/probe-openclaw.sh`，另开终端触发 openclaw 任务，对照 Asig 浮窗/面板。openclaw 升级后先跑此脚本回归（字段/表若变了，会先于 Asig 暴露不一致）。

## 已修复的 Claude 状态判定误判

Claude source（`claude.rs::classify`）的 NeedsDeci/Working 判定踩过的坑（按修复时间倒序，便于回溯）：

| 误判现象 | 根因 | 修复 |
|---|---|---|
| 实际等用户（待决策）、Asig 显示运行中 | session `status=waiting`（Claude 等输入/授权，如工具 permission）`classify` 不识别，落 `_=>Working`；且 bg transcript 尾部是历史 `tool_use`，读 transcript 也给 Working | `classify` 在 status 层加 `waiting=>NeedsDeci`，优先于 transcript（5390228）|
| 实际在运行、Asig 显示待决策 | `read_tail_stop_reason` 只读尾部最后一条 assistant `stop_reason`，忽略其后的 `user` 消息 → 上一轮 `end_turn` 残留被误读 | 改 `read_tail_signal`：尾部最后一条有意义事件（`type:user`→"user"；`type:assistant`→其 stop_reason）；`busy+user=>Working`（82a7a35）|
| fork 任务到后台跑、主进程 idle 成 shell、显示不在运行 | 旧实现跳过所有 `kind:"bg"`，丢失 bg 的 busy 活跃度 | 按 cwd 聚合：interactive 作主、bg 活跃度合并（bb28c06）|
| Claude REPL 空闲（shell）显示运行中 | `shell` status 被当未知 → Working | `classify` 加 `shell=>Done`（20579ca）|

判定优先级（**status 层优先于 transcript**）：pid 死→Offline；活 `waiting`→NeedsDeci；活 `busy` 读 transcript 尾部信号（`end_turn`→NeedsDeci、`user`/`tool_use`/未知→Working）；活 `idle`/`shell`→Done。

## Design

- 需要轮询的，默认3s轮询一次

### Signal Color and State Priority

一个 `AgentStatus` 同时决定**灯的颜色 + 灯效(动画)**,UI 层只消费 `status.light()`。

| 优先级 | 状态 | 状态名称 | 灯 | 默认动效 | 含义 |
|:---:|---|---|:---:|---|---|
| 5 | `Error` | 错误/Error | 🔴 红 | 快闪 | agent 报错且无法自动恢复 |
| 4 | `NeedsDeci` | 待决策/Pending | 🟠 琥珀 | 波纹 | 待决策（要权限 / 要输入） |
| 3 | `Offline` | 异常/Offline | 🟣 紫 | 常亮 | 异常 / 卡住 / 进程没了 / 未知 |
| 2 | `Working` | 运行中/Working | 🟡 黄 | 呼吸-慢速 | 正在跑 |
| 1 | `Done` | 已完成/Done | 🟢 绿 | 波纹 | 完成 / 空闲 / 初始默认态 |
| 0 | `DoneNotif` | 完成通知/Notify | 🔵 浅蓝 | 快速呼吸 | 其他状态转入Done状态 |

- **状态名称** = 中文 / 英文（两档双语专称，表中并列）。Settings Panel「Left Side Tabs」状态 tab 的显示名**只取其中一档**——按常规设置「语言」决定（中文模式→中文 / 英文模式→英文短称），不双语并排。英文为面向 tab 的简称：Error / Pending / Offline / Working / Done / Notify。

- **Done Notification**: 在别的状态转入`Done`时，默认持续 30s 的 DoneNotif (Done-Notification)，用浅蓝色表示，默认动效为快速呼吸
- **Aggregation（两层归并，优先级语义不同）**：
  - **跨 agent 全局聚合**（`aggregate::global_status`）：N 个 agent 会话压成一颗全局灯，统一用 `AgentStatus::priority()`（数字大者覆盖）。排序：红 > 琥珀 > 紫 > 黄 > 绿。
  - **单 agent 内多会话归并**（source 层，聚合之前）：各 source 自行归并，允许有设计性差异 —— 如 `claude::most_active` 故意把 `Offline` 压在 `Done` 之下（一个崩溃的 bg 子进程不该把整个 agent 拉成 Offline，抗抖动），与全局 `priority()`（Offline>Working>Done）有意不同；`openclaw::classify_agent` 不产生 Offline，顺序与 `priority()` 一致。
- **Sticky state**：`NeedsDeci` / `Error` / `Offline` 一旦进入即**锁定**——只有观测到明确的 `Working`（恢复）或 `Done`（结束）才解锁（`transition()`）。不因超时自动清，锁定态之间也**不互相覆盖**（先到先得，避免抖动闪烁）；`Done` / `Working` 可自由接受任意新观测。
- **Latched grace period**：会话连续 `LATCH_GRACE`（=2 轮，约 6s）未被观测到才从锁定表清除，而非一轮即删——覆盖 source 端文件原子替换 / 瞬时改名等抖动（本轮 `live` 集合短暂不含该会话）；否则下轮重现会以 `Done` 为基线重算，丢失锁定态（违反 sticky）。连续超宽限才清，避免幻影堆积。
- **Animation types**：`Steady`（常亮）/ `Pulse`（呼吸）/ `Ripple`（波纹），共 3 种（详见 [Light Animations](#light-animations)）。**快闪 / 慢闪 / 呼吸都是 `Pulse`，只是周期不同**，无独立的明灭（Blink）动效。全部交 CoreAnimation 在 render server 上跑，app 进程 ~0% CPU。
- **Color enum**：颜色定义在内核、平台无关；app 层翻译成具体 RGB。共 12 色（Tailwind 源）：6 个与默认状态一一对应（Green / LightBlue / Yellow / Amber / Red / Purple）+ 6 个个性化扩展（Blue / Indigo / Teal / Cyan / Orange / Pink，仅 Settings 可选，无默认映射）。每色浅 / 深两档（Tailwind 500 / 400），随外观自适应（见下「Appearance」）

### Light Animations

灯效 = 颜色 + 动画（`LightAnim`）。一个 `AgentStatus` → 一套默认灯效（见上表），用户可在 Settings Panel 覆盖（动效种类 / 颜色 / 周期）。

**全部交 CoreAnimation 在 render server 上驱动 GPU 插值，app 进程 ~0% CPU。**

| 动效 | 英文 | 视觉 | 涉及的属性 |
|---|---|---|---|
| 常亮 | Steady | 不变，纯色常亮 | 无周期，period_ms 置 0 |
| 呼吸 | Pulse | 透明度 ~0.2↔1 往复（周期越短越「闪」） | `opacity`，可定义频率 |
| 波纹 | Ripple | 两圈环从**最内层外缘**起、错相(半周期)对称扩散（layers>0 时穿过半透明外层，视觉读作「从最内层扩散出去」）；最大直径 = 灯直径（扩到灯边缘）；`opacity` 中段完全不透明（硬边）、仅末尾短淡出 | `transform`（绕圆心缩放的 `CATransform3D`；scale 终值 = l，终态直径 = dot）+ `opacity`（2 个错相 `RingView` 的 keyframe：中段 1.0、末尾淡到 0），单程一次扩散 |

- Default period：`Error`=350（快闪）/ `NeedsDeci`=2500（波纹,≈0.4Hz，比 Done 稍快）/ `Working`=1800（呼吸）/ `Done`=3333（波纹,≈0.3Hz）/ `DoneNotif`=450（快速呼吸）。**快闪 / 慢闪 / 呼吸都是 `Pulse`，只是周期不同**（数字越小越快），不是不同动效。
- **Done Notification**：别的态刚转 `Done` 的窗口期内，用 `Pulse`（LightBlue，450ms）覆盖全局态。
- Configurable：Settings 里每状态独立改 动效 + 颜色 + 周期 + 渐变层数（`StateStyle`）；缺省回退内置 `AgentStatus::light()`。
- Carrier：Signal Light 浮窗——圆点本体做 Steady/Pulse，波纹用两个错相 `RingView` 子视图扩散（动画用绕圆心缩放的 `CATransform3D`——不动 layer-backed 视图会被 AppKit 重置的 `anchorPoint`，故环从圆点对称扩散）；Signal Icon（菜单栏）无动效，只显示自绘彩色圆点（`overlay::swatch_image`，`setTemplate:NO` 保留真彩），不可设动效。
- 速度（周期）以 **Hz** 呈现给用户（`period_ms = 1000 / Hz`）；常亮（Steady）无周期、速度不可设。
- **渐变层数（Gradient layers）**：圆点本体按半径等距分 L=layers+1 个同心环（slider 值 layers∈0..=4，默认 1），第 k 层（k=0 中心）透明度 α=1−k/L（中心最亮、向外线性递减；0=纯色单层=历史行为，1=两层外层 α=0.5，2=三层中 2/3·外 1/3）。每段画 even-odd 环（外圆+内圆 path）独立 α、互不重叠，避免 source-over 合成使中间层 α 累加。
  - **不进 `LightAnim` 枚举**：`layers` 与动画类型正交、且只被浮窗 `drawRect` 消费，故**不**放 `LightAnim` 枚举（避免随 `light()` 流经菜单栏图标 / 波纹环 / 色块等不分级消费者），而是作 `set_light` 的正交参数，由 `Settings::layers(snap)` 经 `StateStyle::layers()` 单独取。
  - **作用范围**：**仅作用于 Signal Light 浮窗圆点本体**；Signal Icon（菜单栏，18px 太小）与波纹环（`RingView` 扩散动画）不分级。Settings State pane 每状态独立设（整数拉杆，右侧显示 slider 值，0..=4，默认 1）；Reduce Motion 降级为 Steady 时保留层数。

### System Notifications（系统通知）

转入某些状态时弹 macOS 系统通知（`UserNotifications` framework,`notify.rs`），让用户全屏干活也能被叫回。

- 触发:`tick` 边沿检测(`app_delegate::maybe_notify`)——`snap.global` 转入(≠上一轮)且在 `notify_on` → 发通知;同一状态停留不重复弹。
- 默认 `[NeedsDeci, Error]`(`Settings.notify_on`,serde default 向后兼容)。General pane「状态通知」多选可改(5 个 AgentStatus chip)。
- 内容:title=`Asig`,body=状态名(按 `Settings.lang`);identifier 固定 `asig-status`(新通知覆盖旧,通知中心不堆积)。
- 授权:启动 `notify::request_authorization`(alert+sound),首次弹系统对话框;未授权则 `send` 静默 no-op。

### Accessibility（Reduce Motion / Reduce Transparency）

遵循 macOS 无障碍开关（System Settings → Accessibility → Display），读 `NSWorkspace.shared` 的两个布尔：

- **Reduce Motion 开启**：Signal Light 的 `Pulse`/`Ripple` 一律**降级为 `Steady`**（保留颜色、不动）—— 状态仍由颜色区分，只是不再脉冲/扩散，避免对晕动症用户不适。降级在 `overlay::set_light` 入口处据 `reduce_motion_on()` 完成；用户切该开关时，tick 把 `reduce_motion` 并入渲染签名 → 签名变化 → 立即重渲染（无需常驻渲染，不损 CPU）。Signal Icon（菜单栏）本就无动效，不受影响。
- **Reduce Transparency 开启**：Settings/Drop-down 的液态玻璃退化不透明。Drop-down 的 `NSPopover` 由系统自动处理；Settings 在 `glass_pane` 里**跳过 `NSGlassEffectView`**、改用 `NSVisualEffectView`（其在 Reduce Transparency 下自动变实色），保证文字可读（设置窗在(重)开时取最新值）。

### Appearance（Theme + 颜色深浅自适应）

- **Theme**（Settings → General）：跟随系统 / 深色 / 浅色（横向 radio 单选,与「效果」同款），默认跟随系统。改动即设 `NSApp.appearance`（FollowSystem→nil 继承系统）并重建 + 重绘；持久化在 `settings.json` 的 `theme` 字段（serde，旧配置无该字段回退默认）。
- **颜色随外观自适应**：12 色每色含浅 / 深两档（Tailwind 500 / 400），经 `NSColor colorWithDynamicProvider` 包装。
  - 浮窗：自绘 `drawRect` 每次重绘按当前 `NSAppearance` 取档；`PillView` / `RingView` 重写 `viewDidChangeEffectiveAppearance`，故系统深浅切换时浮窗**实时**重绘。
  - 菜单栏图标 / Settings 色块（栅格化位图 `swatch_image`）：动态色在 `lockFocus` 时会被冻结，故改用「当前外观静态色」栅格化，并靠 tick 渲染签名并入 `effectiveAppearance`（同 reduce_motion 模式）在 ≤ 轮询周期内自动刷新。

### Signal Light

- Def: 在桌面上的可以配置动效、大小的叫 Signal Light
- Default Position: 初始位置在主屏幕的左上角（红黄绿按钮下方一行）。
  - **Position memory**：拖动后记住位置，下次启动**按存的坐标点定位其所在屏**来恢复（不依赖可能错配的 `screen_id`：`persist_light_pos` 存原点却按窗口中心判屏，拖动跨屏边界时原点与中心所在屏不一致，按 `screen_id` 恢复会把浮窗 clamp 进屏缝丢失）；接缝上的点归主屏（`screens[0]`），点不在任何屏（屏断开 / 坐标过期）则回退主屏左上角。记忆持久化在 `settings.json` 的 `light_pos` 字段。

### Signal Icon

- Def: 在菜单栏上的，无动效且不可设置动效的叫 Signal Icon

### Drop-down Panel

- Def: 单击菜单栏图标后的弹窗
- Position: 菜单栏单击后在图标右下方弹出菜单栏弹窗，菜单栏弹窗左侧和菜单栏Asig图标左侧对齐，但如果右侧空间不足，则右侧贴屏幕边缘。不可拖动不可自定义大小
- Upper Button: 从左至右分别为`设置`-用于打开 Settings Panel 的最左侧按钮，`锁定`-用于快速设置是否可以拖动圆角单选按钮（与 Settings Panel「浮窗点击穿透」同步同一开关），`退出`-用于退出Asig的最右侧按钮
- 材质：`NSPopover`（SDK 26+ 链接即自动获得液态玻璃，无需手动 vibrancy）。

### Settings Panel

- Def: 点击 Drop-down Panel 的设置按钮后的用于配置显示效果的面板
- Activation: Asig 是 accessory 菜单栏 app（`Info.plist` 的 `LSUIElement=true`，不占 Dock / 不在 Cmd+Tab 切换器）。**设置窗打开期间临时 `setActivationPolicy(.regular)`** —— 出现在 Dock + Cmd+Tab + 主菜单栏，可正常窗口切换；**关闭时（`windowWillClose:`）切回 `.accessory`** 退回纯菜单栏。AppDelegate 仅被设为设置窗的 window delegate，故 `windowWillClose:` 只由设置窗触发，无需判断 object。首次切 regular 时建最小主菜单（`menu.rs`：App 菜单留空由系统补 Quit ⌘Q + File 菜单 Close ⌘W → `performClose:`），让无主菜单的 accessory app 在 regular 期间也有关窗 / 退出快捷键。
- Position: 默认在屏幕中央，可以拖动；**可调整大小**。
  - **尺寸**：minSize = 默认 750×460；侧栏固定宽随高、右区 `NSScrollView` 随窗宽自适应。750 宽配合「点击穿透」label 去掉「则」字收窄 label 列（label_col_width 从 ~160 降到 ~139），让 General pane 的「监控的 Agent」3 chip 与「状态通知」5 chip 默认单行不换行，Group-2 card 只 7 行 content_h≈436 < 460，首屏完整不被窗口底截断。
  - **右区滚动 + 顶部锚定**：右区是 `NSScrollView`（documentView = `FlippedView` 顶锚 + 透明 ClipView 承玻璃），各 pane 内容超高自动出竖滚动条；缩放窗口时 pane 顶部固定不漂移，documentView 高 = max(clip 可视高, pane content_h)（切 tab / 窗口缩放时设 + 滚顶，见 `set_doc_height`）。取 max 而非纯 content_h —— doc 矮于 clip 时 NSClipView 对翻转短文档（`FlippedView`）的顶部锚定会随 doc 高漂移（常规页内容曾比别的偏高 ~9pt）；doc 始终 ≥ clip 则各 pane 顶部锚定一致（短内容下方留白融于玻璃）。
  - **紧凑编排**：W=750 / SIDEBAR_W=160 / CONTENT_PAD_X=22；label 列宽 `label_col_width`（sizeToFit 测最宽文字，排除 reset 按钮），非固定值。
- Navigation: 左侧栏（顶部 tab 列表 + 底部图标行）+ 右侧 pane 切换。点 tab / 「关于」图标切换右侧 pane。
- 材质：真·液态玻璃（macOS 26+ `NSGlassEffectView`，UI 必须放进其 `contentView`；旧系统回退 `NSVisualEffectView` vibrancy）。
  - 窗口 = 一整片主玻璃（透明标题栏，玻璃贯穿顶部）。
  - **左侧栏是浮动玻璃面板**——独立一块 `NSGlassEffectView` 叠在主玻璃上，二次模糊自然更不透明，读作浮于内容之上的圆角玻璃块。刻意**不用** `NSGlassEffectContainerView`：它会合并重叠/相邻的玻璃成一次模糊，反而让浮动侧栏与主玻璃融为一体、失去「浮动」层次。
  - **右侧内容区无外框、标题下无横线**；靠极淡连续圆角卡片（`quaternaryLabelColor`）分组（stats.app 式编排），用层级而非厚重描边区分。
- Content:
  - 右侧内容区有自己的 **header**：标题固定在右侧内容区的左上方（State pane 的 Reset 按钮对齐到该 header 右侧），而不是漂在卡片列中央；标题下方不再有分隔线。
  - General pane: 浮窗大小（滑块）、浮窗点击穿透（勾选；与 Drop-down「锁定」同步同一开关）、轮询间隔（下拉；改完即时重排 tick 定时器）、监控的 Agent（多选块；选中=监控,点击 toggle）、开机启动（占位，待实现）。详见 General Settings Card。
  - State pane(每状态一个): 颜色（12 色块,**固定像素间距(15px)、左对齐 flow**——随窗宽自动换行,每行数量可不同,很宽时合并为 1 行;间距始终恒定、换行后与第一行同间距左对齐;label 左对齐（列宽 = `label_col_width` sizeToFit 测最宽文字）、控件区往左加宽;Tailwind 源、随主题深浅自适应）/ 动画（单选）/ 速度(Hz，`period_ms = 1000/Hz`；常亮时速度禁用)。详见 State Settings Card。
  - About pane: 版本号 + GitHub 链接（纯展示）。
  - 各状态可独立改 动画 + 颜色 + 周期（`StateStyle`）；缺省回退内置 `AgentStatus::light()`。
- **Left Side Tabs**（左侧栏顶部、左对齐、自上而下 7 项；顺序固定）：
  - 顺序（中文 / 英文）：① 常规设置 / General Settings（齿轮）→ ② 完成通知 / Notify → ③ 已完成 / Done → ④ 运行中 / Working → ⑤ 待决策 / Pending → ⑥ 错误 / Error → ⑦ 异常 / Offline。②–⑦ 为状态 tab，名称取自上表「状态名称」列。
  - 语言：按常规设置「语言」**只显示其中一档**——中文模式全中文、英文模式全英文短称，**不双语并排**。
  - 结构：状态 tab = 当前色圆点 + 名称；General tab = 齿轮（template SF Symbol）+ 名称。
  - Color: 除状态色圆点外，其余（齿轮、文字）均黑白风 / macOS 默认暗色，不用彩色。
  - 选中态：选中 tab = 实心强调色圆角块（`controlAccentColor`，cornerRadius 8，连续圆角 squircle），选中文字（及 General 齿轮）转白；状态色圆点保持彩色。**不用文字前缀（无 ▸ 三角形）**，与 stats.app 一致（玻璃/vibrancy 材质的选中态在玻璃侧栏上不可辨，故用实心强调色）。
- Left Side Buttons: 关于(About)、访问官网、调试、捐赠、退出Asig（左→右）。除「关于」外均为占位禁用按钮(留待实现)。
  - Color: 均黑白风 / macOS 默认暗色（单色 SF Symbol 图标），不用彩色。

#### General Settings Pane

- Name/名称: General Settings/常规设置
- icon/图标: 常见的齿轮形状的macos纯色图标
- 标题右侧 Reset/重置按钮: 把本页 General 字段(灯大小/轮询/主题/监控的 Agent/状态通知/点击穿透)恢复为默认值,不动语言与各状态样式;不弹确认(与 State pane 的 reset 一致)。与 Group-1 的「重置所有」并存(后者重置全部含语言+状态样式,弹确认)

> Group不带名称，仅用于分组，以下描述顺序也是卡片内选项的从上至下的顺序

- Group-1:
  - Language/语言: 单行单选列表: English, 中文。默认中文
  - Reset All/重置所有: 按钮，点击后会弹出确认对话框。重制为默认值，包括语言和状态显示的配置，全部自定义内容都恢复为默认值。在该group下居中
- Group-2:
  - Light size/浮窗灯大小: 左右方向的调整拉杆，右侧显示 `xx px`。范围20-80px，默认25px
  - Click-through/点击穿透(取消可拖动): 开关。默认开
  - Agent poll interval/Agent状态轮询间隔: 单选栏，1/2/3/5/10/15 秒。默认3秒
  - Agent to monitor/监控的 Agent: 多选块(Claude Code / CodeBuddy / OpenClaw 横排圆角块,选中=强调色边框+浅底,点击 toggle;选中=监控该 Agent,未选=不监控)。默认全选；允许全不选(=不监控任何 agent)；数据结构 `enabled_agents: Vec<AgentKind>`
  - Status notifications/状态通知: 多选块(已完成/运行中/待决策/错误/异常 横排圆角块,选中=转入该状态时弹 macOS 系统通知,点击 toggle)。默认 待决策+错误;数据结构 `notify_on: Vec<AgentStatus>`
  - Hide in fullscreen/全屏自动隐藏: 开关。默认开。开启时浮窗 collectionBehavior=Managed(不进全屏 Space:全屏自动消失 + 不打断菜单栏);关闭时=CanJoinAllSpaces(跨 Space 显示,含全屏);数据结构 `hide_in_fullscreen: bool`
  - Launch at login/开机自启动(待实现): 开关。默认关
  - Theme/主题: 横向单选按钮组 "跟随系统", "深色", "浅色"。默认"跟随系统"

#### State Pane

- Reset/重置: 右上角"reset"按钮可以将这个State的所有配置恢复为默认值
- Color/颜色: "颜色"为色块单选(按钮中间为颜色展示,选中时外圈带选中环)。色块**固定像素间距(15px)、左对齐 flow**,随窗宽自动换行(每行数量可不同)、很宽时合并为 1 行;换行后与第一行保持同间距、左对齐(间距始终恒定,不随宽度拉伸)。"颜色: "label + 色块组占一或多行。
- Animation/效果: 横向单选按钮组。总共占一行
- Speed/速度: "速度"调整。波纹/呼吸 支持自定义速度，范围为0.2Hz - 5Hz。总共占一行
- Gradient layers/渐变层数: "渐变层数"整数拉杆(0..=4,默认 1),右侧显示 slider 值。把浮窗圆点本体按半径等距分成 layers+1 个同心环(中心 α=1、向外线性递减 α=1−k/L);0=纯色单层(历史行为)。仅作用于浮窗圆点本体,菜单栏图标/波纹环不分级。不受 Animation 类型影响(常亮也可调,与 Speed 不同)。总共占一行

##### DoneNotif Pane

继承普通 State Pane 的全部行,仅额外多一行:

- Duration/持续时间: 左右拉杆调整,范围 5–60s,默认 30s;右侧实时显示 `xx s`。控制别的状态转入 `Done` 后 DoneNotif 灯效持续的窗口时长(内核 `poll` 据此判定,改完下一轮 tick 即生效)。独立占一行,不受 Animation 类型影响(即便常亮也显示并可调,与 Speed 不同)。持久化在 `settings.json` 的 `done_notif_duration_s` 字段。
