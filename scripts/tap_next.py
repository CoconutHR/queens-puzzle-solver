"""点击“下一题”按钮。

macOS 走 USB 隧道（AScriptTunnel），其它平台走 Wi-Fi 直连。
点击方式统一用图像匹配（tap_image），不依赖分辨率。
"""

from __future__ import annotations

import sys

from asclient import AScriptTunnel, connect
from asclient.config import device_options, load_config
from asclient.errors import AScriptError

DEFAULT_ADDRESS = "192.168.3.17:9096"

TAP_IMAGE = "scripts/橙色.png"
TAP_CONFIDENCE = 0.98
TAP_TIMEOUT = 5
TAP_REGION = (0, 0.6, 0.5, 0.9)


def _open_device(options: dict):
    """按平台返回已连接的 device。macOS → USB 隧道，其它 → Wi-Fi 直连。"""
    if sys.platform == "darwin":
        # macOS：USB 隧道
        tunnel = AScriptTunnel.from_config()
        tunnel.__enter__()                       # 手动进入上下文，稍后手动退出
        try:
            device = connect(tunnel.address, password=options.get("password", ""))
        except Exception:
            tunnel.__exit__(None, None, None)    # 连不上时清理隧道
            raise
        return device, tunnel                   # 调用方负责 tunnel.__exit__()

    # Windows / Linux：Wi-Fi 直连
    address = options.get("ipport", DEFAULT_ADDRESS)
    password = options.get("password", "")
    device = connect(address, password=password)
    return device, None


def main() -> None:
    options = device_options(load_config())
    tunnel = None

    try:
        device, tunnel = _open_device(options)
        device.client.status()                   # 等设备就绪

        with device.client.locked():
            device.client.tap_image(
                TAP_IMAGE,
                confidence=TAP_CONFIDENCE,
                timeout=TAP_TIMEOUT,
                region_relative=TAP_REGION,
            )
        print("已点击下一题")

    except AScriptError as exc:
        raise SystemExit(f"点击失败：{exc}") from exc

    finally:
        if tunnel is not None:
            tunnel.__exit__(None, None, None)


if __name__ == "__main__":
    main()
