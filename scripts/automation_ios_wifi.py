"""Queens 自动解题（Wi-Fi 直连版）：截图 → analyze → 点击。

用法：
    python automation_ios.py
    python automation_ios.py --device 192.168.3.17:9096          # 覆盖默认无线地址
    python automation_ios.py --dry-run                           # 只识别，不点击

前置条件：
    * iPhone 与本机处于可互访的网络，AScript 服务已启动；
    * 默认设备地址为 192.168.3.17:9096。
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from asclient import connect
from asclient.config import device_options, load_config
from asclient.errors import AScriptError

PROJECT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SOLVER = str(
    PROJECT_ROOT / "target" / "release" / ("queens-puzzle.exe" if os.name == "nt" else "queens-puzzle")
)
DEFAULT_DEVICE_ADDRESS = "192.168.3.17:9096"
ANALYZE_TIMEOUT = 30.0
TAP_DELAY = 0.35
READY_TIMEOUT = 15.0
# 棋盘提示框（物理像素，左上包含、右下排除）。只作搜索提示，可略大于棋盘；
# 裁剪框仅限制棋盘搜索范围；求解器输出的 cells 已是整张截图中的
# 物理像素坐标，因此点击时不得再次叠加裁剪框左上偏移。
# 设为 None 则由求解器全图检测。
BOARD_CROP: tuple[int, int, int, int] | None = (0, 710, 1179, 1880)


class AnalyzeError(RuntimeError):
    """求解器调用失败：含退出码与 stderr，便于排查。"""

    def __init__(self, message: str, *, returncode: int | None = None, stderr: str = "") -> None:
        super().__init__(message)
        self.returncode = returncode
        self.stderr = stderr


def resolve_solver(candidate: str) -> str:
    candidate_path = Path(candidate).expanduser()
    if candidate_path.is_file():
        return str(candidate_path.resolve())
    found = shutil.which(candidate)
    if found is None:
        raise FileNotFoundError(
            f"找不到求解器 {candidate!r}；请先 cargo install --path . "
            f"或用 --solver 指定可执行文件的绝对路径"
        )
    return found


def analyze_bytes(solver: str, image: bytes, *, crop: tuple[int, int, int, int] | None = None,
                  timeout: float = ANALYZE_TIMEOUT) -> dict[str, Any]:
    """把 PNG 字节经 stdin 送进 `analyze -`，返回解析后的 JSON。

    crop 为可选搜索提示框（left, top, right, bottom）。它只缩小检测范围，
    返回坐标仍是原图坐标系；框内找不到棋盘时求解器自动回退全图检测。
    """
    command = [solver, "analyze"]
    if crop:
        command += ["--crop", ",".join(str(int(value)) for value in crop)]
    command.append("-")                        # `--crop` 必须在 `-` 之前
    try:
        proc = subprocess.run(
            command,
            input=image,                       # 必须是 bytes，不要 text=True
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as exc:
        raise AnalyzeError(f"求解器超时（>{timeout}s）") from exc

    if proc.returncode != 0:
        raise AnalyzeError(
            "求解器执行失败",
            returncode=proc.returncode,
            stderr=proc.stderr.decode("utf-8", "replace").strip(),
        )
    try:
        result = json.loads(proc.stdout.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AnalyzeError(
            "求解器输出不是合法 JSON",
            returncode=proc.returncode,
            stderr=proc.stdout[:500].decode("utf-8", "replace"),
        ) from exc
    if not isinstance(result, dict):
        raise AnalyzeError(f"求解器返回了非对象 JSON：{type(result).__name__}")
    return result


def queen_pixels(result: dict[str, Any]) -> list[tuple[int, int]]:
    """把 solution 的 [row, col] 映射成整张截图中的点击坐标。"""
    cells = result.get("cells") or []
    solution = result.get("solution")
    if not solution:
        return []
    return [(int(cells[row][col]["x"]), int(cells[row][col]["y"])) for row, col in solution]


def wait_until_ready(client: Any, *, timeout: float = READY_TIMEOUT) -> dict[str, Any]:
    """隧道建立后，设备服务可能还要一两秒才可用，轮询等待。"""
    deadline = time.monotonic() + timeout
    last: Exception | None = None
    while time.monotonic() < deadline:
        try:
            return client.status()
        except AScriptError as exc:          # 连接层异常，重试；业务错误不重试
            last = exc
            time.sleep(0.5)
    raise AScriptError(f"隧道已建立，但设备服务在 {timeout}s 内不可用：{last}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="通过 Wi-Fi 识别 Queens 棋盘并自动落子")
    parser.add_argument("--config", help="asclient.json 路径，默认取当前目录")
    parser.add_argument(
        "--device",
        default=DEFAULT_DEVICE_ADDRESS,
        help=f"无线设备地址（默认 {DEFAULT_DEVICE_ADDRESS}）",
    )
    parser.add_argument("--password", help="覆盖 device.password")
    parser.add_argument("--solver", default=DEFAULT_SOLVER)
    parser.add_argument("--crop", help="临时覆盖棋盘提示框，格式 左,上,右,下，例如 0,710,1179,1880")
    parser.add_argument("--tap-delay", type=float, default=TAP_DELAY, help="每次点击后的间隔秒数")
    parser.add_argument("--dry-run", action="store_true", help="只识别并打印计划，不点击")
    args = parser.parse_args(argv)

    options = device_options(load_config(args.config))
    password = args.password if args.password is not None else options.get("password", "")
    solver = resolve_solver(args.solver)
    crop: tuple[int, int, int, int] | None = BOARD_CROP
    if args.crop:
        parts = [part.strip() for part in args.crop.split(",")]
        if len(parts) != 4:
            print("--crop 需要四个整数：左,上,右,下", file=sys.stderr)
            return 4
        crop = tuple(int(part) for part in parts)  # type: ignore[assignment]

    try:
        print(f"正在通过 Wi-Fi 连接设备：{args.device}")
        device = connect(args.device, password=password)
        client = device.client
        status = wait_until_ready(client)
        print(f"设备可用：{status.get('screen') or status.get('logical_screen')}")

        with client.locked():
            png = client.screenshot()                  # 无损 PNG，不用 HID JPEG
        result = analyze_bytes(solver, png, crop=crop)

        size, difficulty, board = result.get("size"), result.get("difficulty"), result.get("board")
        print(f"棋盘 {size}x{size}  难度 {difficulty or '无评级'}  区域 {board}")
        if BOARD_CROP and board:
            cl, ct, cr, cb = BOARD_CROP
            if not (cl <= board.get("left", 0) and ct <= board.get("top", 0)
                    and board.get("right", 0) <= cr and board.get("bottom", 0) <= cb):
                print("提示：棋盘不在提示框内，可能已回退全图检测；如属预期可忽略。", file=sys.stderr)

        points = queen_pixels(result)
        if not points:
            print("求解器未给出解（无解或多解），不执行点击。", file=sys.stderr)
            return 1
        print(f"待点击 {len(points)} 个格子（整张截图坐标）：{points}")

        if args.dry_run:
            print("--dry-run：已跳过点击。")
            return 0

        with client.locked():
            for index, (x, y) in enumerate(points, start=1):
                # Queens 棋盘每次点击会切换格子状态；必须单击。双击会
                # 在同一格执行两次切换，最终回到未选中状态，看起来像“没有点击”。
                client.double_tap(x, y, interval=0.01)                # 与截图同一物理像素坐标系
                print(f"[{index}/{len(points)}] tap ({x}, {y})")
                if index < len(points):
                    time.sleep(args.tap_delay)
        print("完成。")
        return 0

    except AnalyzeError as exc:
        print(f"识别失败：{exc}", file=sys.stderr)
        if exc.stderr:
            print(f"求解器 stderr（退出码 {exc.returncode}）：\n{exc.stderr}", file=sys.stderr)
        return 2
    except AScriptError as exc:
        print(f"设备操作失败：{exc}", file=sys.stderr)
        print("排查：确认 iPhone 与本机网络可互访，且 AScript 服务已启动、地址和密码正确。", file=sys.stderr)
        return 3
    except KeyboardInterrupt:
        print("已中断。", file=sys.stderr)
        return 130
    except (FileNotFoundError, ValueError, KeyError, IndexError) as exc:
        print(f"参数或数据错误：{exc}", file=sys.stderr)
        return 4
if __name__ == "__main__":
    raise SystemExit(main())
