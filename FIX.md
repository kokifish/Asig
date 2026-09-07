# Asig 故障排查与修复

> Asig 的问题现象、根因分析、恢复步骤与排障经验沉淀。设计与开发主手册见 [DEV.md](./DEV.md)。

## Tahoe: 菜单栏图标已注册但始终不显示

- **症状**：`Asig` 进程正常运行，`tray.rs` 的 `NSStatusItem`/`NSStatusBarButton` 创建链路也正常，但右上角没有状态栏图标。
- **根因**：在 macOS 26 Tahoe 上，问题不一定在 `NSStatusItem` 代码本身，而可能是 `ControlCenter` / StatusKit 对某个 **bundle id** 持久化了坏状态。典型日志序列是：
  1. `Host properties initialized`
  2. `Starting to track host`
  3. `Created instance ... in .menuBar`
  4. 紧接着 `Moving host to blocked list`
  5. 然后 `Responding to displayables availability update; hiding status items`
- **为什么这样判断**：这次实测里，同一份 `build/Asig.app` 只把 bundle id 临时改成 `com.kokifish.asig.bidtest` 后，状态栏图标立刻显示；`ControlCenter` 也不再出现 `Moving host to blocked list`。这说明当前 `tray.rs` 绘制逻辑本身可以工作，坏的是旧 `com.kokifish.asig` 对应的系统持久化状态，而不是菜单栏渲染代码。
- **如何确认**：
  - 看 `ControlCenter` 日志里是否出现 `Starting to track blocked host` / `Moving host to blocked list`
  - 再用一个只更换 bundle id 的临时副本做对照实验；如果临时 bundle id 能显示图标，就基本坐实是旧 bundle id 的系统状态被污染
- **修复方式**：
  - 优先修系统状态：修补 `~/Library/Group Containers/group.com.apple.controlcenter/Library/Preferences/group.com.apple.controlcenter.plist` 里的 `trackedApplications`
  - 备选 workaround：改正式 bundle id，绕过坏状态；代价是 app 身份发生一次性迁移
- **恢复正式 Asig 状态栏图标的推荐流程**：

```bash
# 前置:当前终端(Claude Code / Terminal / iTerm / Warp)必须有 Full Disk Access。
#   sudo 不能绕过这个 TCC 检查 —— 报 Operation not permitted 就去
#   System Settings → Privacy & Security → Full Disk Access 开权限后重开终端。
#   plist 文件归当前用户所有,有 FDA 即可直接读写,无需 sudo。

# 一条命令:原地修补系统 plist(+自动 .bak 备份)→ killall cfprefsd/ControlCenter → open Asig
python3 scripts/repair_statuskit_block.py --apply --relaunch-app build/Asig.app

# 只读检查(不改文件):
python3 scripts/repair_statuskit_block.py --print-summary-only

# 手动分解(脚本仍支持,用于 /tmp 上的离线修补/对照):
#   python3 scripts/repair_statuskit_block.py --bundle-id com.kokifish.asig \
#     --input <拷出的 plist> --output <修好.plist>
```

- **为什么这样恢复**：
  - `--apply` = `--in-place --restart-controlcenter` 的合一（再加 `--relaunch-app` 重开 app）；脚本自动建 `.bak` 备份，并在访问被拒时提示去开 Full Disk Access
  - 必须重启 `cfprefsd` / `ControlCenter`：它们缓存坏状态，不重启看不到修补效果
  - 无需 sudo：plist 归当前用户；真正的门槛是终端的 Full Disk Access（TCC），`sudo` 绕不过（实测 `sudo cp` 在无 FDA 时同样 `Operation not permitted`）
- **项目内脚本**：`scripts/repair_statuskit_block.py`
  - 用途：修补 `trackedApplications` 里的目标 bundle entry，并移除其他 `isAllowed = false` entry 中残留指向目标 bundle 的 `menuItemLocations` 脏引用
  - 设计原因：只把目标 entry 改成 allowed 并不总够，因为 Tahoe 可能是被“外部 blocked entry 的脏引用”覆盖掉
  - 示例：

