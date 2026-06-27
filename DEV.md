# Asig 设计与开发

> Asig 简洁的、最权威的开发维护手册，语义冲突时以本文档为准，包括README.md和源码。
> 没有明确允许，Agent 不可修改本文档。

Asig = macOS 多 Agent 状态监控灯。菜单栏灯 + 全局置顶动态药丸浮窗。
监控 Claude Code / CodeBuddy / OpenClaw，Trae 待支持。

## Principals

- 结构清晰，逻辑简单，高内聚低耦合，提高代码复用率，降低总代码量和行数
- 不过度设计，避免不必要的薄封装

## Tech Overview

- **内核**: Rust workspace `crates/core`，零 AppKit 依赖 — 可移植，Windows 壳可直接复用
- **UI 壳**: objc2 / AppKit 纯 Rust，无 WebView — 常驻灯 <60MB，CoreAnimation 交 render server，CPU ~0%
- **跨平台**: 暂只 macOS，留口子（内核可移植，UI 壳按平台另写）

## Build and Run

```bash
cargo run -p agent-light                 # 跑(debug)
cargo build --release -p agent-light     # 发布版
cargo build -p agent-light-core          # 只验内核(纯 Rust,快)
```

性能预算: 运行内存 < 60MB，CPU 平均 < 1%

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
| 1 | `Done-Notification` | 深绿 | 快速呼吸 | 其他状态转入Done状态 |

- **Done Notification**: 在别的状态转入`Done`时，默认持续 30s 的 Done-Notification，用深绿色表示，默认动效为快速呼吸
- **聚合规则**：同一个Agent多个会话同时存在时，全局灯取**优先级最高**的那一个（`AgentStatus::priority()`，数字大者覆盖）。排序：红 > 琥珀 > 紫 > 黄 > 绿。
- **Sticky 锁定态**：`NeedsDeci` / `Error` / `Offline` 一旦进入即**锁定**——只有观测到明确的 `Working`（恢复）或 `Done`（结束）才解锁（`transition()`）。不因超时自动清，锁定态之间也**不互相覆盖**（先到先得，避免抖动闪烁）；`Done` / `Working` 可自由接受任意新观测。
- **灯效种类**：`Steady`（常亮）/ `Pulse`（呼吸）/ `Ripple`（波纹），共 3 种（详见 [Light Animations](#light-animations)）。**快闪 / 慢闪 / 呼吸都是 `Pulse`，只是周期不同**，无独立的明灭（Blink）动效。全部交 CoreAnimation 在 render server 上跑，app 进程 ~0% CPU。
- **颜色枚举**：颜色与状态一一对应，定义在内核、平台无关；app 层翻译成具体 RGB

### Light Animations

灯效 = 颜色 + 动画（`LightAnim`）。一个 `AgentStatus` → 一套默认灯效（见上表），用户可在 Settings Panel 覆盖（动效种类 / 颜色 / 周期）。

**全部交 CoreAnimation 在 render server 上驱动 GPU 插值，app 进程 ~0% CPU。**

| 动效 | 英文 | 视觉 | 涉及的属性 |
|---|---|---|---|---|
| 常亮 | Steady | 不变，纯色常亮 | 无周期，period_ms 置 0 |
| 呼吸 | Pulse | 透明度 ~0.2↔1 往复（周期越短越「闪」） | `opacity`，可定义频率 |
| 波纹 | Ripple | 一圈环以圆点为圆心对称扩散并淡出 | `transform`（绕圆心缩放的 `CATransform3D`）+ `opacity`（独立 `RingView`），单程一次扩散 |

- 默认周期：`Error`=350（快闪）/ `NeedsDeci`=1000（慢闪）/ `Working`=1800（呼吸）/ `Done`=1600（波纹）/ `Done-Notification`=450（快速呼吸）。**快闪 / 慢闪 / 呼吸都是 `Pulse`，只是周期不同**（数字越小越快），不是不同动效。
- **Done Notification**：别的态刚转 `Done` 的窗口期内，用 `Pulse`（DarkGreen，450ms）覆盖全局态。
- 可配置：Settings 里每状态独立改 动效 + 颜色 + 周期（`StateStyle`）；缺省回退内置 `AgentStatus::light()`。
- 载体：Signal Light 浮窗——圆点本体做 Steady/Pulse，波纹用独立 `RingView` 子视图叠加扩散（动画用绕圆心缩放的 `CATransform3D`——不动 layer-backed 视图会被 AppKit 重置的 `anchorPoint`，故环从圆点对称扩散）；Signal Icon（菜单栏）无动效，只显示静态色块/emoji，不可设动效。

### Signal Light

- Def: 在桌面上的可以配置动效、大小的叫 Signal Light
- Default Position: 初始位置在主屏幕的左上角（红黄绿按钮下方一行）。**位置记忆**：拖动后记住位置，下次启动自动恢复到上次位置（含所在屏幕，按 `CGDirectDisplayID` 匹配）；若该屏已断开则回退主屏左上角。记忆持久化在 `settings.json` 的 `light_pos` 字段。

### Signal Icon

- Def: 在菜单栏上的，无动效且不可设置动效的叫 Signal Icon

### Drop-down Panel

- Def: 单击菜单栏图标后的弹窗
- Position: 菜单栏单击后在图标右下方弹出菜单栏弹窗，菜单栏弹窗左侧和菜单栏Asig图标左侧对齐，但如果右侧空间不足，则右侧贴屏幕边缘。不可拖动不可自定义大小
- Upper Button: 从左至右分别为`设置`-用于打开 Settings Panel 的最左侧按钮，`锁定`-用于快速设置是否可以拖动圆角单选按钮（与 Settings Panel「浮窗点击穿透」同步同一开关），`退出`-用于退出Asig的最右侧按钮

### Settings Panel

- Def: 点击 Drop-down Panel 的设置按钮后的用于配置显示效果的面板
- Position: 默认在屏幕中央，可以拖动
- Content: 浮窗大小（滑块）、各状态样式（每状态可独立设 动画/颜色/周期）、浮窗点击穿透（勾选；与 Drop-down「锁定」同步同一开关）

