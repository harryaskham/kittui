# @kittui/koffi

Pure-JavaScript binding to `libkittui_ffi`. Zero build step — `koffi`
dlopens the kittui cdylib at runtime and exposes a tiny typed surface.

## Install

```sh
npm install @kittui/koffi
# build the library at the repo root:
cargo build --release -p kittui-ffi
```

## Use

```js
import { Kittui } from '@kittui/koffi';

const k = await Kittui.open({
  transport: 'direct',
  supportsKitty: true,
  supportsUnicodePlaceholders: true,
});

const bytes = k.place({
  footprint: { x: 0, y: 0, cols: 60, rows: 8 },
  cell_size: { width_px: 8, height_px: 16 },
  layers: [
    {
      label: 'background',
      root: {
        kind: 'rect',
        rect: { origin: [0, 0], width: 480, height: 128 },
        fill: { kind: 'solid', color: [0, 216, 255, 255] },
        stroke: null,
        corners: { tl: 6, tr: 6, bl: 6, br: 6 },
      },
    },
  ],
});

process.stdout.write(bytes);

// Place multiple scenes in one FFI round-trip.
process.stdout.write(k.placeMany([sceneJsonOrObject, sceneJsonOrObject]));

// Inspect runtime metadata and clean up uploaded images.
console.log(k.probe());
process.stdout.write(k.unplace('0x1234'));

// Reuse the same scene/render identity but place it elsewhere.
process.stdout.write(k.placeAt(sceneJsonOrObject, 10, 4));
```

The returned string is the concatenated `upload + placement + embed`
escape sequences ready to write at the cursor's current position. Use
`placeAt(scene, x, y)` when the host wants to control terminal placement
without mutating the scene JSON.

## Library discovery

`Kittui.open()` probes the following paths in order:

1. `$KITTUI_LIB_PATH`
2. `<repo>/target/release/libkittui_ffi.{so,dylib,dll}`
3. `<repo>/target/debug/libkittui_ffi.{so,dylib,dll}`
4. The platform's library search path (`libkittui_ffi`)

For a fully explicit load, use `Kittui.openWithLibrary(path)`.

## Roadmap

The `koffi` path stays as the always-works fallback. A `napi-rs`-based
`@kittui/napi` package with `prebuildify`-built binaries per `(platform,
arch)` is the higher-performance path; it lives under a separate
package once we have CI to build the prebuilts.
