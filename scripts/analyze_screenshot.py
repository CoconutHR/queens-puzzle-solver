"""Python 调用示例。

两种输入方式，按场景选：

1. 传字节（图片已在内存）：analyze_screenshot(png_bytes)
   字节经 stdin 送入，不落盘。
2. 传路径（图片在磁盘，离线）：analyze_file("shot.png")
   直接把路径作为参数传给二进制。

命令行：
    python scripts/analyze_screenshot.py <图片路径>

设计要点（面向风控敏感的企业环境）：
- 用 subprocess 而非 PyO3：崩溃只影响子进程，Python 主进程存活，可捕获退出码与 stderr
- 带超时，避免异常图片导致挂起
- 失败时二进制返回非零码 + stderr，这里转成 AnalyzeError 抛出
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


def _run(
    args: list[str],
    *,
    input_data: bytes | None,
    binary: str,
    timeout: float,
    crop: tuple[int, int, int, int] | None,
) -> dict[str, Any]:
    """执行二进制并把 stdout 解析为 JSON。"""
    cmd = [binary, "analyze"]
    if crop is not None:
        if len(crop) != 4:
            raise AnalyzeError(f"crop 需要 4 个值 (left, top, right, bottom)，收到 {crop!r}")
        cmd += ["--crop", ",".join(str(int(v)) for v in crop)]
    cmd += args
    try:
        proc = subprocess.run(
            cmd,
            input=input_data,
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


def analyze_screenshot(
    data: bytes,
    binary: str = DEFAULT_BINARY,
    timeout: float = 30.0,
    crop: tuple[int, int, int, int] | None = None,
) -> dict[str, Any]:
    """把图片字节传给二进制（经 stdin，不落盘），返回解析结果。

    Args:
        data: 图片的原始字节（PNG / JPEG / WebP）。
        binary: 二进制路径，默认从 PATH 查找。
        timeout: 超时秒数。
        crop: 可选的 (left, top, right, bottom) 提示框，只在框内搜索棋盘。
              不需要精确等于棋盘，检测仍在框内进行。返回的坐标始终是原图坐标。

    Returns:
        含 size / board / regions / cells / solution / difficulty / palette 的字典。

    Raises:
        AnalyzeError: 二进制返回非零退出码或超时。
    """
    return _run(["-"], input_data=data, binary=binary, timeout=timeout, crop=crop)


def analyze_file(
    path: str | Path,
    binary: str = DEFAULT_BINARY,
    timeout: float = 30.0,
    crop: tuple[int, int, int, int] | None = None,
) -> dict[str, Any]:
    """把图片路径直接传给二进制（离线使用，不经 stdin），返回解析结果。

    适合图片已经在磁盘上的场景（例如刚保存的截图），比读成字节再传更省事。

    Args:
        path: 图片文件路径。
        binary: 二进制路径，默认从 PATH 查找。
        timeout: 超时秒数。
        crop: 可选的 (left, top, right, bottom) 提示框，只在框内搜索棋盘。

    Raises:
        AnalyzeError: 二进制返回非零退出码或超时，或文件不存在。
    """
    p = Path(path)
    if not p.is_file():
        raise AnalyzeError(f"文件不存在: {p}")
    return _run([str(p)], input_data=None, binary=binary, timeout=timeout, crop=crop)


def queen_pixels(result: dict[str, Any]) -> list[tuple[int, int]]:
    """解对应的像素中心坐标，便于驱动自动点击。"""
    cells = result["cells"]
    return [(cells[row][col]["x"], cells[row][col]["y"]) for row, col in result["solution"]]


def main() -> int:
    args = sys.argv[1:]
    crop = None
    if "--crop" in args:
        i = args.index("--crop")
        try:
            spec = args[i + 1]
            crop = tuple(int(v) for v in spec.split(","))
            if len(crop) != 4:
                raise ValueError
            del args[i : i + 2]
        except (IndexError, ValueError):
            print("--crop 需要形如 0,700,1178,1900 的四个数字", file=sys.stderr)
            return 2

    if len(args) != 1:
        print(__doc__)
        return 2

    path = Path(args[0])
    if not path.is_file():
        print(f"文件不存在: {path}", file=sys.stderr)
        return 2

    # 用路径模式（离线）。若要演示字节模式，改成 analyze_screenshot(path.read_bytes(), crop=crop)
    result = analyze_file(path, crop=crop)

    print(f"棋盘: {result['size']}x{result['size']}  "
          f"区域: {result['board']}" + (f"  (crop={crop})" if crop else ""))
    print(f"难度: {result['difficulty']}")
    print(f"解（行列）: {result['solution']}")
    print(f"解（像素中心，可直接点击）: {queen_pixels(result)}")
    print("\n区域矩阵:")
    for row in result["regions"]:
        print("  " + " ".join(f"{v:>2}" for v in row))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
