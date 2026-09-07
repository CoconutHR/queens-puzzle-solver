"""Queens 自动解题（USB 隧道版）：进程内管理 iproxy，截图 → analyze → 点击。

用法：
    python queens_usb.py
    python queens_usb.py --udid 00008120-XXXXXXXXXXXXXXXX        # 多设备时锁定目标
    python queens_usb.py --local-port 9097 --no-logs             # 端口冲突时避让
    python queens_usb.py --dry-run                               # 只识别，不点击

前置条件：
    * iproxy 已安装（本机 /opt/homebrew/bin/iproxy），可用 `asc doctor` 确认；
    * 手机已 USB 连接、解锁并信任本机；
    * 本机 9096 / 10102 未被其它进程占用（否则改用 --local-port / --no-logs）。
"""
from __future__ import annotations

import argparse
import json
import shutil
import signal
import subprocess
import sys
import time
from typing import Any

from asclient import connect
from asclient.config import device_options, load_config
from asclient.errors import AScriptError, IProxyNotFoundError, TunnelError
from asclient.tunnel import AScriptTunnel

DEFAULT_SOLVER = "../target/release/queens-puzzle"
ANALYZE_TIMEOUT = 30.0
TAP_DELAY = 0.35
READY_TIMEOUT = 15.0
# 棋盘提示框（物理像素，左上包含、右下排除）。只作搜索提示，可略大于棋盘；
# 求解器返回的 board / cells 始终是原图坐标，点击时不需要加偏移。
# 设为 None 则由求解器全图检测。
BOARD_CROP: tuple[int, int, int, int] | None = (0, 710, 1179, 1880)


class AnalyzeError(RuntimeError):
    """求解器调用失败：含退出码与 stderr，便于排查。"""

    def __init__(self, message: str, *, returncode: int | None = None, stderr: str = "") -> None:
        super().__init__(message)
        self.returncode = returncode
        self.stderr = stderr


def resolve_solver(candidate: str) -> str:
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
    """把 solution 的 [row, col] 映射成可直接 tap 的物理像素坐标。"""
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


def _stop_on_sigterm(signum: int, frame: object) -> None:
    """让 SIGTERM 也走正常清理路径，避免 iproxy 子进程残留。"""
    raise KeyboardInterrupt


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="USB 隧道下识别 Queens 棋盘并自动落子")
    parser.add_argument("--config", help="asclient.json 路径，默认取当前目录")
    parser.add_argument("--device", help="覆盖连接地址；USB 场景一般不需要（默认用隧道本地地址）")
    parser.add_argument("--password", help="覆盖 device.password")
    parser.add_argument("--udid", help="多设备时锁定目标手机")
    parser.add_argument("--local-port", type=int, help="本地控制端口，默认 9096")
    parser.add_argument("--local-log-port", type=int, help="本地日志端口，默认 10102")
    parser.add_argument("--iproxy", help="iproxy 绝对路径，未加入 PATH 时使用")
    parser.add_argument("--no-logs", action="store_true", help="不转发 10102，端口冲突时可用")
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


    overrides: dict[str, Any] = {}
    for key, value in (
        ("local_port", args.local_port),
        ("local_log_port", args.local_log_port),
        ("udid", args.udid),
        ("iproxy", args.iproxy),
    ):
        if value:
            overrides[key] = value
    if args.no_logs:
        overrides["forward_logs"] = False

    try:
        tunnel = AScriptTunnel.from_config(args.config, **overrides)
    except ValueError as exc:
        print(f"隧道参数错误：{exc}", file=sys.stderr)
        return 4

    previous_sigterm = signal.signal(signal.SIGTERM, _stop_on_sigterm)
    try:
        with tunnel:                                   # 进入时 start()，离开时 stop()
            # 关键：USB 场景下必须用隧道本地地址，不能沿用配置里的 device.address
            # （tunnel 只做端口映射，不会改写该配置；Wi-Fi 地址在 USB 下不可达）。
            address = args.device or tunnel.address
            routes = f"{tunnel.address} -> 设备:{tunnel.remote_port}"
            if tunnel.log_address:
                routes += f"；日志 {tunnel.log_address} -> 设备:{tunnel.remote_log_port}"
            print(f"USB 隧道已就绪：{routes}")

            device = connect(address, password=password)
            client = device.client
            status = wait_until_ready(client)
            print(f"设备可用：{status.get('screen') or status.get('logical_screen')}")

            with client.locked():
                png = client.screenshot()              # 无损 PNG，不用 HID JPEG
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
            print(f"待点击 {len(points)} 个格子：{points}")

            if args.dry_run:
                print("--dry-run：已跳过点击。")
                return 0

            with client.locked():
                for index, (x, y) in enumerate(points, start=1):
                    client.double_tap(x, y, interval=0.01)                   # 与截图同一物理像素坐标系
                    print(f"[{index}/{len(points)}] tap ({x}, {y})")
                    if index < len(points):
                        time.sleep(args.tap_delay)
            print("完成。")
            return 0

    except (IProxyNotFoundError, TunnelError) as exc:
        print(f"USB 隧道失败：{exc}", file=sys.stderr)
        print("排查：iproxy 是否已安装、手机是否已解锁信任、本机 9096/10102 是否被占用。", file=sys.stderr)
        return 5
    except AnalyzeError as exc:
        print(f"识别失败：{exc}", file=sys.stderr)
        if exc.stderr:
            print(f"求解器 stderr（退出码 {exc.returncode}）：\n{exc.stderr}", file=sys.stderr)
        return 2
    except AScriptError as exc:
        print(f"设备操作失败：{exc}", file=sys.stderr)
        return 3
    except KeyboardInterrupt:
        print("已中断，隧道正在关闭。", file=sys.stderr)
        return 130
    except (FileNotFoundError, ValueError, KeyError, IndexError) as exc:
        print(f"参数或数据错误：{exc}", file=sys.stderr)
        return 4
    finally:
        signal.signal(signal.SIGTERM, previous_sigterm)


if __name__ == "__main__":
    raise SystemExit(main())
