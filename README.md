# Asig

macOS 上的多 Agent 状态监控灯。把 Claude Code / CodeBuddy / OpenClaw 的实时状态,变成屏幕上一眼就懂的灯。

**三种形态**:菜单栏状态灯 + 全局置顶的动态药丸浮窗 + 点灯弹出的详情/设置面板。
**目标**:切到别的窗口干活,瞄一眼就知道 —— 在跑 / 完成了 / 要你决策 / 出错了。

> 当前为早期版本(Phase 1–2)。Claude Code / CodeBuddy / OpenClaw 已支持;Trae 暂未支持。

---

## 灯的含义

| 优先级 | 灯 | 状态 | 动画 | 含义 |
|:---:|:---:|---|---|---|
| 5 | 🔴 红 | Error | 快闪 | 报错且无法自动恢复 |
| 4 | 🟠 琥珀 | NeedsDeci | 慢闪 | 待决策(要权限 / 要输入) |
| 3 | 🟣 紫 | Offline | 常亮 | 异常 / 卡住 / 进程没了 / 不可观测 |
| 2 | 🟡 黄 | Working | 慢呼吸 | 正在跑 |
| 1 | 🟢 绿 | Done | 波纹 | 完成 / 空闲 / 初始默认态 |

- **Done Notification**:别的状态刚转入 Done 的 **1 分钟内**,灯短暂变成**深绿、快速呼吸**(菜单栏用 💚 表示),提示「刚完成,回来看」;之后回退为绿色波纹。
- 多个会话同时存在时,灯显示最需要关注的那一个(优先级数字大者覆盖:**红 > 琥珀 > 紫 > 黄 > 绿**)。
- `NeedsDeci` / `Error` / `Offline` 一旦出现即**保持**,只有重新 `Working` 或 `Done` 才解除——不会因超时自动变。

## 安装与运行

需要 macOS + Rust 工具链。

```bash
git clone https://github.com/kokifish/Asig.git
cd Asig
cargo run -p agent-light            # 直接跑(debug)
# 或构建发布版(更小、更省):
cargo build --release -p agent-light
./target/release/agent-light
```

> 国内网络拉依赖慢,可配 rsproxy 镜像(见 DEV.md「依赖镜像」)。

启动后:右上角菜单栏出现灯;屏幕上方出现一个药丸浮窗。**零配置**,自动发现已安装的 agent 会话。

## 退出

点菜单栏灯 → 弹出面板 → **退出**。

## 支持的 Agent

| Agent | 支持 | 怎么读状态 |
|---|---|---|
| Claude Code | ✅ | `~/.claude/sessions/<pid>.json`(`busy`/`idle`) |
| CodeBuddy | ✅ | `~/.codebuddy/sessions/<pid>.json` |
| OpenClaw | ✅(基础) | `~/.openclaw/state/openclaw.sqlite` |
| Trae | ⏳ 暂未 | (闭源,需 Accessibility,见 DEV.md) |

## 隐私

纯本地:只读取上述 agent 在你电脑上自己写的状态文件,不联网、不上传任何数据。

## 已知限制

- 药丸浮窗默认点击穿透;在**设置**里取消勾选「浮窗点击穿透」即可用鼠标拖动浮窗位置。
- 🟣(异常 / 不可观测):所有会话都消失时自然出现;另外 **Asig 见过的 Claude 进程若崩溃/被杀**(残留 session 文件)也会标 🟣。🟠(需决策)与 🔴(报错)Claude Code 的状态文件不提供,需接 hook 才能精准触发(详见 DEV.md)。
- OpenClaw 的「需决策」态还不够准(待补 trajectory 解析)。
- 设置面板为初版占位,真实控件(启用 Agent / 间隔 / 浮窗开关 / 主题)待补。

更多信息见 [DEV.md](./DEV.md)。
