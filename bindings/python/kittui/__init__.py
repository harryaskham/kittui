"""Small ctypes binding for the kittui C ABI.

The binding intentionally mirrors the TypeScript wrapper: callers pass scene
objects (dicts) or JSON strings and receive terminal byte strings.  It uses only
Python's standard library so shell automation can vendor or copy it easily.
"""

from __future__ import annotations

import base64
import ctypes
import json
import os
import sys
from ctypes import POINTER, c_char_p, c_int, c_size_t, c_uint8, c_uint16, c_uint32, c_void_p
from pathlib import Path
from typing import Any, Iterable


class KittuiStatus:
    OK = 0
    NULL_POINTER = 1
    BAD_SCENE = 2
    RUNTIME = 3
    PANIC = 4


class KittuiError(RuntimeError):
    """Raised when a kittui FFI call returns a non-OK status."""


_DEFAULT_NAMES = {
    "darwin": ["libkittui_ffi.dylib"],
    "linux": ["libkittui_ffi.so"],
    "win32": ["kittui_ffi.dll"],
}


def _candidate_paths() -> list[Path]:
    env = os.environ.get("KITTUI_FFI_LIB")
    if env:
        return [Path(env)]
    names = _DEFAULT_NAMES.get(sys.platform, ["libkittui_ffi.so", "libkittui_ffi.dylib"])
    here = Path(__file__).resolve()
    repo = here.parents[3] if len(here.parents) > 3 else Path.cwd()
    out: list[Path] = []
    for profile in ["debug", "release"]:
        for name in names:
            out.append(repo / "target" / profile / name)
    out.extend(Path(name) for name in names)
    return out


def find_library() -> str:
    """Return the first plausible kittui FFI library path, or a bare name."""
    candidates = _candidate_paths()
    for path in candidates:
        if path.exists():
            return str(path)
    return str(candidates[-1])


def _json(value: Any) -> bytes:
    if isinstance(value, bytes):
        return value
    if isinstance(value, str):
        return value.encode("utf-8")
    return json.dumps(value, separators=(",", ":")).encode("utf-8")


def _scene_array(scenes: Iterable[Any]) -> bytes:
    normalized = [json.loads(s) if isinstance(s, str) else s for s in scenes]
    return _json(normalized)


class _SceneHelpers:
    """Primitive-only helpers for constructing JSON-compatible Scene objects."""

    @staticmethod
    def build(footprint_cells: tuple[int, int] | list[int], layers: list[dict[str, Any]], cell_size: dict[str, int] | None = None, animation: dict[str, Any] | None = None) -> dict[str, Any]:
        cols, rows = footprint_cells
        out = {
            "footprint": {"x": 0, "y": 0, "cols": int(cols), "rows": int(rows)},
            "cell_size": cell_size or {"width_px": 8, "height_px": 16},
            "layers": layers,
        }
        if animation is not None:
            out["animation"] = animation
        return out

    @staticmethod
    def rect_layer(cols: int, rows: int, rgba: list[int] | tuple[int, int, int, int], *, cell_size: dict[str, int] | None = None, label: str = "background", radius: float = 0.0) -> dict[str, Any]:
        cell = cell_size or {"width_px": 8, "height_px": 16}
        width = int(cols) * int(cell.get("width_px", 8))
        height = int(rows) * int(cell.get("height_px", 16))
        return {
            "label": label,
            "root": {
                "kind": "rect",
                "rect": {"origin": [0, 0], "width": width, "height": height},
                "fill": {"kind": "solid", "color": list(rgba)},
                "stroke": None,
                "corners": {"tl": radius, "tr": radius, "bl": radius, "br": radius},
            },
        }

    @staticmethod
    def solid_box(cols: int, rows: int, rgba: list[int] | tuple[int, int, int, int], *, cell_size: dict[str, int] | None = None, label: str = "background", radius: float = 0.0) -> dict[str, Any]:
        cell = cell_size or {"width_px": 8, "height_px": 16}
        return _SceneHelpers.build(
            (cols, rows),
            [_SceneHelpers.rect_layer(cols, rows, rgba, cell_size=cell, label=label, radius=radius)],
            cell,
        )

    @staticmethod
    def background_solid(rgba: list[int] | tuple[int, int, int, int]) -> dict[str, Any]:
        return _SceneHelpers.rect_layer(0, 0, rgba)


scene = _SceneHelpers()


