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
# 0) 若当前终端连 sudo cp 都报 Operation not permitted，
#    先给 Terminal / iTerm / Warp 打开 Full Disk Access，再重开终端

# 1) 从受保护路径复制出 ControlCenter plist
sudo cp /Users/<你的用户名>/Library/Group\ Containers/group.com.apple.controlcenter/Library/Preferences/group.com.apple.controlcenter.plist \
  /tmp/group.com.apple.controlcenter.plist

# 2) 用项目脚本修补 trackedApplications
python3 scripts/repair_statuskit_block.py \
  --bundle-id com.kokifish.asig \
  --input /tmp/group.com.apple.controlcenter.plist \
  --output /tmp/group.com.apple.controlcenter.fixed.plist

# 3) 覆盖回系统 plist
sudo cp /tmp/group.com.apple.controlcenter.fixed.plist \
  /Users/<你的用户名>/Library/Group\ Containers/group.com.apple.controlcenter/Library/Preferences/group.com.apple.controlcenter.plist

# 4) 重启缓存与菜单栏宿主，再打开最新版 Asig
killall cfprefsd ControlCenter
open build/Asig.app
```

- **为什么这样恢复**：
  - 第 1/3 步只对 `cp` 单独提权，避免 `sudo su` 把 `$HOME` 变成 `/var/root`
  - 第 2 步保留普通用户执行，便于在 `/tmp` 上调试和复用脚本
  - 第 4 步是必须的，因为 `ControlCenter` / `cfprefsd` 会缓存坏状态；不重启宿主，看不到修补效果
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
