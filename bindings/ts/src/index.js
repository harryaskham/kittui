// Pure-JS wrapper around libkittui_ffi using `koffi`.
//
// Build-step-free: this module dlopens the kittui FFI cdylib and exposes
// a small typed surface. Scenes are passed as JSON strings; placements
// are returned as plain JS strings ready to be written to the terminal.
//
// The library probes a few standard paths to find libkittui_ffi:
//   1. KITTUI_LIB_PATH env var
//   2. <repo>/target/release/libkittui_ffi.{so,dylib,dll}
//   3. <repo>/target/debug/libkittui_ffi.{so,dylib,dll}
//   4. System library search path (`libkittui_ffi`)
//
// The first that loads wins. Hosts that ship their own binary should
// either set KITTUI_LIB_PATH or call Kittui.openWithLibrary(path).

import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import koffi from 'koffi';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const REPO_ROOT = resolve(__dirname, '..', '..', '..');

function platformLibName() {
  switch (process.platform) {
    case 'darwin': return 'libkittui_ffi.dylib';
    case 'win32': return 'kittui_ffi.dll';
    default: return 'libkittui_ffi.so';
  }
}

function candidatePaths() {
  const name = platformLibName();
  const paths = [];
  if (process.env.KITTUI_LIB_PATH) paths.push(process.env.KITTUI_LIB_PATH);
  paths.push(join(REPO_ROOT, 'target', 'release', name));
  paths.push(join(REPO_ROOT, 'target', 'debug', name));
  paths.push(name); // search system path
  return paths;
}

function loadLibrary(explicitPath) {
  const tried = [];
  const paths = explicitPath ? [explicitPath] : candidatePaths();
  for (const candidate of paths) {
    if (candidate.includes('/') && !existsSync(candidate)) {
      tried.push(`${candidate} (not found)`);
      continue;
    }
    try {
      return koffi.load(candidate);
    } catch (err) {
      tried.push(`${candidate} (${err.message})`);
    }
  }
  throw new Error(
    `kittui: failed to load libkittui_ffi. Tried:\n  - ${tried.join('\n  - ')}\n` +
    'Set KITTUI_LIB_PATH or pass an explicit path to Kittui.openWithLibrary().',
  );
}

const KittuiStatus = Object.freeze({
  Ok: 0,
  NullPointer: 1,
  BadScene: 2,
  Runtime: 3,
  Panic: 4,
});

let ffiTypesWired = false;

function hasRuntimeConfig(options) {
  return [
    'renderer',
    'transport',
    'columns',
    'rows',
    'cellWidthPx',
    'cellHeightPx',
    'supportsKitty',
    'supportsUnicodePlaceholders',
  ].some((key) => Object.prototype.hasOwnProperty.call(options, key));
}

function runtimeConfigJson(options) {
  const cfg = {};
  if (options.cacheDir !== undefined) cfg.cache_dir = options.cacheDir;
  if (options.renderer !== undefined) cfg.renderer = options.renderer;
  if (options.transport !== undefined) cfg.transport = options.transport;
  if (options.columns !== undefined) cfg.columns = options.columns;
  if (options.rows !== undefined) cfg.rows = options.rows;
  if (options.cellWidthPx !== undefined) cfg.cell_width_px = options.cellWidthPx;
  if (options.cellHeightPx !== undefined) cfg.cell_height_px = options.cellHeightPx;
  if (options.supportsKitty !== undefined) cfg.supports_kitty = options.supportsKitty;
  if (options.supportsUnicodePlaceholders !== undefined) {
    cfg.supports_unicode_placeholders = options.supportsUnicodePlaceholders;
  }
  return JSON.stringify(cfg);
}

/**
 * High-level wrapper around the kittui FFI surface.
 */
export class Kittui {
  /**
   * Open a kittui runtime using the auto-discovered library path.
   * @param {{ cacheDir?: string }} options
   */
  static async open(options = {}) {
    return new Kittui(loadLibrary(), options);
  }

  /**
   * Open a kittui runtime using an explicit library path.
   * @param {string} libraryPath
   * @param {{ cacheDir?: string }} options
   */
  static async openWithLibrary(libraryPath, options = {}) {
    return new Kittui(loadLibrary(libraryPath), options);
  }

  constructor(lib, options) {
    this.lib = lib;
    this._wire();
    if (hasRuntimeConfig(options)) {
      this.runtime = this._kittui_runtime_new_config(runtimeConfigJson(options));
      if (this.runtime === null || this.runtime === undefined) {
        throw new Error('kittui_runtime_new_config returned null');
      }
    } else {
      const cachePtr = options.cacheDir ? options.cacheDir : null;
      this.runtime = this._kittui_runtime_new(cachePtr);
      if (this.runtime === null || this.runtime === undefined) {
        throw new Error('kittui_runtime_new returned null');
      }
    }
  }

