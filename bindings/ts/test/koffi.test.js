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

function fakeLib(calls) {
  return {
    func(signature) {
      if (signature.includes('kittui_runtime_new_config')) {
        return (json) => {
          calls.push(['new_config', JSON.parse(json)]);
          return { runtime: 'configured' };
        };
      }
      if (signature.includes('kittui_runtime_new(')) {
        return (cacheDir) => {
          calls.push(['new', cacheDir]);
          return { runtime: 'plain' };
        };
      }
      if (signature.includes('kittui_runtime_free')) {
        return () => calls.push(['free']);
      }
      if (signature.includes('kittui_string_free')) {
        return () => undefined;
      }
      if (signature.includes('kittui_probe_json')) {
        return () => '{"abi_major":0,"abi_minor":4,"transport":"Direct"}';
      }
      if (signature.includes('kittui_unplace')) {
        return (_runtime, imageId) => {
          calls.push(['unplace', imageId]);
          return 'deleted';
        };
      }
      if (signature.includes('kittui_place_many_json_at')) {
        return (_runtime, scenesJson, x, y, out) => {
          calls.push(['place_many_at', JSON.parse(scenesJson), x, y]);
          out[0] = 'placed-many-at';
          return 0;
        };
      }
      if (signature.includes('kittui_place_many_json')) {
        return (_runtime, scenesJson, out) => {
          calls.push(['place_many', JSON.parse(scenesJson)]);
          out[0] = 'placed-many';
          return 0;
        };
      }
      if (signature.includes('kittui_place_json_at')) {
        return (_runtime, sceneJson, x, y, out) => {
          calls.push(['place_at', JSON.parse(sceneJson), x, y]);
          out[0] = 'placed-at';
          return 0;
        };
      }
      if (signature.includes('kittui_place_json')) {
        return (_runtime, sceneJson, out) => {
          calls.push(['place', JSON.parse(sceneJson)]);
          out[0] = 'placed';
          return 0;
        };
      }
      if (signature.includes('kittui_abi_version')) {
        return () => (0 << 16) | 4;
      }
      throw new Error(`unexpected signature ${signature}`);
    },
  };
}

test('constructor uses JSON runtime config when terminal options are present', () => {
  const calls = [];
  const k = new Kittui(fakeLib(calls), {
    cacheDir: '/tmp/kittui-cache',
    renderer: 'cpu',
    transport: 'direct',
    columns: 100,
    rows: 40,
    cellWidthPx: 9,
    cellHeightPx: 18,
    supportsKitty: true,
    supportsUnicodePlaceholders: true,
  });
  assert.equal(calls[0][0], 'new_config');
  assert.deepEqual(calls[0][1], {
    cache_dir: '/tmp/kittui-cache',
    renderer: 'cpu',
    transport: 'direct',
    columns: 100,
    rows: 40,
    cell_width_px: 9,
    cell_height_px: 18,
    supports_kitty: true,
    supports_unicode_placeholders: true,
  });
  k.close();
});

test('probe parses runtime metadata and unplace forwards image ids', () => {
  const calls = [];
  const k = new Kittui(fakeLib(calls), {});
  assert.deepEqual(k.probe(), { abi_major: 0, abi_minor: 4, transport: 'Direct' });
  assert.equal(k.unplace('0x1234'), 'deleted');
  assert.equal(k.unplace(7), 'deleted');
  assert.deepEqual(calls.filter((call) => call[0] === 'unplace'), [
    ['unplace', 0x1234],
    ['unplace', 7],
  ]);
  k.close();
});

test('placeAt forwards explicit x/y to FFI', () => {
  const calls = [];
  const k = new Kittui(fakeLib(calls), {});
  const s = scene.build({
    footprintCells: [4, 2],
    layers: [scene.backgroundSolid([0, 216, 255, 255])],
  });
  assert.equal(k.placeAt(s, 7, 9), 'placed-at');
  const placeAtCall = calls.find((call) => call[0] === 'place_at');
  assert.equal(placeAtCall[2], 7);
  assert.equal(placeAtCall[3], 9);
  assert.equal(placeAtCall[1].footprint.cols, 4);
  k.close();
});

test('placeMany forwards one JSON scene array to FFI', () => {
  const calls = [];
  const k = new Kittui(fakeLib(calls), {});
  const s = scene.build({
    footprintCells: [4, 2],
    layers: [scene.backgroundSolid([0, 216, 255, 255])],
  });
  assert.equal(k.placeMany([s, JSON.stringify(s)]), 'placed-many');
  const call = calls.find((c) => c[0] === 'place_many');
  assert.equal(call[1].length, 2);
  assert.equal(call[1][0].footprint.cols, 4);
  assert.equal(call[1][1].footprint.rows, 2);
  k.close();
});

test('placeManyAt forwards scene array and origin to FFI', () => {
  const calls = [];
  const k = new Kittui(fakeLib(calls), {});
  const s = scene.build({
    footprintCells: [4, 2],
    layers: [scene.backgroundSolid([0, 216, 255, 255])],
  });
  assert.equal(k.placeManyAt([s, JSON.stringify(s)], 10, 20), 'placed-many-at');
  const call = calls.find((c) => c[0] === 'place_many_at');
  assert.equal(call[1].length, 2);
  assert.equal(call[2], 10);
  assert.equal(call[3], 20);
  assert.equal(call[1][0].footprint.cols, 4);
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
