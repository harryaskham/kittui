# kittwm dirty-grid kitty frame update investigation

Tracking bead: `bd-510a36`

## Summary

Kitty's animation protocol is useful for storing multiple frames under one image
id and letting the terminal play or update frames, but it is not a complete
compositor protocol. It does not by itself make arbitrary sub-rectangle pixel
patches safe for a live WM surface. kittwm can still track dirtiness internally
and use that information for future transport choices, but the default renderer
should continue to use full-frame replacement until a terminal-compatible delta
path is proven.

This bead adds an opt-in-safe prototype layer: a dirty grid diff model that can
identify changed tiles in RGBA frames. It does not change live kittwm rendering
or assume kitty partial updates are correct.

## Current safe path

The current native graphics hot path remains:

1. capture/render surface into a bounded RGBA frame;
2. delete any previous terminal image payload for the image id;
3. upload a replacement raw RGBA frame (`f=32`), optionally with kitty zlib
   compression (`o=z`);
4. place the image in the allocated shell frame.

Inside tmux, kittwm defaults to the pure terminal renderer unless graphics are
explicitly forced.

## Dirty-grid model

A dirty-grid model splits a surface into fixed-size pixel tiles, hashes each
tile, and compares hashes between revisions:

```text
RGBA frame -> tile hashes -> changed tile list
```

The model is useful for:

- deciding whether to skip an upload entirely when no pixels changed;
- measuring how dirty a frame is;
- choosing between future full-frame upload, compressed upload, file/shm upload,
  or experimental region update;
- driving tests for renderer dirtiness without involving a terminal.

It is not sufficient alone for safe partial display updates. Any terminal-facing
partial update must also define composition order, stale-pixel cleanup, fallback,
frame identity, and compatibility behavior.

## Kitty animation considerations

Kitty graphics supports animation commands (`a=t`, `a=f`, `a=a`) and frame
indexes. That lets a terminal retain multiple full image frames and play/switch
between them. It does not obviously provide a portable "patch this subrectangle
of the current frame" primitive for arbitrary WM surfaces.

Possible experimental strategies:

1. **Skip unchanged full frames**: safest. If dirty grid reports no dirty tiles,
   do not upload. If any tiles changed, upload the full bounded frame.
2. **Region image overlays**: upload dirty rectangles as separate images and
   place them over a base image. Risk: stale overlay cleanup, ordering, too many
   placements, and terminal memory fragmentation.
3. **Frame replacement through animation indexes**: keep a small ring of full
   frames under one image id and update/play the current visual frame. Risk:
   protocol/terminal differences and still full-frame bandwidth unless paired
   with another transport.
4. **Future terminal-specific patching**: only if a terminal exposes a verified
   extension for sub-image updates.

Only strategy 1 is safe enough for default behavior today.

## Prototype API

`kittui-wm::dirty` provides a pure in-memory helper:

- `DirtyGrid::new(tile_width_px, tile_height_px)`;
- `DirtyGrid::diff_rgba(width, height, rgba)`;
- `DirtyFrameDiff` with `first_frame`, `tiles`, `changed_tiles`, and
  `changed_fraction()`;
- `DirtyTile` rectangles in pixel coordinates.

The helper deliberately does not emit kitty escape codes. It is a correctness and
policy input, not a transport implementation.

## Runtime policy recommendation

Initial runtime integration is conservative and opt-in:

```text
KITTWM_DIRTY_FRAMES=skip-unchanged
```

Modes:

- unset / `off`: current full-frame behavior;
- `skip-unchanged`: use dirty grid only to skip identical frames while still redrawing placement/embed text;
- `measure`: log or expose dirty fraction in status without changing output;
- `overlay-experimental`: future non-default dirty-rectangle overlay prototype.

`overlay-experimental` should remain off by default until tested across kitty,
Ghostty, tmux passthrough, remote SSH, and zlib/raw RGBA combinations.

## Risks

- stale pixels if a dirty region misses an edge case;
- flicker if overlays arrive out of order;
- terminal memory growth from many region images/placements;
- tmux passthrough amplification;
- worse bandwidth for many small dirty rectangles due per-command overhead;
- mismatched frame timing if animation commands race placement/deletion;
- different behavior across kitty-compatible terminals.

## Follow-up implementation beads

Recommended follow-ups:

1. `kittwm: use dirty grid to skip unchanged raw frame uploads` — landed as opt-in `KITTWM_DIRTY_FRAMES=skip-unchanged` behavior (`bd-889f33`).
2. `kittwm: expose dirty-frame metrics in native status/events` — useful for
   renderer policy and debugging.
3. `kittwm: prototype dirty-rectangle overlay transport behind env flag` — upload
   dirty regions as bounded images with aggressive cleanup and terminal guards.
4. `kittui-kitty: add explicit animation frame update primitives` — typed helpers
   for full-frame animation/ring-buffer experiments, separate from partial
   region overlays.
