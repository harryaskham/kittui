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

import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
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
    this._kittui_runtime_configure = this.lib.func('int kittui_runtime_configure(void* runtime, const char* json)');
    this._kittui_runtime_free = this.lib.func('void kittui_runtime_free(void* runtime)');
    this._kittui_string_free = this.lib.func('void kittui_string_free(void* ptr)');
    this._kittui_bytes_free = this.lib.func('void kittui_bytes_free(void* ptr, size_t len)');
    this._kittui_probe_json = this.lib.func('KittuiOwnedStr kittui_probe_json(void* runtime)');
    this._kittui_unplace = this.lib.func('KittuiOwnedStr kittui_unplace(void* runtime, uint32_t image_id)');
    this._kittui_last_error = this.lib.func('KittuiOwnedStr kittui_last_error(void* runtime)');
    // `KittuiOwnedStr` ties the auto-decoded `char**` to kittui_string_free
    // so ownership of the C buffer transfers cleanly into JS.
    this._kittui_render_json = this.lib.func(
      'int kittui_render_json(void* runtime, const char* scene_json, _Out_ void** out_ptr, _Out_ size_t* out_len)',
    );
    this._kittui_render_many_json = this.lib.func(
      'int kittui_render_many_json(void* runtime, const char* scenes_json, _Out_ KittuiOwnedStr* out)',
    );
    this._kittui_place_json = this.lib.func(
      'int kittui_place_json(void* runtime, const char* scene_json, _Out_ KittuiOwnedStr* out)',
    );
    this._kittui_place_json_at = this.lib.func(
      'int kittui_place_json_at(void* runtime, const char* scene_json, uint16_t x, uint16_t y, _Out_ KittuiOwnedStr* out)',
    );
    this._kittui_place_many_json = this.lib.func(
      'int kittui_place_many_json(void* runtime, const char* scenes_json, _Out_ KittuiOwnedStr* out)',
    );
    this._kittui_place_many_json_at = this.lib.func(
      'int kittui_place_many_json_at(void* runtime, const char* scenes_json, uint16_t x, uint16_t y, _Out_ KittuiOwnedStr* out)',
    );
    this._kittui_place_many_json_channels = this.lib.func(
      'int kittui_place_many_json_channels(void* runtime, const char* scenes_json, uint16_t x, uint16_t y, _Out_ KittuiOwnedStr* out)',
    );
    this._kittui_abi_version = this.lib.func('uint32_t kittui_abi_version()');
  }

  _ffiError(name, status) {
    let detail = '';
    if (this.runtime && this._kittui_last_error) {
      try {
        detail = this._kittui_last_error(this.runtime) || '';
      } catch (_err) {
        detail = '';
      }
    }
    return new Error(`${name} failed: status=${status}${detail ? `: ${detail}` : ''}`);
  }

  _parseImageId(imageId) {
    if (typeof imageId === 'number') return imageId >>> 0;
    if (typeof imageId === 'string') {
      const trimmed = imageId.trim();
      const value = trimmed.startsWith('0x') || trimmed.startsWith('0X')
        ? Number.parseInt(trimmed.slice(2), 16)
        : Number.parseInt(trimmed, 10);
      if (Number.isFinite(value) && value >= 0) return value >>> 0;
    }
    throw new Error(`invalid image id: ${imageId}`);
  }

  /**
   * Reconfigure the live runtime. Accepts the same options as `open()`.
   * @param {object} options
   * @returns {Kittui}
   */
  configure(options = {}) {
    if (!this.runtime) throw new Error('kittui runtime closed');
    const status = this._kittui_runtime_configure(this.runtime, runtimeConfigJson(options));
    if (status !== KittuiStatus.Ok) {
      throw this._ffiError('kittui_runtime_configure', status);
    }
    return this;
  }

  /**
   * Returns the loaded library's ABI version as a `{major, minor}` object.
   */
  abiVersion() {
    const packed = this._kittui_abi_version();
    return { major: (packed >>> 16) & 0xffff, minor: packed & 0xffff };
  }

  /**
   * Probe the runtime and return ABI/renderer/transport metadata.
   * @returns {object}
   */
  probe() {
    if (!this.runtime) throw new Error('kittui runtime closed');
    const json = this._kittui_probe_json(this.runtime);
    if (!json) throw new Error('kittui_probe_json returned null');
    return JSON.parse(json);
  }

  /**
   * Delete an uploaded image id from the terminal.
   * @param {number|string} imageId decimal number/string or 0x-prefixed string.
   * @returns {string}
   */
  unplace(imageId) {
    if (!this.runtime) throw new Error('kittui runtime closed');
    const bytes = this._kittui_unplace(this.runtime, this._parseImageId(imageId));
    if (bytes === null || bytes === undefined) throw new Error('kittui_unplace returned null');
    return bytes || '';
  }

  /**
   * Render a scene to PNG bytes without terminal placement.
   *
   * @param {object|string} scene A kittui Scene as a plain JS object or JSON string.
   * @returns {Uint8Array}
   */
  render(scene) {
    if (!this.runtime) throw new Error('kittui runtime closed');
    const json = typeof scene === 'string' ? scene : JSON.stringify(scene);
    const ptrBox = [null];
    const lenBox = [0];
    const status = this._kittui_render_json(this.runtime, json, ptrBox, lenBox);
    if (status !== KittuiStatus.Ok) {
      throw this._ffiError('kittui_render_json', status);
    }
    const ptr = ptrBox[0];
    const len = Number(lenBox[0] || 0);
    try {
      if (Array.isArray(ptr) || ArrayBuffer.isView(ptr)) {
        return Uint8Array.from(ptr).slice(0, len);
      }
      return Uint8Array.from(koffi.decode(ptr, 'uint8_t', len));
    } finally {
      if (ptr) this._kittui_bytes_free(ptr, len);
    }
  }

  /**
   * Render multiple scenes to a JSON manifest with base64 PNG entries.
   *
   * @param {(object|string)[]} scenes
   * @returns {object}
   */
  renderMany(scenes) {
    if (!this.runtime) throw new Error('kittui runtime closed');
    const normalized = scenes.map((scene) => (typeof scene === 'string' ? JSON.parse(scene) : scene));
    const outBox = [null];
    const status = this._kittui_render_many_json(this.runtime, JSON.stringify(normalized), outBox);
    if (status !== KittuiStatus.Ok) {
      throw this._ffiError('kittui_render_many_json', status);
    }
    return JSON.parse(outBox[0] || '{"count":0,"images":[]}');
  }

  /**
   * Render multiple scenes and write deterministic PNG files plus manifest.json.
   * @param {(object|string)[]} scenes
   * @param {string} outDir
   * @param {{ prefix?: string }} options
   * @returns {object}
   */
  renderManyToDir(scenes, outDir, options = {}) {
    const prefix = options.prefix || 'scene';
    const manifest = this.renderMany(scenes);
    mkdirSync(outDir, { recursive: true });
    const images = (manifest.images || []).map((image, fallbackIndex) => {
      const index = Number.isInteger(image.index) ? image.index : fallbackIndex;
      const file = `${prefix}-${String(index).padStart(5, '0')}.png`;
      writeFileSync(join(outDir, file), Buffer.from(image.png_base64 || '', 'base64'));
      return { ...image, file };
    });
    const written = { ...manifest, images, out_dir: outDir };
    const manifestPath = join(outDir, 'manifest.json');
    writeFileSync(manifestPath, `${JSON.stringify(written, null, 2)}\n`);
    return { ...written, manifest: manifestPath };
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
      throw this._ffiError('kittui_place_json', status);
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
      throw this._ffiError('kittui_place_json_at', status);
    }
    return outBox[0] || '';
  }

  /**
   * Render and place multiple scenes in one round-trip across the FFI
   * boundary. Returns one concatenated batch byte string.
   *
   * @param {(object|string)[]} scenes
   * @returns {string}
   */
  placeMany(scenes) {
    if (!this.runtime) throw new Error('kittui runtime closed');
    const normalized = scenes.map((scene) => (typeof scene === 'string' ? JSON.parse(scene) : scene));
    const outBox = [null];
    const status = this._kittui_place_many_json(this.runtime, JSON.stringify(normalized), outBox);
    if (status !== KittuiStatus.Ok) {
      throw this._ffiError('kittui_place_many_json', status);
    }
    return outBox[0] || '';
  }

  /**
   * Render and place multiple scenes at an explicit batch origin in one
   * FFI round-trip. The batch's minimum x/y maps to x/y and relative
   * offsets are preserved.
   *
   * @param {(object|string)[]} scenes
   * @param {number} x Terminal x column for the batch origin.
   * @param {number} y Terminal y row for the batch origin.
   * @returns {string}
   */
  placeManyAt(scenes, x, y) {
    if (!this.runtime) throw new Error('kittui runtime closed');
    const normalized = scenes.map((scene) => (typeof scene === 'string' ? JSON.parse(scene) : scene));
    const outBox = [null];
    const status = this._kittui_place_many_json_at(this.runtime, JSON.stringify(normalized), x, y, outBox);
    if (status !== KittuiStatus.Ok) {
      throw this._ffiError('kittui_place_many_json_at', status);
    }
    return outBox[0] || '';
  }

  /**
   * Render and place multiple scenes at a batch origin, returning parsed
   * channel JSON with upload/placement/embed strings and metadata.
   *
   * @param {(object|string)[]} scenes
   * @param {number} x Terminal x column for the batch origin.
   * @param {number} y Terminal y row for the batch origin.
   * @returns {object}
   */
  placeManyChannels(scenes, x = 0, y = 0) {
    if (!this.runtime) throw new Error('kittui runtime closed');
    const normalized = scenes.map((scene) => (typeof scene === 'string' ? JSON.parse(scene) : scene));
    const outBox = [null];
    const status = this._kittui_place_many_json_channels(this.runtime, JSON.stringify(normalized), x, y, outBox);
    if (status !== KittuiStatus.Ok) {
      throw this._ffiError('kittui_place_many_json_channels', status);
    }
    return JSON.parse(outBox[0] || '{}');
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

  /** Solid rectangle layer sized in cells. */
  rectLayer({ cols, rows, rgba, cellSize = { width_px: 8, height_px: 16 }, label = 'background', radius = 0 }) {
    return {
      label,
      root: {
        kind: 'rect',
        rect: { origin: [0, 0], width: cols * cellSize.width_px, height: rows * cellSize.height_px },
        fill: { kind: 'solid', color: rgba },
        stroke: null,
        corners: { tl: radius, tr: radius, bl: radius, br: radius },
      },
    };
  },

  /** Complete solid-box scene. */
  solidBox({ cols, rows, rgba, cellSize = { width_px: 8, height_px: 16 }, label = 'background', radius = 0 }) {
    return this.build({
      footprintCells: [cols, rows],
      cellSize,
      layers: [this.rectLayer({ cols, rows, rgba, cellSize, label, radius })],
    });
  },

  /** Two-stop gradient layer sized in cells. */
  gradientLayer({ cols, rows, start, end, direction = 'horizontal', cellSize = { width_px: 8, height_px: 16 }, label = 'background' }) {
    return {
      label,
      root: {
        kind: 'gradient',
        rect: { origin: [0, 0], width: cols * cellSize.width_px, height: rows * cellSize.height_px },
        stops: [
          { offset: 0.0, color: start },
          { offset: 1.0, color: end },
        ],
        direction,
      },
    };
  },

  /** Complete two-stop gradient scene. */
  gradientBox({ cols, rows, start, end, direction = 'horizontal', cellSize = { width_px: 8, height_px: 16 }, label = 'background' }) {
    return this.build({
      footprintCells: [cols, rows],
      cellSize,
      layers: [this.gradientLayer({ cols, rows, start, end, direction, cellSize, label })],
    });
  },

  /** Glow layer sized in cells. */
  glowLayer({ cols, rows, rgba, intensity = 0.8, centerXFrac = 0.5, centerYFrac = 0.5, radiusFrac = 0.5, cellSize = { width_px: 8, height_px: 16 }, label = 'glow' }) {
    return {
      label,
      root: {
        kind: 'glow',
        rect: { origin: [0, 0], width: cols * cellSize.width_px, height: rows * cellSize.height_px },
        center_x_frac: centerXFrac,
        center_y_frac: centerYFrac,
        radius_frac: radiusFrac,
        color: rgba,
        intensity,
      },
    };
  },

  /** Complete glow scene. */
  glowBox({ cols, rows, rgba, intensity = 0.8, cellSize = { width_px: 8, height_px: 16 }, label = 'glow' }) {
    return this.build({
      footprintCells: [cols, rows],
      cellSize,
      layers: [this.glowLayer({ cols, rows, rgba, intensity, cellSize, label })],
    });
  },

  /** Scanlines layer sized in cells. */
  scanlinesLayer({ cols, rows, alpha = 32, periodPx = 2, cellSize = { width_px: 8, height_px: 16 }, label = 'scanlines' }) {
    return {
      label,
      root: {
        kind: 'scanlines',
        rect: { origin: [0, 0], width: cols * cellSize.width_px, height: rows * cellSize.height_px },
        alpha,
        period_px: periodPx,
      },
    };
  },

  /** Complete scanlines scene. */
  scanlinesBox({ cols, rows, alpha = 32, periodPx = 2, cellSize = { width_px: 8, height_px: 16 }, label = 'scanlines' }) {
    return this.build({
      footprintCells: [cols, rows],
      cellSize,
      layers: [this.scanlinesLayer({ cols, rows, alpha, periodPx, cellSize, label })],
    });
  },

  /** Image layer sized in cells. `src` may be a path string or byte array. */
  imageLayer({ cols, rows, src, fit = 'contain', tint = null, cellSize = { width_px: 8, height_px: 16 }, label = 'image' }) {
    const imageSrc = typeof src === 'string'
      ? { kind: 'path', path: src }
      : { kind: 'bytes', bytes: Array.from(src) };
    return {
      label,
      root: {
        kind: 'image',
        rect: { origin: [0, 0], width: cols * cellSize.width_px, height: rows * cellSize.height_px },
        src: imageSrc,
        fit,
        tint,
      },
    };
  },

  /** Complete image scene. */
  imageBox({ cols, rows, src, fit = 'contain', tint = null, cellSize = { width_px: 8, height_px: 16 }, label = 'image' }) {
    return this.build({
      footprintCells: [cols, rows],
      cellSize,
      layers: [this.imageLayer({ cols, rows, src, fit, tint, cellSize, label })],
    });
  },

  /** Solid-background layer placeholder for callers that size rects themselves. */
  backgroundSolid(rgba) {
    return {
      label: 'background',
      root: {
        kind: 'rect',
        rect: { origin: [0, 0], width: 0, height: 0 },
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
