# Asig 开发文档

> Asig 简洁的、最权威的开发维护手册，语义冲突时以本文档为准。
> 没有明确允许，Agent 不可修改本文档。

Asig = macOS 多 Agent 状态监控灯。菜单栏灯 + 全局置顶动态药丸浮窗。
监控 Claude Code / CodeBuddy / OpenClaw，Trae 待支持。

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

## Color and State Priority

一个 `AgentStatus` 同时决定**灯的颜色 + 灯效(动画)**,UI 层只消费 `status.light()`。

| 优先级 | 状态 | 灯 | 默认动效 | 含义 |
|:---:|---|:---:|---|---|
| 5 | `Error` | 🔴 红 | 快闪 | agent 报错且无法自动恢复 |
| 4 | `NeedsDeci` | 🟠 琥珀 | 慢闪 | 待决策（要权限 / 要输入） |
| 3 | `Offline` | 🟣 紫 | 常亮 | 异常 / 卡住 / 进程没了 / 未知 |
| 2 | `Working` | 🟡 黄 | 呼吸-慢速 | 正在跑 |
| 1 | `Done` | 🟢 绿 | 波纹 | 完成 / 空闲 / 初始默认态 |

- **Done Notification**: 在别的状态转入`Done`时，默认持续1分钟的 Done Notification，用深绿色表示，默认动效为快速呼吸
- **聚合规则**：同一个Agent多个会话同时存在时，全局灯取**优先级最高**的那一个（`AgentStatus::priority()`，数字大者覆盖）。排序：红 > 琥珀 > 紫 > 黄 > 绿。
- **Sticky 锁定态**：`NeedsDeci` / `Error` / `Offline` 一旦进入即**锁定**——只有观测到明确的 `Working`（恢复）或 `Done`（结束）才解锁（`transition()`）。不因超时自动清，锁定态之间也**不互相覆盖**（先到先得，避免抖动闪烁）；`Done` / `Working` 可自由接受任意新观测。
- **灯效种类**：`Steady`（常亮）/ `Pulse`（呼吸，透明度渐变）/ `Blink`（明灭）。全部交 CoreAnimation 在 render server 上跑，app 进程 ~0% CPU。
- **颜色枚举**：颜色与状态一一对应，定义在内核、平台无关；app 层翻译成具体 RGB
