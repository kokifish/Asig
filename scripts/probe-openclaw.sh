#!/usr/bin/env bash
# 对真实 openclaw 跑 Asig(OpenClawSource)同款状态判定,打印每个 agent 应被 Asig 判成的状态。
# 用途:openclaw 升级后快速回归 Asig 的真实数据判定;或对照 Asig 浮窗/面板核对一致。
#
# 用法:
#   ./scripts/probe-openclaw.sh                          # 打印一次快照
#   watch -n2 ./scripts/probe-openclaw.sh                # 每 2s 刷新(观察状态流转)
#   while sleep 2; do clear; ./scripts/probe-openclaw.sh; done   # 无 watch 时
#
# 配合触发:另开终端 `openclaw agent --agent <id> -m "..."`,看本脚本 → 状态是否与
# Asig 面板/灯一致。prompt 集(Working/工具链/长工具/完成)见 DEV.md「测试」。
#
# 判定逻辑与 crates/core/src/openclaw.rs 保持一致 —— 改 openclaw.rs 时务必同步改本脚本:
#   近期(30s)任意表 failed/lost/subagent-error          → Error 🔴
#   else flow_runs blocked 且 ended_at IS NULL            → NeedsDeci 🟠
#   else 任意 running(主库 ended_at NULL 或 交互式在跑)  → Working 🟡
#   else                                                   → Done 🟢
# 交互式「在跑」:尾部 message stopReason='toolUse' 或 (role∈{user,toolResult} 且近 5min);
# toolUse 跳过 5min 闸门(工具执行长间隙不算完成);
# 或 主 agent sessions_yield 让出 + 文件以 leaf 结尾(协调后台子 agent,子 agent 走独立
# trajectory 不进 subagent_runs 表):子 agent 全 ended 或协调态超 30min → 卡死 → Error 🔴;
# 子 agent 在跑 → Working 🟡。
set -euo pipefail

DB="${OPENCLAW_DB:-$HOME/.openclaw/state/openclaw.sqlite}"
ROOT="${OPENCLAW_ROOT:-$HOME/.openclaw}"
NOW_S=$(date +%s)
NOW_MS=$((NOW_S * 1000))
ERR_MS=30000          # ERROR_RECENT_MS
SESSION_RECENT_S=300  # SESSION_RECENT_MS(5min)
SUBAGENT_WAIT_S=1800  # SUBAGENT_WAIT_MS(30min):sessions_yield 协调态宽窗
AGENT_RECENT_S=$((30 * 86400))

[ -f "$DB" ] || { echo "找不到 $DB(openclaw 未安装或未运行?)"; exit 0; }
command -v jq >/dev/null || { echo "需要 jq(读 jsonl 尾部)"; exit 1; }

echo "now: $(date '+%H:%M:%S')  db=$DB"

# 近期 agent 集合(30 天 last_seen,与 Asig AGENT_RECENT_MS 一致)
agents=$(sqlite3 -readonly "$DB" \
  "SELECT DISTINCT agent_id FROM agent_databases WHERE last_seen_at >= $((NOW_S - AGENT_RECENT_S)) * 1000 ORDER BY agent_id;")

