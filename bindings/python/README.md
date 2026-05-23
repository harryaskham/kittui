# kittui Python binding

Small standard-library `ctypes` wrapper for the `kittui-ffi` C ABI. It is meant
for shell automation and platform glue that wants to use kittui as a terminal
renderer without spawning the CLI for every frame.

```python
from kittui import Kittui

scene = {
    "footprint": {"x": 0, "y": 0, "cols": 8, "rows": 3},
    "cell_size": {"width_px": 8, "height_px": 16},
    "layers": [],
}

with Kittui.open({"renderer": "cpu", "transport": "direct"}) as k:
    print(k.abi_version())
    bytes_to_write = k.place_at(scene, 10, 4)
    batch = k.place_many_channels([scene], 10, 4)
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
- `unplace(image_id)`
- `place(scene)`
- `place_at(scene, x, y)`
- `place_many(scenes)`
- `place_many_at(scenes, x, y)`
- `place_many_channels(scenes, x=0, y=0)`

Scenes can be Python dictionaries or JSON strings. Batch methods accept a mix of
both.

## Tests

```sh
PYTHONPATH=bindings/python python3 -m unittest discover bindings/python
```

The tests use a fake CDLL object, so they do not require a compiled kittui shared
library.
