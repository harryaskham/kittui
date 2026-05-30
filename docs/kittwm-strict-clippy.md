# kittwm strict-clippy baseline

This repo is moving toward a clean strict-clippy baseline so agents can run
`cargo clippy -- -D warnings` on a touched crate without tripping over
pre-existing warnings (the friction documented in `bd-dc44f1`). Until every
crate is clean, strict clippy is adopted **crate-by-crate**.

## Clean crates (strict-clippy clean, `--all-targets`)

These 13 crates pass `cargo clippy -p <crate> --all-targets -- -D warnings`:

- `kittui-core`
- `kittui-cache`
- `kittui-kitty`
- `kittui-render-cpu`
- `kittui-render-gpu`
- `kittui` (facade)
- `kittui-overlay`
- `kittui-tmux`
- `ratakittui`
- `kittui-ghostty-vt`
- `kittui-ffi`
- `kittui-quartz`
- `kittui-xvfb`

Run the guard to confirm the baseline still holds (fails non-zero on regression):

```sh
scripts/kittwm-strict-clippy.sh            # whole baseline
scripts/kittwm-strict-clippy.sh kittui-kitty kittui-core   # subset
```

The script defaults to `LIBGHOSTTY_VT_NO_PKG_CONFIG=1` (stub `libghostty-vt`
build) so it runs without a system libghostty.

## Not-yet-clean crates (excluded from the baseline)

These heavier, actively developed crates still carry pre-existing warnings and
are **excluded** from strict mode for now:

- `kittui-cli`
- `kittwm-sdk`
- `kittui-wm`

For these, run **non-strict** clippy as smoke (`cargo clippy -p <crate>`) and
rely on targeted tests plus `cargo build`. Their owners can clear the warnings
and graduate each crate into `scripts/kittwm-strict-clippy.sh` when ready.

## Policy: real fix vs scoped allow

When making a crate strict-clippy clean, prefer in this order:

1. **Real idiomatic fix** when behaviour-identical and low risk — e.g.
   `unwrap_or_default()`, deriving `Default`, removing a no-op
   `.map_err(io::Error::from)`, dropping an always-true `u8 <= 255` comparison,
   `to_vec()` over `iter().copied().collect()`, `!x.is_empty()` over
   `x.len() > 0`, `io::Error::other(..)`.
2. **Per-call scoped `#[allow(clippy::...)]` with a one-line rationale** when the
   lint flags a deliberate, intrinsic shape — e.g.
   `clippy::too_many_arguments` on the kitty upload encoders (id + data + dims +
   format + transport + quiet + compression are intrinsic), `self_named_constructors`
   on the ergonomic `Rgba::rgba` (150+ call sites; rename infeasible), or
   `large_enum_variant` on a hot enum where boxing adds indirection.

Avoid crate-wide `#![allow(...)]` blanket suppressions; they hide future real
warnings.

## A note on deterministic tests

Strict-clippy/test validation is only trustworthy if the suite is deterministic.
`kittui-kitty`'s compression-grammar tests previously depended on ambient
`KITTUI_KITTY_COMPRESSION` and test order (other tests mutate that process-global
env). They now pin it via a `with_compression_none(...)` helper under the shared
`ENV_LOCK` and restore the prior value, so `cargo test -p kittui-kitty --lib` is
deterministically green across env and ordering (`bd-f2cc0f`). Any new test that
reads a process-global env knob should follow the same lock-set-restore pattern.
