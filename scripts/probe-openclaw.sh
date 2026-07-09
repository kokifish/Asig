#!/usr/bin/env bash
# OpenClaw 状态探针(shell 入口)。
#
# 判定逻辑统一走 agent-light CLI 的 `probe-openclaw` 子命令(core::openclaw::probe,单一事实源);
# 本脚本仅作 shell 入口,保留 `watch -n2` / 配合 `openclaw agent` 触发的便利。
#
# 这样消除了「openclaw.rs 与 probe-openclaw.sh 两套实现、改一边必须同步另一边」的维护负担
# (CLAUDE.md 旧要求)—— 判定改动只改 Rust,shell 自动跟随。
#
# 用法:
#   ./scripts/probe-openclaw.sh                     # 打印一次快照
#   watch -n2 ./scripts/probe-openclaw.sh           # 每 2s 刷新(观察状态流转)
#   while sleep 2; do clear; ./scripts/probe-openclaw.sh; done   # 无 watch 时
#
# ⚠️ 需先 `./scripts/make-app.sh` 打包(build/Asig.app 含 CLI 子命令)。
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="$HERE/../build/Asig.app/Contents/MacOS/agent-light"
if [ ! -x "$BIN" ]; then
    echo "未找到 $BIN —— 先 ./scripts/make-app.sh 打包" >&2
    exit 1
fi
exec "$BIN" probe-openclaw "$@"
