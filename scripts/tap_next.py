"""通过 USB 隧道点击“下一题”按钮。"""

from asclient import AScriptTunnel, connect
from asclient.config import device_options, load_config
from asclient.errors import AScriptError

NEXT_BUTTON = (600, 2000)


def main() -> None:
    options = device_options(load_config())

    try:
        with AScriptTunnel.from_config() as tunnel:
            device = connect(
                tunnel.address,
                password=options.get("password", ""),
            )
            device.tap(*NEXT_BUTTON)
            print(f"已点击下一题：{NEXT_BUTTON}")
    except AScriptError as exc:
        raise SystemExit(f"点击失败：{exc}") from exc


if __name__ == "__main__":
    main()
