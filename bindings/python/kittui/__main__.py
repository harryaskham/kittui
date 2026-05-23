"""Module entry point for the kittui Python binding."""

from __future__ import annotations

import argparse
import json
import sys

from . import Kittui, KittuiError, find_library


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Inspect the kittui Python/FFI binding")
    parser.add_argument("--library", help="explicit libkittui_ffi path")
    parser.add_argument("--config-json", help="runtime config JSON for probe/abi checks")
    parser.add_argument("--find-library", action="store_true", help="print discovered library path and exit")
    parser.add_argument("--abi", action="store_true", help="print ABI version JSON")
    parser.add_argument("--probe", action="store_true", help="print runtime probe JSON")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.find_library:
        print(find_library())
        return 0

    config = json.loads(args.config_json) if args.config_json else None
    try:
        with Kittui.open(config=config, library_path=args.library) as k:
            if args.probe:
                print(json.dumps(k.probe(), sort_keys=True))
            else:
                # Default to ABI JSON so `python -m kittui` is a cheap smoke check.
                print(json.dumps(k.abi_version(), sort_keys=True))
    except (OSError, KittuiError, json.JSONDecodeError) as exc:
        print(f"kittui: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
