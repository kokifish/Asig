#!/usr/bin/env python3
"""Preview every AgentStatus default light animation on the Asig overlay.

Launches the latest build in a dev mode (ASIG_PREVIEW=1) that cycles the
floating signal light through each state's DEFAULT animation — Done, DoneNotif,
Working, NeedsDeci, Error, Offline — one per tick (~3 s), looping forever. The
app prints the current state to stdout. Press Ctrl-C to stop.

Usage:
    python3 scripts/preview_anims.py        # or: ./scripts/preview_anims.py
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

APP = Path(__file__).resolve().parent.parent / "build/Asig.app/Contents/MacOS/agent-light"

# Order matches the app's preview cycle (settings tab order). Bilingual hint.
STATES = [
    "Done · 波纹 · 绿",
    "DoneNotif · 快呼吸 · 浅蓝",
    "Working · 呼吸 · 黄",
    "NeedsDeci · 慢闪 · 琥珀",
    "Error · 快闪 · 红",
    "Offline · 常亮 · 紫",
]


def main() -> int:
    """Run the app in preview mode; return process exit code."""
    if not APP.exists():
        print(f"app not built: {APP}\nrun ./scripts/make-app.sh first", file=sys.stderr)
        return 1
    print("Asig 默认动效预览:循环展示各状态默认灯效(每个 ~3s)。Ctrl-C 退出。")
    for i, s in enumerate(STATES, 1):
        print(f"  {i}. {s}")
    print("-" * 48)
    env = {**os.environ, "ASIG_PREVIEW": "1"}
    try:
        return subprocess.call([str(APP)], env=env)
    except KeyboardInterrupt:
        print("\nstopped")
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