for aid in $agents; do
  # task_runs(干净 agent_id 列):running / 近期 failed+lost
  read -r tr_run tr_err <<< "$(sqlite3 -separator ' ' -readonly "$DB" "
    SELECT COALESCE(SUM(ended_at IS NULL),0),
           COALESCE(SUM(ended_at IS NOT NULL AND ended_at > $((NOW_MS - ERR_MS)) AND status IN ('failed','lost')),0)
    FROM task_runs WHERE agent_id = '$aid';")" || true

  # flow_runs(owner_key 前缀 或 纯 id):running / blocked+NULL / 近期 failed
  read -r fr_run fr_blk fr_err <<< "$(sqlite3 -separator ' ' -readonly "$DB" "
    SELECT COALESCE(SUM(ended_at IS NULL AND status != 'blocked'),0),
           COALESCE(SUM(status = 'blocked' AND ended_at IS NULL),0),
           COALESCE(SUM(ended_at IS NOT NULL AND ended_at > $((NOW_MS - ERR_MS)) AND status = 'failed'),0)
    FROM flow_runs WHERE owner_key LIKE 'agent:$aid:%' OR owner_key = '$aid';")" || true

  # subagent_runs(requester_display_key 同前缀/纯 id):running / 近期 subagent-error
  read -r sr_run sr_err <<< "$(sqlite3 -separator ' ' -readonly "$DB" "
    SELECT COALESCE(SUM(ended_at IS NULL),0),
           COALESCE(SUM(ended_at IS NOT NULL AND ended_at > $((NOW_MS - ERR_MS)) AND ended_reason = 'subagent-error'),0)
    FROM subagent_runs WHERE requester_display_key LIKE 'agent:$aid:%' OR requester_display_key = '$aid';")" || true

  # 交互式会话:agents/<aid>/sessions/ 下 mtime 最新的活跃 jsonl,读尾部 message 的 role + stopReason
  # + 文件末事件类型(leaf?) + 尾部 6 条是否含 sessions_yield/spawn(协调态)
  role="-"; stop="-"; age="-"; last_event="-"; tail6_coord=0
  F=$(ls -t "$ROOT/agents/$aid/sessions/"*.jsonl 2>/dev/null \
        | grep -v -e '\.trajectory\.' -e '\.deleted' -e '\.bak' -e '\.reset' | head -1 || true)
  if [ -n "$F" ]; then
    mt=$(stat -f '%m' "$F"); age=$((NOW_S - mt))
    line=$(grep '"type":"message"' "$F" 2>/dev/null | tail -1 \
            | jq -r '[.message.role // "-", (.message.stopReason // "-")] | @tsv' 2>/dev/null || true)
    role=$(echo "$line" | cut -f1); stop=$(echo "$line" | cut -f2)
    last_event=$(tail -1 "$F" 2>/dev/null | jq -r '.type // "-"' 2>/dev/null || true)
    tail6_coord=$(tail -6 "$F" 2>/dev/null | grep -cE 'sessions_yield|sessions_spawn' || true)
    [ -z "$tail6_coord" ] && tail6_coord=0
  fi

  # 普通「在跑」(与 openclaw.rs session_running 一致):toolUse 跳闸门 / user|toolResult 需 fresh
  sess=0
  if [ "$stop" = "toolUse" ]; then
    sess=1                                            # toolUse 跳 mtime 闸门
  elif { [ "$role" = "user" ] || [ "$role" = "toolResult" ]; } \
       && [ "$age" != "-" ] && [ "$age" -lt "$SESSION_RECENT_S" ]; then
    sess=1
  fi
  # 协调态:leaf 结尾 ∧ 尾部含 yield/spawn(主 agent sessions_yield 等后台子 agent)
  if [ "$last_event" = "leaf" ] && [ "${tail6_coord:-0}" -gt 0 ]; then yl="Y"; else yl="-"; fi
  coord=0; [ "$yl" = "Y" ] && coord=1
  stale=0
  { [ "$age" != "-" ] && [ "$age" -ge "$SUBAGENT_WAIT_S" ]; } && stale=1
  runs=$((tr_run + fr_run + sr_run))   # run 表 running(子 agent 在跑)

  # classify:Error > NeedsDeci > Working > Done;协调态卡死 → Error
  err=$((tr_err + fr_err + sr_err))
  stuck=0
  if [ "$coord" -eq 1 ] && { [ "$runs" -eq 0 ] || [ "$stale" -eq 1 ]; }; then
    stuck=1                                          # B 子 agent 全 ended / A 超 30min 兜底
  fi
  st="Done 🟢"
  [ "$err" -gt 0 ] && st="Error 🔴"
  { [ "$err" -eq 0 ] && [ "$fr_blk" -gt 0 ]; } && st="NeedsDeci 🟠"
  { [ "$err" -eq 0 ] && [ "$stuck" -eq 1 ]; } && st="Error 🔴"
  { [ "$err" -eq 0 ] && [ "$stuck" -eq 0 ] && [ "$fr_blk" -eq 0 ] && [ "$((runs + sess))" -gt 0 ]; } && st="Working 🟡"

  printf '%-8s task_r=%-2s flow_r=%-2s flow_blk=%-2s sub_r=%-2s sess=%s | role=%-10s stop=%-7s yl=%-2s age=%-3s → %s\n' \
    "$aid" "${tr_run:-0}" "${fr_run:-0}" "${fr_blk:-0}" "${sr_run:-0}" "$sess" "$role" "$stop" "$yl" "$age" "$st"
done