class Kittui:
    """High-level wrapper around ``libkittui_ffi``."""

    def __init__(self, lib: Any, runtime: Any):
        self.lib = lib
        self.runtime = runtime

    @classmethod
    def open(cls, config: dict[str, Any] | None = None, library_path: str | None = None) -> "Kittui":
        lib = ctypes.CDLL(library_path or find_library())
        _wire_library(lib)
        if config is None:
            runtime = lib.kittui_runtime_new(None)
        else:
            runtime = lib.kittui_runtime_new_config(_json(config))
        if not runtime:
            raise KittuiError("failed to create kittui runtime")
        return cls(lib, runtime)

    @classmethod
    def from_library(cls, lib: Any, config: dict[str, Any] | None = None) -> "Kittui":
        """Construct from an already loaded/fake library (primarily for tests)."""
        if hasattr(lib, "_kittui_needs_wire"):
            _wire_library(lib)
        runtime = lib.kittui_runtime_new(None) if config is None else lib.kittui_runtime_new_config(_json(config))
        if not runtime:
            raise KittuiError("failed to create kittui runtime")
        return cls(lib, runtime)

    def configure(self, config: dict[str, Any]) -> "Kittui":
        """Reconfigure this live runtime using the FFI JSON config shape."""
        if not self.runtime:
            raise KittuiError("kittui runtime closed")
        self._check(
            self.lib.kittui_runtime_configure(self.runtime, _json(config)),
            "kittui_runtime_configure",
        )
        return self

    def close(self) -> None:
        if self.runtime:
            self.lib.kittui_runtime_free(self.runtime)
            self.runtime = None

    def __enter__(self) -> "Kittui":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    def abi_version(self) -> dict[str, int]:
        packed = int(self.lib.kittui_abi_version())
        return {"major": (packed >> 16) & 0xFFFF, "minor": packed & 0xFFFF}

    def probe(self) -> dict[str, Any]:
        return json.loads(self._owned_string(self.lib.kittui_probe_json(self.runtime)))

    def unplace(self, image_id: int | str) -> str:
        if isinstance(image_id, str):
            image_id = int(image_id, 16) if image_id.lower().startswith("0x") else int(image_id)
        return self._owned_string(self.lib.kittui_unplace(self.runtime, c_uint32(image_id)))

    def render(self, scene: Any) -> bytes:
        out_ptr = POINTER(c_uint8)()
        out_len = c_size_t()
        self._check(
            self.lib.kittui_render_json(self.runtime, _json(scene), ctypes.byref(out_ptr), ctypes.byref(out_len)),
            "kittui_render_json",
        )
        try:
            return bytes(ctypes.string_at(out_ptr, out_len.value))
        finally:
            if out_ptr:
                self.lib.kittui_bytes_free(out_ptr, out_len)

    def render_many(self, scenes: Iterable[Any]) -> dict[str, Any]:
        out = c_char_p()
        self._check(
            self.lib.kittui_render_many_json(self.runtime, _scene_array(scenes), ctypes.byref(out)),
            "kittui_render_many_json",
        )
        return json.loads(self._consume_out(out))

    def render_many_to_dir(self, scenes: Iterable[Any], out_dir: str | os.PathLike[str], prefix: str = "scene") -> dict[str, Any]:
        """Render many scenes and write deterministic PNG files plus manifest.json."""
        manifest = self.render_many(scenes)
        out_path = Path(out_dir)
        out_path.mkdir(parents=True, exist_ok=True)
        images = []
        for image in manifest.get("images", []):
            index = int(image.get("index", len(images)))
            filename = f"{prefix}-{index:05d}.png"
            png_base64 = image.get("png_base64") or ""
            data = base64.b64decode(png_base64)
            (out_path / filename).write_bytes(data)
            entry = dict(image)
            entry["file"] = filename
            images.append(entry)
        written = dict(manifest)
        written["images"] = images
        written["out_dir"] = str(out_path)
        manifest_path = out_path / "manifest.json"
        manifest_path.write_text(json.dumps(written, indent=2) + "\n", encoding="utf-8")
        written["manifest"] = str(manifest_path)
        return written

    def place(self, scene: Any) -> str:
        out = c_char_p()
        self._check(self.lib.kittui_place_json(self.runtime, _json(scene), ctypes.byref(out)), "kittui_place_json")
        return self._consume_out(out)

    def place_at(self, scene: Any, x: int, y: int) -> str:
        out = c_char_p()
        self._check(
            self.lib.kittui_place_json_at(self.runtime, _json(scene), c_uint16(x), c_uint16(y), ctypes.byref(out)),
            "kittui_place_json_at",
        )
        return self._consume_out(out)

    def place_many(self, scenes: Iterable[Any]) -> str:
        out = c_char_p()
        self._check(self.lib.kittui_place_many_json(self.runtime, _scene_array(scenes), ctypes.byref(out)), "kittui_place_many_json")
        return self._consume_out(out)

    def place_many_at(self, scenes: Iterable[Any], x: int, y: int) -> str:
        out = c_char_p()
        self._check(
            self.lib.kittui_place_many_json_at(self.runtime, _scene_array(scenes), c_uint16(x), c_uint16(y), ctypes.byref(out)),
            "kittui_place_many_json_at",
        )
        return self._consume_out(out)

    def place_many_channels(self, scenes: Iterable[Any], x: int = 0, y: int = 0) -> dict[str, Any]:
        out = c_char_p()
        self._check(
            self.lib.kittui_place_many_json_channels(self.runtime, _scene_array(scenes), c_uint16(x), c_uint16(y), ctypes.byref(out)),
            "kittui_place_many_json_channels",
        )
        return json.loads(self._consume_out(out))

    def _check(self, status: int, name: str) -> None:
        if int(status) == KittuiStatus.OK:
            return
        err = ""
        if hasattr(self.lib, "kittui_last_error") and self.runtime:
            try:
                err = self._owned_string(self.lib.kittui_last_error(self.runtime))
            except Exception:
                err = ""
        suffix = f": {err}" if err else ""
        raise KittuiError(f"{name} failed status={int(status)}{suffix}")

    def _consume_out(self, out: c_char_p) -> str:
        return self._owned_string(out.value)

    def _owned_string(self, ptr: Any) -> str:
        if not ptr:
            return ""
        raw = ctypes.cast(ptr, c_char_p).value or b""
        text = raw.decode("utf-8", errors="replace")
        self.lib.kittui_string_free(ptr)
        return text


