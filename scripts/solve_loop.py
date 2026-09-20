#!/usr/bin/env python3
"""循环自动解题（外层循环版）：原样调用现有脚本，不修改、不重写它们。

每轮按顺序独立运行（每个脚本独立进程、独立 USB 隧道，与手动运行节奏完全一致）：

    1. scripts/100.py            求解当前题：截图 → queens-puzzle analyze → 落子
    2. scripts/tap_next.py       点击“下一题”（橙色.png，左下区域）
    3. scripts/find_00_region.py 区域校验 00.png（region=328,256,408,319，阈值 0.98）
         退出码 0（在指定位置找到）→ 继续下一轮求解
         退出码 1（未找到）       → 停止，不再继续求解

停止条件（任一满足）：
    - 区域校验未找到 00.png（约定的停止条件，正常退出码 0）；
    - 100.py / tap_next.py 任一失败（识别失败、按钮未出现、设备/隧道问题）；
    - 达到 --max-rounds 上限；Ctrl+C。

用法（需在装有 Pillow 的 Python 下运行，如 python3.11）：
    python3.11 scripts/solve_loop.py                 # 循环直到 00.png 校验失败
    python3.11 scripts/solve_loop.py --max-rounds 3  # 最多求解 3 轮
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent
PYTHON = sys.executable          # 循环器跑在哪个解释器下，子脚本就用同一个（需自带 Pillow）

# 子进程需要能 import asclient：把库源码目录放进 PYTHONPATH（原脚本本身没有该 fallback）
ASCLIENT_SRC = os.environ.get("ASCLIENT_SRC", str(Path.home() / "Developer" / "automation"))

SOLVE_SCRIPT = "scripts/100.py"
NEXT_SCRIPT = "scripts/tap_next.py"
CHECK_SCRIPT = "scripts/find_00_region.py"


def child_env() -> dict[str, str]:
    """复制当前环境并追加 PYTHONPATH，让子脚本能找到 asclient 源码。"""
    env = os.environ.copy()
    if Path(ASCLIENT_SRC, "asclient").is_dir():
        existing = env.get("PYTHONPATH", "")
        env["PYTHONPATH"] = f"{ASCLIENT_SRC}{os.pathsep}{existing}" if existing else ASCLIENT_SRC
    else:
        print(f"[警告] {ASCLIENT_SRC} 下没有 asclient 包，子脚本可能 import 失败。", file=sys.stderr)
    return env


def run_step(description: str, script: str) -> int:
    """按原样运行一个脚本（工作目录=项目根，输出实时透传），返回退出码。"""
    print(f"\n――― {description}：`{PYTHON} {script}` ―――", flush=True)
    result = subprocess.run([PYTHON, script], cwd=PROJECT_ROOT, env=child_env())
    return result.returncode


def main() -> int:
    parser = argparse.ArgumentParser(description="外层循环：100.py → tap_next.py → 区域校验 00.py → 继续/停止")
    parser.add_argument("--max-rounds", type=int, default=0,
                        help="最多求解几轮，0 表示不限制（默认：0）")
    args = parser.parse_args()

    for script in (SOLVE_SCRIPT, NEXT_SCRIPT, CHECK_SCRIPT):
        if not (PROJECT_ROOT / script).is_file():
            print(f"[错误] 找不到脚本：{PROJECT_ROOT / script}", file=sys.stderr)
            return 2

    round_no = 0
    try:
        while True:
            round_no += 1
            print(f"\n================ 第 {round_no} 轮 ================")

            # 1. 求解当前题（100.py 原样运行）
            if run_step(f"第 {round_no} 轮求解", SOLVE_SCRIPT) != 0:
                print(f"\n[停止] {SOLVE_SCRIPT} 失败，不再继续（共完成 {round_no - 1} 轮）。", file=sys.stderr)
                return 1

            if args.max_rounds and round_no >= args.max_rounds:
                print(f"\n[停止] 已达 --max-rounds={args.max_rounds} 上限，共求解 {round_no} 轮。")
                return 0

            # 2. 点击“下一题”（tap_next.py 原样运行）
            if run_step("点击下一题", NEXT_SCRIPT) != 0:
                print(f"\n[停止] {NEXT_SCRIPT} 失败，不再继续（共完成 {round_no} 轮）。", file=sys.stderr)
                return 1

            # 3. 点击下一题之后、求解之前：区域校验 00.png（find_00_region.py 原样运行）
            code = run_step("区域校验 00.png", CHECK_SCRIPT)
            if code == 0:
                print("\n[校验通过] 00.png 仍在指定位置，继续下一轮求解。")
                continue
            print(f"\n[停止] 区域内未找到 00.png（退出码 {code}），按约定不再继续求解（共完成 {round_no} 轮）。")
            return 0
    except KeyboardInterrupt:
        print(f"\n[中断] 手动停止，共求解 {round_no} 轮。", file=sys.stderr)
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
