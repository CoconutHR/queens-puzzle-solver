"""Python 调用示例：把图片字节直接传给 Rust 二进制，不落盘。

用法：
    python scripts/analyze_screenshot.py <图片路径>
    # 或作为库使用：analyze_screenshot(png_bytes)

设计要点（面向风控敏感的企业环境）：
- 用 subprocess 而非 PyO3：崩溃只影响子进程，Python 主进程存活，可捕获退出码与 stderr
- 图片字节经 stdin 传入，不写临时文件
- 带超时，避免异常图片导致挂起
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

DEFAULT_BINARY = os.environ.get("QUEENS_PUZZLE_BIN", "queens-puzzle")


class AnalyzeError(RuntimeError):
    """解析失败。保留退出码与 stderr 便于排查，而不是让进程崩掉。"""


def analyze_screenshot(
    data: bytes,
    binary: str = DEFAULT_BINARY,
    timeout: float = 30.0,
) -> dict[str, Any]:
    """把图片字节传给二进制，返回解析结果。

    Args:
        data: 图片的原始字节（PNG / JPEG / WebP）。
        binary: 二进制路径，默认从 PATH 查找。
        timeout: 超时秒数。

    Returns:
        含 size / board / regions / cells / solution / difficulty / palette 的字典。

    Raises:
        AnalyzeError: 二进制返回非零退出码或超时。
    """
    try:
        proc = subprocess.run(
            [binary, "analyze", "-"],
            input=data,          # 字节直传 stdin，不落盘
            capture_output=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as exc:
        raise AnalyzeError(f"解析超时（{timeout}s）") from exc
    except FileNotFoundError as exc:
        raise AnalyzeError(f"找不到二进制 {binary!r}，请先 cargo install --path .") from exc

    if proc.returncode != 0:
        reason = proc.stderr.decode("utf-8", errors="replace").strip()
        raise AnalyzeError(f"解析失败（exit={proc.returncode}）: {reason}")

    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise AnalyzeError(f"返回内容不是合法 JSON: {exc}") from exc


def queen_pixels(result: dict[str, Any]) -> list[tuple[int, int]]:
    """解对应的像素中心坐标，便于驱动自动点击。"""
    cells = result["cells"]
    return [(cells[row][col]["x"], cells[row][col]["y"]) for row, col in result["solution"]]


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2

    path = Path(sys.argv[1])
    if not path.is_file():
        print(f"文件不存在: {path}", file=sys.stderr)
        return 2

    result = analyze_screenshot(path.read_bytes())

    print(f"棋盘: {result['size']}x{result['size']}  "
          f"区域: {result['board']}")
    print(f"难度: {result['difficulty']}")
    print(f"解（行列）: {result['solution']}")
    print(f"解（像素中心，可直接点击）: {queen_pixels(result)}")
    print("\n区域矩阵:")
    for row in result["regions"]:
        print("  " + " ".join(f"{v:>2}" for v in row))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