def _wire_library(lib: Any) -> None:
    """Attach ctypes signatures to a real CDLL. Fake test libs opt out."""
    if not isinstance(lib, ctypes.CDLL):
        return
    lib.kittui_runtime_new.argtypes = [c_char_p]
    lib.kittui_runtime_new.restype = c_void_p
    lib.kittui_runtime_new_config.argtypes = [c_char_p]
    lib.kittui_runtime_new_config.restype = c_void_p
    lib.kittui_runtime_free.argtypes = [c_void_p]
    lib.kittui_runtime_free.restype = None
    lib.kittui_runtime_configure.argtypes = [c_void_p, c_char_p]
    lib.kittui_runtime_configure.restype = c_int
    lib.kittui_string_free.argtypes = [c_char_p]
    lib.kittui_string_free.restype = None
    lib.kittui_bytes_free.argtypes = [POINTER(c_uint8), c_size_t]
    lib.kittui_bytes_free.restype = None
    lib.kittui_abi_version.argtypes = []
    lib.kittui_abi_version.restype = c_uint32
    lib.kittui_probe_json.argtypes = [c_void_p]
    lib.kittui_probe_json.restype = c_char_p
    lib.kittui_unplace.argtypes = [c_void_p, c_uint32]
    lib.kittui_unplace.restype = c_char_p
    lib.kittui_last_error.argtypes = [c_void_p]
    lib.kittui_last_error.restype = c_char_p
    lib.kittui_render_json.argtypes = [c_void_p, c_char_p, ctypes.POINTER(POINTER(c_uint8)), ctypes.POINTER(c_size_t)]
    lib.kittui_render_json.restype = c_int
    for name in [
        "kittui_render_many_json",
        "kittui_place_json",
        "kittui_place_many_json",
    ]:
        fn = getattr(lib, name)
        fn.argtypes = [c_void_p, c_char_p, ctypes.POINTER(c_char_p)]
        fn.restype = c_int
    lib.kittui_place_json_at.argtypes = [c_void_p, c_char_p, c_uint16, c_uint16, ctypes.POINTER(c_char_p)]
    lib.kittui_place_json_at.restype = c_int
    for name in ["kittui_place_many_json_at", "kittui_place_many_json_channels"]:
        fn = getattr(lib, name)
        fn.argtypes = [c_void_p, c_char_p, c_uint16, c_uint16, ctypes.POINTER(c_char_p)]
        fn.restype = c_int


__all__ = ["Kittui", "KittuiError", "KittuiStatus", "find_library", "scene"]
