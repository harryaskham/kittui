# kittui Python binding

Small standard-library `ctypes` wrapper for the `kittui-ffi` C ABI. It is meant
for shell automation and platform glue that wants to use kittui as a terminal
renderer without spawning the CLI for every frame.

```python
from kittui import Kittui, scene

scene_obj = scene.solid_box(8, 3, [0, 216, 255, 255], radius=6)
gradient_obj = scene.gradient_box(8, 3, [0, 0, 0, 255], [0, 216, 255, 255])
image_obj = scene.image_box(16, 8, "/tmp/preview.png", fit="contain")
glow_obj = scene.glow_box(8, 3, [0, 216, 255, 128], intensity=0.5)
scanlines_obj = scene.scanlines_box(8, 3, alpha=24, period_px=3)
clipped_layer = scene.clip_layer(
    {"origin": [0, 0], "width": 64, "height": 32},
    scene.group([scene.rect_layer(8, 3, [0, 216, 255, 255])["root"]], opacity=0.8),
)

with Kittui.open({"renderer": "cpu", "transport": "direct"}) as k:
    k.configure({"renderer": "cpu", "transport": "tmux"})
    print(k.abi_version())
    png_bytes = k.render(scene_obj)
    png_manifest = k.render_many([scene_obj, scene_obj])
    written = k.render_many_to_dir([scene_obj, scene_obj], "previews", prefix="scene")
    bytes_to_write = k.place_at(scene_obj, 10, 4)
    batch = k.place_many_channels([scene_obj], 10, 4)
    assert "upload" in batch and "placement" in batch and "embed" in batch
```

## Library discovery

Set `KITTUI_FFI_LIB=/path/to/libkittui_ffi.{dylib,so,dll}` to force a shared
library path. Otherwise the binding looks in `target/debug` and
`target/release` relative to the repository, then falls back to the platform
library name.

## Packaging / entry point

The directory has a minimal `pyproject.toml`, so it can be installed in editable
mode while developing:

```sh
python3 -m pip install -e bindings/python
python3 -m kittui --find-library
python3 -m kittui --abi
python3 -m kittui --probe --config-json '{"renderer":"cpu"}'
```

`python -m kittui` defaults to printing ABI version JSON. It needs a built
`libkittui_ffi`; `--find-library` only prints the path it would try.

## API

- `Kittui.open(config=None, library_path=None)`
- `Kittui.from_library(lib, config=None)` for tests/embedding
- `abi_version()`
- `probe()`
- `configure(config)`
- `unplace(image_id)`
- `render(scene)`
- `render_many(scenes)`
- `render_many_to_dir(scenes, out_dir, prefix="scene")`
- `place(scene)`
- `place_at(scene, x, y)`
- `place_many(scenes)`
- `place_many_at(scenes, x, y)`
- `place_many_channels(scenes, x=0, y=0)`
- `scene.build(...)`, `scene.rect_layer(...)`, `scene.solid_box(...)`, `scene.gradient_layer(...)`, `scene.gradient_box(...)`, `scene.glow_layer(...)`, `scene.glow_box(...)`, `scene.scanlines_layer(...)`, `scene.scanlines_box(...)`, `scene.image_layer(...)`, `scene.image_box(...)`
- composition primitives: `scene.layer(...)`, `scene.group(...)`, `scene.group_layer(...)`, `scene.composite(...)`, `scene.composite_layer(...)`, `scene.clip(...)`, `scene.clip_layer(...)`, `scene.mask(...)`, `scene.mask_layer(...)`

Scenes can be Python dictionaries or JSON strings. Batch methods accept a mix of
both. The `scene` helper builds primitive-only JSON-compatible Scene objects for
common platform/shell previews.

## Tests

```sh
PYTHONPATH=bindings/python python3 -m unittest discover bindings/python
```

The tests use a fake CDLL object, so they do not require a compiled kittui shared
library.
