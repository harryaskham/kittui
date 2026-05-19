// Integration smoke test for the koffi binding.
//
// Locates a built libkittui_ffi via the same probe order the binding
// uses; skips with a clear message if the library isn't present.

import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import { strict as assert } from 'node:assert';

import { Kittui, scene } from '../src/index.js';

const __filename = fileURLToPath(import.meta.url);
const REPO_ROOT = resolve(__filename, '..', '..', '..', '..');

function libBuilt() {
  for (const name of ['libkittui_ffi.dylib', 'libkittui_ffi.so', 'kittui_ffi.dll']) {
    if (existsSync(`${REPO_ROOT}/target/release/${name}`)) return true;
    if (existsSync(`${REPO_ROOT}/target/debug/${name}`)) return true;
  }
  return false;
}

test('koffi binding round-trips a still scene through the FFI', { skip: !libBuilt() }, async () => {
  const k = await Kittui.open({ cacheDir: `${REPO_ROOT}/target/kittui-cache-ts-test` });
  const abi = k.abiVersion();
  assert.equal(typeof abi.major, 'number');
  assert.equal(typeof abi.minor, 'number');

  const s = {
    footprint: { x: 0, y: 0, cols: 4, rows: 2 },
    cell_size: { width_px: 8, height_px: 16 },
    layers: [
      {
        label: 'background',
        root: {
          kind: 'rect',
          rect: { origin: [0, 0], width: 32, height: 32 },
          fill: { kind: 'solid', color: [0, 216, 255, 255] },
          stroke: null,
          corners: { tl: 0, tr: 0, bl: 0, br: 0 },
        },
      },
    ],
  };
  const bytes = k.place(s);
  assert.ok(bytes.length > 0, 'place() should return non-empty bytes');
  assert.ok(bytes.includes('\x1b_G'), 'output should contain kitty graphics escape');

  // Cache hit on the second call: upload omitted, placement+embed still
  // produced.
  const second = k.place(s);
  assert.ok(second.length > 0);

  k.close();
});

test('scene helpers produce JSON-compatible plain objects', () => {
  const s = scene.build({
    footprintCells: [4, 2],
    layers: [scene.backgroundSolid([0, 216, 255, 255])],
  });
  assert.equal(s.footprint.cols, 4);
  assert.equal(s.layers.length, 1);
});
