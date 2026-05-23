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

// Reconfigure long-lived runtimes without closing the handle.
k.configure({ transport: 'tmux', supportsKitty: true });

const sceneObject = scene.solidBox({
  cols: 60,
  rows: 8,
  rgba: [0, 216, 255, 255],
  radius: 6,
});
const gradientScene = scene.gradientBox({
  cols: 60,
  rows: 8,
  start: [0, 0, 0, 255],
  end: [0, 216, 255, 255],
});
const imageScene = scene.imageBox({
  cols: 16,
  rows: 8,
  src: '/tmp/preview.png',
  fit: 'contain',
});
const glowScene = scene.glowBox({ cols: 8, rows: 3, rgba: [0, 216, 255, 128], intensity: 0.5 });
const scanlineScene = scene.scanlinesBox({ cols: 8, rows: 3, alpha: 24, periodPx: 3 });

// Render-only PNG bytes for previews/artifacts.
const png = k.render(sceneObject);
const renderManifest = k.renderMany([sceneObject, sceneObject]);
console.log(renderManifest.images[0].png_base64);
const written = k.renderManyToDir([sceneObject, sceneObject], 'previews', { prefix: 'scene' });
console.log(written.manifest, written.images[0].file);

const bytes = k.place(sceneObject);

process.stdout.write(bytes);

// Place multiple scenes in one FFI round-trip.
process.stdout.write(k.placeMany([sceneJsonOrObject, sceneJsonOrObject]));

// Inspect runtime metadata and clean up uploaded images.
console.log(k.probe());
process.stdout.write(k.unplace('0x1234'));

// Reuse the same scene/render identity but place it elsewhere.
process.stdout.write(k.placeAt(sceneJsonOrObject, 10, 4));

// Batch placement at a group origin, with channelized output for hosts that
// want to schedule upload / placement / embed writes separately.
const channels = k.placeManyChannels([sceneJsonOrObject], 10, 4);
console.log(channels.image_ids, channels.footprints, channels.upload_bytes);
process.stdout.write(channels.upload + channels.placement + channels.embed);
```

The returned string is the concatenated `upload + placement + embed`
escape sequences ready to write at the cursor's current position. Use
`placeAt(scene, x, y)` when the host wants to control terminal placement
without mutating the scene JSON. Failed FFI calls throw errors containing both
numeric status and `kittui_last_error` detail when the runtime provides it.

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