```bash
# 先做只读检查（若你已经把系统 plist 复制到可读路径）
python3 scripts/repair_statuskit_block.py \
  --bundle-id com.kokifish.asig \
  --input /tmp/group.com.apple.controlcenter.plist \
  --print-summary-only

# 生成修复后的副本
python3 scripts/repair_statuskit_block.py \
  --bundle-id com.kokifish.asig \
  --input /tmp/group.com.apple.controlcenter.plist \
  --output /tmp/group.com.apple.controlcenter.fixed.plist

# 真正落回系统文件前，先自行做好 root 级备份；应用后需要重启缓存进程
killall cfprefsd ControlCenter
open build/Asig.app
```

- **注意**：
  - 系统 `group.com.apple.controlcenter.plist` 是受保护文件，直接覆盖通常需要管理员权限
  - **额外坑**：Tahoe 下这个文件即使 `stat` 显示为当前用户拥有（例如 `koki:staff 600`），从受限 agent shell 里仍可能在真正 `open/read` 时返回 `Operation not permitted`。也就是说，**“看得到元数据”不代表“拿得到内容”**；落地修复时要么用本机交互式终端执行，要么确保当前终端 / IDE 已有足够的系统授权（如 Full Disk Access）
  - **再一个坑**：不要直接 `sudo su` 后再用 `$HOME/Library/...`，因为此时 `$HOME` 会变成 `/var/root`，路径就错了。正确做法是**保留普通用户 shell，只对碰系统 plist 的 `cp` 命令单独加 `sudo`**，或者把路径写死成 `/Users/<你的用户名>/Library/...`
  - **如果连 `sudo cp /Users/<你>/Library/Group Containers/...` 都报 `Operation not permitted`**：说明问题已经不是管理员权限，而是当前终端 app 没有 **Full Disk Access**。这时先去 `System Settings -> Privacy & Security -> Full Disk Access` 给你正在用的终端（Terminal / iTerm / Warp 等）开权限，然后重开终端再执行修复命令；否则脚本本身永远没有输入文件可修
  - 如果只是想验证根因，不要先改源码，优先做“临时 bundle id 对照实验”

## Claude Code: 状态判定误判(历史)

Claude source（`claude.rs::classify`）的 NeedsDeci/Working 判定踩过的坑（按修复时间倒序，便于回溯；现行判定优先级见 [DEV.md](./DEV.md)）：

| 误判现象                                                   | 根因                                                                                                                                                                    | 修复                                                                                                                                  |
| ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| 同 cwd 多个 interactive、当前会话已结束、Asig 仍显示待决策 | `group_status` 把组内所有成员（含其他 interactive）`most_active` 合并，遗留 `busy+end_turn` REPL 把整组拉成 NeedsDeci                                                   | primary 改取 `statusUpdatedAt` 最新 interactive；`group_status` 只合并 primary+bg，跳过其他 interactive                               |
| 实际等用户（待决策）、Asig 显示运行中                      | session `status=waiting`（Claude 等输入/授权，如工具 permission）`classify` 不识别，落 `_=>Working`；且 bg transcript 尾部是历史 `tool_use`，读 transcript 也给 Working | `classify` 在 status 层加 `waiting=>NeedsDeci`，优先于 transcript（5390228）                                                          |
| 实际在运行、Asig 显示待决策                                | `read_tail_stop_reason` 只读尾部最后一条 assistant `stop_reason`，忽略其后的 `user` 消息 → 上一轮 `end_turn` 残留被误读                                                 | 改 `read_tail_signal`：尾部最后一条有意义事件（`type:user`→"user"；`type:assistant`→其 stop_reason）；`busy+user=>Working`（82a7a35） |
| fork 任务到后台跑、主进程 idle 成 shell、显示不在运行      | 旧实现跳过所有 `kind:"bg"`，丢失 bg 的 busy 活跃度                                                                                                                      | 按 cwd 聚合：interactive 作主、bg 活跃度合并（bb28c06）                                                                               |
| Claude REPL 空闲（shell）显示运行中                        | `shell` status 被当未知 → Working                                                                                                                                       | `classify` 加 `shell=>Done`（20579ca）                                                                                                |
