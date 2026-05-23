// TypeScript declarations for @kittui/koffi.

export interface Rgba {
  /** 4-tuple `[r, g, b, a]` with `0..=255` channels. The kittui FFI accepts
   * this form directly. */
  0: number;
  1: number;
  2: number;
  3: number;
  length: 4;
}

export interface CellRect {
  x: number;
  y: number;
  cols: number;
  rows: number;
}

export interface CellSize {
  width_px: number;
  height_px: number;
}

export interface PxRect {
  origin: [number, number];
  width: number;
  height: number;
}

export type Direction = 'horizontal' | 'vertical' | 'diagonal';

export type Paint =
  | { kind: 'solid'; color: Rgba }
  | { kind: 'linear'; direction: Direction; stops: Stop[] }
  | { kind: 'radial'; center_x_frac: number; center_y_frac: number; radius_frac: number; stops: Stop[] };

export interface Stop {
  offset: number;
  color: Rgba;
}

export interface Corners {
  tl: number;
  tr: number;
  bl: number;
  br: number;
}

export type Node =
  | { kind: 'rect'; rect: PxRect; fill: Paint; stroke: Stroke | null; corners: Corners }
  | { kind: 'gradient'; rect: PxRect; stops: Stop[]; direction: Direction }
  | {
      kind: 'glow';
      rect: PxRect;
      center_x_frac: number;
      center_y_frac: number;
      radius_frac: number;
      color: Rgba;
      intensity: number;
    }
  | { kind: 'scanlines'; rect: PxRect; alpha: number; period_px: number }
  | { kind: 'group'; opacity: number; children: Node[] }
  | { kind: 'composite'; mode: 'normal' | 'add' | 'multiply' | 'screen'; children: Node[] }
  | { kind: 'mask'; mask: Node; child: Node }
  | { kind: 'clip'; rect: PxRect; child: Node };

export interface Stroke {
  align: 'inside' | 'outside' | 'center';
  width_px: number;
  paint: Paint;
}

export interface Layer {
  label?: string;
  root: Node;
}

export type PhaseCurve =
  | { kind: 'linear' }
  | { kind: 'ease_in_out' }
  | { kind: 'pulse'; harmonics: number }
  | { kind: 'custom'; phases: number[] };

export interface Animation {
  frames: number;
  cycle_ms: number;
  curve: PhaseCurve;
  loops: number;
}

export interface Scene {
  footprint: CellRect;
  cell_size: CellSize;
  layers: Layer[];
  animation?: Animation;
}

export interface KittuiOptions {
  cacheDir?: string;
  renderer?: 'cpu' | 'gpu' | 'auto';
  transport?: 'direct' | 'tmux' | 'tmux_passthrough' | 'file' | 'memory' | 'shm' | 'shared';
  columns?: number;
  rows?: number;
  cellWidthPx?: number;
  cellHeightPx?: number;
  supportsKitty?: boolean;
  supportsUnicodePlaceholders?: boolean;
}

export class Kittui {
  static open(options?: KittuiOptions): Promise<Kittui>;
  static openWithLibrary(libraryPath: string, options?: KittuiOptions): Promise<Kittui>;

  abiVersion(): { major: number; minor: number };
  probe(): Record<string, unknown>;
  unplace(imageId: number | string): string;
  place(scene: Scene | string): string;
  placeAt(scene: Scene | string, x: number, y: number): string;
  placeMany(scenes: (Scene | string)[]): string[];
  close(): void;
}

export const scene: {
  build(args: { footprintCells: [number, number]; cellSize?: CellSize; layers: Layer[]; animation?: Animation }): Scene;
  backgroundSolid(rgba: Rgba): Layer;
  backgroundLinear(direction: Direction, startRgba: Rgba, endRgba: Rgba): Layer;
};

export const KittuiStatus: {
  readonly Ok: 0;
  readonly NullPointer: 1;
  readonly BadScene: 2;
  readonly Runtime: 3;
  readonly Panic: 4;
};
