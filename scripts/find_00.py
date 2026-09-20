#!/usr/bin/env python3
"""通过 USB 隧道连接 iPhone，用模板匹配定位 00.png 在屏幕上的位置。

- 连接方式：AScriptTunnel.from_config()，读取 asclient.json 的 tunnel 段，
  本机 127.0.0.1:9096 / 10102 经 iproxy 转发到手机上的 AScript 服务。
- 找图方式：client.find_image()，本机模板匹配，坐标为设备物理像素
  （与 tap / swipe / 截图 / OCR 同一坐标系）。
- 只读操作：仅截图与匹配，不点击、不输入、不上传、不删除。

用法（需在装有 Pillow 的 Python 下运行）：
    python3 find_00.py                      # 默认 confidence=0.9
    python3 find_00.py --confidence 0.8     # 降低阈值
    python3 find_00.py --template 00.png    # 指定其他模板
"""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

# ―― 无需 pip install：失败时把 asclient 源码目录加入 sys.path（可用环境变量 ASCCLIENT_SRC 覆盖） ――
try:
    import asclient  # noqa: F401
except ModuleNotFoundError:
    _SRC = os.environ.get("ASCLIENT_SRC", str(Path.home() / "Developer" / "automation"))
    if not Path(_SRC, "asclient").is_dir():
        sys.exit(f"[错误] 找不到 asclient 包：请安装库，或设置环境变量 ASCCLIENT_SRC 指向其源码目录（当前尝试：{_SRC}）")
    sys.path.insert(0, _SRC)

from asclient import AScriptClient, AScriptError, AScriptTunnel  # noqa: E402


def find_config() -> Path | None:
    """按优先级探测 asclient.json：当前目录 → 脚本目录 → 脚本目录上级 → ~/Developer/automation。"""
    here = Path(__file__).resolve().parent
    for path in (Path.cwd() / "asclient.json", here / "asclient.json", here.parent / "asclient.json",
                 Path.home() / "Developer" / "automation" / "asclient.json"):
        if path.is_file():
            return path
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description="USB 隧道连接设备并定位模板图片位置")
    parser.add_argument("--template", default="00.png", help="模板图片（默认：脚本同目录的 00.png）")
    parser.add_argument("--confidence", type=float, default=0.9, help="匹配阈值 (0, 1]，默认 0.9")
    args = parser.parse_args()

    template = Path(args.template)
    if not template.is_absolute():
        template = Path(__file__).resolve().parent / template
    if not template.is_file():
        print(f"[错误] 模板不存在：{template}")
        return 2

    config = find_config()
    if config is not None:
        print(f"使用配置：{config}")
        tunnel = AScriptTunnel.from_config(path=str(config))
    else:
        print("未找到 asclient.json，使用内置默认隧道配置")
        tunnel = AScriptTunnel.from_config()

    try:
        with tunnel as t:
            print(f"USB 隧道已建立：service={t.address}  logs={t.log_address}")
            client = AScriptClient(t.address)
            print(f"设备连通：{client.ping()}")
            print(f"前台应用：{client.current_app()}")

            print(f"开始找图：{template.name}（confidence={args.confidence}）")
            match = client.find_image(str(template), confidence=args.confidence)
    except AScriptError as exc:
        print(f"[错误] asclient 操作失败：{exc}")
        return 3

    if match is None:
        print(f"[未找到] 屏幕上没有匹配到 {template.name}；可尝试降低 --confidence（如 0.8）后重试")
        return 1

    # ImageMatch(x, y, width, height, confidence)，均为设备物理像素
    left, top = match.x, match.y
    right, bottom = match.x + match.width, match.y + match.height
    print(f"[已找到] {template.name}  尺寸 {match.width}x{match.height}  置信度 {match.confidence:.3f}")
    print(f"左上角坐标：({left}, {top})")
    print(f"右下角坐标：({right}, {bottom})   # 矩形外沿（左上含、右下不含），最后有效像素为 ({right - 1}, {bottom - 1})")
    print(f"中心点坐标：({match.center[0]:.0f}, {match.center[1]:.0f})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