  _wire() {
    if (!ffiTypesWired) {
      koffi.opaque('KittuiRuntime');
      koffi.alias('KittuiOwnedStr', koffi.disposable('char*', this.lib.func('void kittui_string_free(void* ptr)')));
      ffiTypesWired = true;
    }
    this._kittui_runtime_new = this.lib.func('void* kittui_runtime_new(const char* cache_dir)');
    this._kittui_runtime_new_config = this.lib.func('void* kittui_runtime_new_config(const char* json)');
    this._kittui_runtime_free = this.lib.func('void kittui_runtime_free(void* runtime)');
    this._kittui_string_free = this.lib.func('void kittui_string_free(void* ptr)');
    // `KittuiOwnedStr` ties the auto-decoded `char**` to kittui_string_free
    // so ownership of the C buffer transfers cleanly into JS.
    this._kittui_place_json = this.lib.func(
      'int kittui_place_json(void* runtime, const char* scene_json, _Out_ KittuiOwnedStr* out)',
    );
    this._kittui_place_json_at = this.lib.func(
      'int kittui_place_json_at(void* runtime, const char* scene_json, uint16_t x, uint16_t y, _Out_ KittuiOwnedStr* out)',
    );
    this._kittui_abi_version = this.lib.func('uint32_t kittui_abi_version()');
  }

  /**
   * Returns the loaded library's ABI version as a `{major, minor}` object.
   */
  abiVersion() {
    const packed = this._kittui_abi_version();
    return { major: (packed >>> 16) & 0xffff, minor: packed & 0xffff };
  }

  /**
   * Render and place a scene. Returns a string containing the upload,
   * placement, and embed bytes concatenated. Hosts can split the result
   * before writing if they want to interleave with their own output.
   *
   * @param {object} scene  A kittui Scene as a plain JS object.
   * @returns {string}
   */
  place(scene) {
    if (!this.runtime) throw new Error('kittui runtime closed');
    const json = typeof scene === 'string' ? scene : JSON.stringify(scene);
    const outBox = [null];
    const status = this._kittui_place_json(this.runtime, json, outBox);
    if (status !== KittuiStatus.Ok) {
      throw new Error(`kittui_place_json failed: status=${status}`);
    }
    // koffi decodes the C string into JS and runs the disposable's
    // destructor (kittui_string_free) once decoding completes.
    return outBox[0] || '';
  }

  /**
   * Render/cache a scene but place it at explicit terminal coordinates.
   * The scene's own width/height remain the render/cache footprint.
   *
   * @param {object|string} scene A kittui Scene as a plain JS object or JSON string.
   * @param {number} x Terminal x column.
   * @param {number} y Terminal y row.
   * @returns {string}
   */
  placeAt(scene, x, y) {
    if (!this.runtime) throw new Error('kittui runtime closed');
    const json = typeof scene === 'string' ? scene : JSON.stringify(scene);
    const outBox = [null];
    const status = this._kittui_place_json_at(this.runtime, json, x, y, outBox);
    if (status !== KittuiStatus.Ok) {
      throw new Error(`kittui_place_json_at failed: status=${status}`);
    }
    return outBox[0] || '';
  }

  /**
   * Render and place multiple scenes in one round-trip across the FFI
   * boundary. Returns an array of strings in the same order.
   *
   * @param {object[]} scenes
   * @returns {string[]}
   */
  placeMany(scenes) {
    return scenes.map((scene) => this.place(scene));
  }

  /**
   * Free the underlying runtime. Subsequent calls throw.
   */
  close() {
    if (this.runtime) {
      this._kittui_runtime_free(this.runtime);
      this.runtime = null;
    }
  }
}

/**
 * Convenience helpers for building scene JSON without touching the raw
 * schema. Mirror the primitive builders in the kittui Rust facade — the
 * library never grows affordances on the Rust side, so the JS helpers
 * stay equally minimal.
 */
export const scene = {
  /** Construct a scene wrapper object. */
  build({ footprintCells, cellSize = { width_px: 8, height_px: 16 }, layers, animation }) {
    const [cols, rows] = footprintCells;
    return {
      footprint: { x: 0, y: 0, cols, rows },
      cell_size: { width_px: cellSize.width_px, height_px: cellSize.height_px },
      layers,
      ...(animation ? { animation } : {}),
    };
  },

  /** Solid-background layer. */
  backgroundSolid(rgba) {
    return {
      label: 'background',
      root: {
        kind: 'rect',
        rect: { origin: [0, 0], width: 0, height: 0 }, // placeholder; sized at place time
        fill: { kind: 'solid', color: rgba },
        stroke: null,
        corners: { tl: 0, tr: 0, bl: 0, br: 0 },
      },
    };
  },

  /** Two-stop linear gradient layer. */
  backgroundLinear(direction, startRgba, endRgba) {
    return {
      label: 'background',
      root: {
        kind: 'gradient',
        rect: { origin: [0, 0], width: 0, height: 0 },
        stops: [
          { offset: 0.0, color: startRgba },
          { offset: 1.0, color: endRgba },
        ],
        direction,
      },
    };
  },
};

export { KittuiStatus };
