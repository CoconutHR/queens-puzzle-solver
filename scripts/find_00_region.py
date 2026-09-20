#!/usr/bin/env python3
"""在限定区域内用模板匹配定位 00.png（区域找图验证）。

region 来自上一个脚本 find_00.py 的实测结果：
    左上角 (328, 256)，右下角 (408, 319)   # 物理像素，左上含、右下不含
即 00.png 上次被找到的位置本身，用于验证 find_image 的 region 参数：
把搜索范围限定到该区域后，是否仍能成功识别图片位置。

同时跑一次全屏找图作对照，用于判断：
- 区域内命中 + 全屏位置一致 → 区域找图工作正常；
- 区域内未命中 + 全屏命中   → 图片还在屏上但已移出/偏出该区域；
- 两者都未命中             → 当前屏幕上没有该图（如页面已切换）。

只读操作：仅截图与匹配，不点击、不输入、不上传、不删除。

用法（需在装有 Pillow 的 Python 下运行，如 python3.11）：
    python3 find_00_region.py                                    # 默认区域 + 阈值 0.98
    python3 find_00_region.py --region 328 256 408 319            # 自定义区域
    python3 find_00_region.py --confidence 0.95                   # 自定义阈值
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

# find_00.py 实测的 00.png 位置：左上 (328, 256)，右下 (408, 319)，尺寸 80x63
DEFAULT_REGION = (328, 256, 408, 319)
DEFAULT_CONFIDENCE = 0.98


def find_config() -> Path | None:
    """按优先级探测 asclient.json：当前目录 → 脚本目录 → 脚本目录上级 → ~/Developer/automation。"""
    here = Path(__file__).resolve().parent
    for path in (Path.cwd() / "asclient.json", here / "asclient.json", here.parent / "asclient.json",
                 Path.home() / "Developer" / "automation" / "asclient.json"):
        if path.is_file():
            return path
    return None


def describe(match, label: str) -> None:
    """按统一格式输出一次匹配结果。"""
    if match is None:
        print(f"[{label}] 未找到匹配")
        return
    left, top = match.x, match.y
    right, bottom = match.x + match.width, match.y + match.height
    print(f"[{label}] 已找到  尺寸 {match.width}x{match.height}  置信度 {match.confidence:.3f}")
    print(f"    左上角坐标：({left}, {top})")
    print(f"    右下角坐标：({right}, {bottom})   # 矩形外沿（左上含、右下不含），最后有效像素为 ({right - 1}, {bottom - 1})")
    print(f"    中心点坐标：({match.center[0]:.0f}, {match.center[1]:.0f})")


def main() -> int:
    parser = argparse.ArgumentParser(description="在限定区域内定位模板图片（物理像素 region）")
    parser.add_argument("--template", default="00.png", help="模板图片（默认：脚本同目录的 00.png）")
    parser.add_argument("--region", nargs=4, type=int, metavar=("LEFT", "TOP", "RIGHT", "BOTTOM"),
                        default=list(DEFAULT_REGION),
                        help=f"限定搜索区域，物理像素（默认：{' '.join(map(str, DEFAULT_REGION))}）")
    parser.add_argument("--confidence", type=float, default=DEFAULT_CONFIDENCE,
                        help=f"匹配阈值 (0, 1]（默认：{DEFAULT_CONFIDENCE}）")
    args = parser.parse_args()

    template = Path(args.template)
    if not template.is_absolute():
        template = Path(__file__).resolve().parent / template
    if not template.is_file():
        print(f"[错误] 模板不存在：{template}")
        return 2

    region = tuple(args.region)

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

            print(f"\n开始区域找图：{template.name}  region={region}  confidence={args.confidence}")
            region_match = client.find_image(str(template), confidence=args.confidence, region=region)

            print("\n全屏对照找图（同一阈值，不限区域）：")
            full_match = client.find_image(str(template), confidence=args.confidence)
    except AScriptError as exc:
        print(f"[错误] asclient 操作失败：{exc}")
        return 3

    print("\n――― 结果 ―――")
    describe(region_match, "区域找图")
    describe(full_match, "全屏对照")

    print("\n――― 结论 ―――")
    if region_match is not None:
        if full_match is not None and (full_match.x, full_match.y) == (region_match.x, region_match.y):
            print(f"成功：在限定区域 {region} 内识别到 {template.name}，且与全屏找图位置一致，region 参数工作正常。")
        else:
            print(f"成功：在限定区域 {region} 内识别到 {template.name}（全屏对照位置不同，图片可能有多处相似内容）。")
        return 0
    if full_match is not None:
        print(f"区域内未命中，但全屏在 ({full_match.x}, {full_match.y}) 找到：图片仍在屏幕上，"
              f"但已不在原区域内（页面可能发生变化）。请用新坐标更新 region 后重试。")
        return 1
    print(f"当前屏幕上未找到 {template.name}（区域与全屏均未命中）：可能页面已切换，或阈值 {args.confidence} 过高，"
          f"可回到目标页面或适当降低 --confidence 重试。")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
