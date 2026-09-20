#!/usr/bin/env python3
"""Queens 自动解题循环（USB 隧道版）。

每轮流程：
    1) 轮询截图 + analyze，直到出现**新棋盘**
    2) 点击解出的格子
    3) 固定等待胜利动画播完
    4) 点击“下一题”
    5) 轮询查找目标数字：找到 → 停止

轮询 + 状态机（取代大部分固定 sleep）：
    - 求解器报 detect_failed        → 棋盘没就绪 → 继续截图轮询
    - 识别到新棋盘（regions 变了）   → 正常求解
    - 识别到与上次相同的棋盘         → “下一题”没点到 → 再点一次
    - 一直认不出、超时              → app 可能卡住 → 点一次“下一题”兜底

固定等待保留在“胜利动画”这一处：
    全部落子后，棋盘会播放约 2 秒的完成动画；动画期间点“下一题”会失效或误触。
    时长由 --post-solve-wait 控制。
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Literal

from asclient import connect
from asclient.config import device_options, load_config
from asclient.errors import AScriptError, IProxyNotFoundError, TunnelError
from asclient.tunnel import AScriptTunnel

# ――― 求解器 ―――
PROJECT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SOLVER = str(
    PROJECT_ROOT / "target" / "release"
    / ("queens-puzzle.exe" if os.name == "nt" else "queens-puzzle")
)
ANALYZE_TIMEOUT = 30.0
READY_TIMEOUT = 15.0

# 棋盘提示框（物理像素，左上含、右下不含）；None 则全图检测
BOARD_CROP: tuple[int, int, int, int] | None = (0, 710, 1179, 1880)

# ――― 停止条件：目标数字模板 ―――
NUMBER_TEMPLATE_NAME = "00.png"
NUMBER_REGION = (328, 256, 408, 319)          # 物理像素：左上含、右下不含
NUMBER_CONFIDENCE = 0.98

# ――― “下一题”按钮 ―――
NEXT_TEMPLATE = "scripts/橙色.png"
NEXT_CONFIDENCE = 0.98
NEXT_TIMEOUT = 5.0
NEXT_REGION_RELATIVE = (0, 0.6, 0.5, 0.9)     # 相对比例：左、上、右、下

# ――― 节奏（秒）―――
TAP_DELAY = 0.35               # 落子之间的点击间隔
POST_SOLVE_WAIT = 5.5          # 胜利动画时长（固定硬等）
DEFAULT_POLL_INTERVAL = 0.4    # 两次截图之间的间隔
DEFAULT_BOARD_TIMEOUT = 5.0   # 等待新棋盘的总超时
DEFAULT_NUMBER_TIMEOUT = 3.0   # 等待目标数字的总超时


class AnalyzeError(RuntimeError):
    """求解器调用失败：带 kind / 退出码 / stderr，便于分类处理。"""

    def __init__(
        self,
        message: str,
        *,
        kind: str | None = None,
        returncode: int | None = None,
        stderr: str = "",
    ) -> None:
        super().__init__(message)
        self.kind = kind
        self.returncode = returncode
        self.stderr = stderr


# ---------------------------------------------------------------------------
# 求解器交互
# ---------------------------------------------------------------------------

def resolve_solver(candidate: str) -> str:
    candidate_path = Path(candidate).expanduser()
    if candidate_path.is_file():
        return str(candidate_path.resolve())
    found = shutil.which(candidate)
    if found is None:
        raise FileNotFoundError(
            f"找不到求解器 {candidate!r}；请先 cargo install --path . 或用 --solver 指定绝对路径"
        )
    return found


def analyze_bytes(
    solver: str,
    image: bytes,
    *,
    crop: tuple[int, int, int, int] | None = None,
    timeout: float = ANALYZE_TIMEOUT,
) -> dict[str, Any]:
    """把 PNG 字节经 stdin 送进 `analyze -`，返回信封里的 `data`。

    成功 → 返回 data（dict）
    失败 → 抛 AnalyzeError（kind 来自求解器的 error.kind）
    """
    command = [solver, "analyze"]
    if crop:
        command += ["--crop", ",".join(str(int(v)) for v in crop)]
    command.append("-")

    try:
        proc = subprocess.run(
            command,
            input=image,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as exc:
        raise AnalyzeError(f"求解器超时（>{timeout}s）") from exc

    stdout_text = proc.stdout.decode("utf-8", "replace").strip()
    try:
        envelope = json.loads(stdout_text)
    except json.JSONDecodeError as exc:
        raise AnalyzeError(
            "求解器输出不是合法 JSON",
            returncode=proc.returncode,
            stderr=stdout_text[:500],
        ) from exc

    if not isinstance(envelope, dict) or "ok" not in envelope:
        raise AnalyzeError(
            f"求解器返回的信封格式异常：{envelope!r}",
            returncode=proc.returncode,
        )

    if not envelope["ok"]:
        err = envelope.get("error") or {}
        raise AnalyzeError(
            err.get("message", "求解器报告失败"),
            kind=err.get("kind"),
            returncode=proc.returncode,
            stderr=proc.stderr.decode("utf-8", "replace").strip(),
        )

    data = envelope.get("data")
    if not isinstance(data, dict):
        raise AnalyzeError("求解器返回的 data 不是对象", returncode=proc.returncode)
    return data


# ---------------------------------------------------------------------------
# 设备操作
# ---------------------------------------------------------------------------

def screenshot(client: Any) -> bytes:
    with client.locked():
        return client.screenshot()


def queen_pixels(result: dict[str, Any]) -> list[tuple[int, int]]:
    """把 solution 的 [row, col] 映射成整张截图中的点击坐标。"""
    cells = result.get("cells") or []
    solution = result.get("solution")
    if not solution:
        return []
    return [(int(cells[r][c]["x"]), int(cells[r][c]["y"])) for r, c in solution]


def tap_points(client: Any, points: list[tuple[int, int]], tap_delay: float) -> None:
    with client.locked():
        for index, (x, y) in enumerate(points, start=1):
            # 单击；双击会在同格切换两次状态。与截图同一物理像素坐标系。
            client.double_tap(x, y, interval=0.01)
            print(f"  [{index}/{len(points)}] tap ({x}, {y})")
            if index < len(points):
                time.sleep(tap_delay)


def tap_next(client: Any) -> None:
    """点击“下一题”按钮；失败向外抛 AScriptError。"""
    client.tap_image(
        NEXT_TEMPLATE,
        confidence=NEXT_CONFIDENCE,
        timeout=NEXT_TIMEOUT,
        region_relative=NEXT_REGION_RELATIVE,
    )
    print("已点击下一题")


def safe_tap_next(client: Any) -> bool:
    """点击“下一题”的容错版：失败只打警告，不抛异常。"""
    try:
        tap_next(client)
        return True
    except AScriptError as exc:
        print(f"[警告] 点击“下一题”失败：{exc}", file=sys.stderr)
        return False


def find_number(client: Any, template_path: Path) -> Any:
    """在限定区域内查找目标数字；返回 match 或 None。"""
    return client.find_image(
        str(template_path),
        confidence=NUMBER_CONFIDENCE,
        region=NUMBER_REGION,
    )


def wait_until_ready(client: Any, *, timeout: float = READY_TIMEOUT) -> dict[str, Any]:
    """隧道建立后，设备服务可能还要一两秒才可用，轮询等待。"""
    deadline = time.monotonic() + timeout
    last: Exception | None = None
    while time.monotonic() < deadline:
        try:
            return client.status()
        except AScriptError as exc:
            last = exc
            time.sleep(0.5)
    raise AScriptError(f"隧道已建立，但设备服务在 {timeout}s 内不可用：{last}")


# ---------------------------------------------------------------------------
# 轮询：等棋盘 / 等目标数字
# ---------------------------------------------------------------------------

BoardStatus = Literal["new", "same", "no_board"]


def poll_board(
    client: Any,
    solver: str,
    crop: tuple[int, int, int, int] | None,
    *,
    previous_regions: list[list[int]] | None,
    timeout: float,
    poll_interval: float,
    same_threshold: int = 2,
) -> tuple[BoardStatus, dict[str, Any] | None]:
    """轮询截图 + 识别，直到出现符合期望的棋盘或超时。

    返回 (status, result)：
        ("new",  result) —— 出现了新棋盘（previous_regions 为 None 时首次识别即可）
        ("same", result) —— 连续 same_threshold 次识别到与上次相同的棋盘
        ("no_board", None) —— 一直 detect_failed 或超时
    """
    deadline = time.monotonic() + timeout
    detect_fails = 0
    same_streak = 0

    while time.monotonic() < deadline:
        try:
            png = screenshot(client)
            result = analyze_bytes(solver, png, crop=crop)
        except AnalyzeError as exc:
            if exc.kind == "detect_failed":
                # 棋盘没就绪 —— 这就是取代固定 sleep 的关键路径
                detect_fails += 1
                time.sleep(poll_interval)
                continue
            # decode_error / io_error 之类是硬错误，直接上抛
            raise

        # 识别成功
        regions = result.get("regions")
        if previous_regions is None or regions != previous_regions:
            return ("new", result)

        same_streak += 1
        if same_streak >= same_threshold:
            return ("same", result)

        time.sleep(poll_interval)

    if detect_fails:
        print(f"  [调试] {timeout}s 内 {detect_fails} 次 detect_failed", file=sys.stderr)
    return ("no_board", None)


def poll_number(
    client: Any,
    template_path: Path,
    *,
    timeout: float,
    poll_interval: float,
) -> Any:
    """轮询查找目标数字；命中返回 match，超时返回 None。"""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            match = find_number(client, template_path)
        except AScriptError as exc:
            print(f"[警告] find_image 失败：{exc}", file=sys.stderr)
            return None
        if match is not None:
            return match
        time.sleep(poll_interval)
    return None


# ---------------------------------------------------------------------------
# 主循环
# ---------------------------------------------------------------------------

def run_loop(
    client: Any,
    solver: str,
    crop: tuple[int, int, int, int] | None,
    number_template: Path,
    args: argparse.Namespace,
) -> int:
    """返回进程退出码：0 = 找到目标数字，1 = 达最大轮数，2 = 无法恢复。"""
    last_regions: list[list[int]] | None = None
    round_num = 0

    while round_num < args.max_rounds:
        # ――― 1. 轮询等棋盘（取代固定 sleep）―――
        status, result = poll_board(
            client, solver, crop,
            previous_regions=last_regions,
            timeout=args.board_timeout,
            poll_interval=args.poll_interval,
        )

        if status == "same":
            # 求解器明确识别出“还是上一盘” —— 说明“下一题”没点到
            print("[警告] 点击“下一题”后棋盘未变化，再试一次。")
            if not args.dry_run:
                safe_tap_next(client)
            continue

        if status == "no_board":
            # 一直认不出 —— 可能卡在中间态 / 弹窗遮挡
            print(f"[警告] {args.board_timeout}s 内无法识别棋盘，点一次“下一题”兜底。")
            if not args.dry_run:
                safe_tap_next(client)
            time.sleep(args.poll_interval)
            continue

        # status == "new" —— 到这里才真正开始新一轮
        round_num += 1
        print(f"\n===== 第 {round_num}/{args.max_rounds} 轮 =====")

        last_regions = result.get("regions")
        size = result.get("size")
        difficulty = result.get("difficulty")
        board = result.get("board") or {}
        print(f"棋盘 {size}x{size}  难度 {difficulty or '无评级'}  区域 {board}")

        # ――― 2. 落子 ―――
        points = queen_pixels(result)
        if not points:
            print("[警告] 求解器未给出解（无解或多解），跳过本轮落子。", file=sys.stderr)
        elif args.dry_run:
            print(f"[dry-run] 本应点击 {len(points)} 个格子：{points}")
        else:
            print(f"待点击 {len(points)} 个格子（整张截图坐标）：{points}")
            try:
                tap_points(client, points, args.tap_delay)
            except AScriptError as exc:
                print(f"[警告] 落子过程中设备报错：{exc}", file=sys.stderr)
                # 不退出；下一轮 poll_board 会通过 regions 比对发现棋盘没变

        # ――― 3. 等胜利动画播完（固定时长，必须硬等）―――
        # 全部落子后棋盘会播放完成动画；动画期间点“下一题”会失效或误触。
        if not args.dry_run and points:
            print(f"等待 {args.post_solve_wait}s 胜利动画……")
            time.sleep(args.post_solve_wait)

        # ――― 4. 点“下一题” ―――
        if args.dry_run:
            print("[dry-run] 跳过点击“下一题”。")
        else:
            safe_tap_next(client)

        # ――― 5. 轮询查找目标数字 ―――
        match = poll_number(
            client, number_template,
            timeout=args.number_timeout,
            poll_interval=args.poll_interval,
        )
        if match is not None:
            print(
                f"\n找到目标数字：({match.x}, {match.y})-"
                f"({match.x + match.width}, {match.y + match.height})  "
                f"尺寸 {match.width}x{match.height}  置信度 {match.confidence:.3f}"
            )
            print("停止脚本。")
            return 0

    print(f"\n已达到最大轮数 {args.max_rounds}，仍未找到目标数字。")
    return 1


# ---------------------------------------------------------------------------
# 入口
# ---------------------------------------------------------------------------

def _stop_on_sigterm(signum: int, frame: object) -> None:
    """让 SIGTERM 也走正常清理路径，避免 iproxy 子进程残留。"""
    raise KeyboardInterrupt


def parse_crop(spec: str) -> tuple[int, int, int, int]:
    parts = [p.strip() for p in spec.split(",")]
    if len(parts) != 4:
        raise argparse.ArgumentTypeError("--crop 需要四个整数：左,上,右,下")
    try:
        return tuple(int(p) for p in parts)  # type: ignore[return-value]
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"--crop 含非整数：{spec!r}") from exc


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Queens 自动解题循环（USB 隧道版）")
    parser.add_argument("--config", help="asclient.json 路径，默认取当前目录")
    parser.add_argument("--device", help="覆盖连接地址；USB 场景一般不需要")
    parser.add_argument("--password", help="覆盖 device.password")
    parser.add_argument("--udid", help="多设备时锁定目标手机")
    parser.add_argument("--local-port", type=int, help="本地控制端口，默认 9096")
    parser.add_argument("--local-log-port", type=int, help="本地日志端口，默认 10102")
    parser.add_argument("--iproxy", help="iproxy 绝对路径")
    parser.add_argument("--no-logs", action="store_true", help="不转发日志端口")
    parser.add_argument("--solver", default=DEFAULT_SOLVER, help="求解器可执行文件路径")
    parser.add_argument(
        "--crop",
        type=parse_crop,
        default=BOARD_CROP,
        help="棋盘提示框，格式 左,上,右,下，例如 0,710,1179,1880",
    )
    parser.add_argument("--tap-delay", type=float, default=TAP_DELAY,
                        help="落子点击之间的间隔秒数")
    parser.add_argument("--post-solve-wait", type=float, default=POST_SOLVE_WAIT,
                        help="全部落子后等待胜利动画的秒数（默认 2）")
    parser.add_argument("--max-rounds", type=int, default=100, help="最大循环轮数（默认 100）")
    parser.add_argument("--poll-interval", type=float, default=DEFAULT_POLL_INTERVAL,
                        help="两次截图之间的间隔秒数（默认 0.4）")
    parser.add_argument("--board-timeout", type=float, default=DEFAULT_BOARD_TIMEOUT,
                        help="等待新棋盘的总超时秒数（默认 15）")
    parser.add_argument("--number-timeout", type=float, default=DEFAULT_NUMBER_TIMEOUT,
                        help="等待目标数字的总超时秒数（默认 3）")
    parser.add_argument("--dry-run", action="store_true",
                        help="只识别不点击：跳过落子与“下一题”")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)

    number_template = Path(__file__).resolve().parent / NUMBER_TEMPLATE_NAME
    if not number_template.is_file():
        print(f"[错误] 目标数字模板不存在：{number_template}", file=sys.stderr)
        return 2

    try:
        solver = resolve_solver(args.solver)
    except FileNotFoundError as exc:
        print(f"[错误] {exc}", file=sys.stderr)
        return 4

    options = device_options(load_config(args.config))
    password = args.password if args.password is not None else options.get("password", "")

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
        with tunnel:
            routes = f"{tunnel.address} -> 设备:{tunnel.remote_port}"
            if tunnel.log_address:
                routes += f"；日志 {tunnel.log_address} -> 设备:{tunnel.remote_log_port}"
            print(f"USB 隧道已就绪：{routes}")

            device = connect(args.device or tunnel.address, password=password)
            client = device.client
            status = wait_until_ready(client)
            print(f"设备可用：{status.get('screen') or status.get('logical_screen')}")

            return run_loop(client, solver, args.crop, number_template, args)

    except (IProxyNotFoundError, TunnelError) as exc:
        print(f"USB 隧道失败：{exc}", file=sys.stderr)
        print("排查：iproxy 是否已安装、手机是否已解锁信任、本机 9096/10102 是否被占用。",
              file=sys.stderr)
        return 5
    except AScriptError as exc:
        print(f"设备操作失败：{exc}", file=sys.stderr)
        print("排查：确认 iPhone 与本机通过 USB 连接、已解锁并信任本机，且 AScript 服务已启动。",
              file=sys.stderr)
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