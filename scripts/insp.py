from asclient import AScriptTunnel, connect
from asclient.config import device_options, load_config
from asclient.errors import AScriptError
from asclient.inspector import run_forever


def main() -> None:
    options = device_options(load_config())

    try:
        with AScriptTunnel.from_config() as tunnel:
            device = connect(
                tunnel.address,
                password=options.get("password", ""),
            )
            run_forever(device.client, host="127.0.0.1", port=0, open_browser=True)
    except AScriptError as exc:
        raise SystemExit(f"打开inspect失败：{exc}") from exc


if __name__ == "__main__":
    main()


