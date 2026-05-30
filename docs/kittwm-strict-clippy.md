# kittwm strict-clippy baseline

This repo is moving toward a clean strict-clippy baseline so agents can run
`cargo clippy -- -D warnings` on a touched crate without tripping over
pre-existing warnings (the friction documented in `bd-dc44f1`). Until every
crate is clean, strict clippy is adopted **crate-by-crate**.

## Always measure with `--all-targets` (and feature-gated code under its features)

Strict-clippy status is **only** meaningful when measured with:

```sh
cargo clippy -p <crate> --all-targets -- -D warnings
```

A plain `cargo clippy -p <crate>` (or `--lib`) run is **NOT** sufficient evidence
of cleanliness and routinely gives **false-clean** signals, because it skips the
crate's bins, tests, examples, and benches. This bit us in `bd-89422b`: a
lib-only run reported "kittui-wm/kittui-cli have zero own lints" when
`--all-targets` showed kittui-wm had 12 own lints in `native.rs` and kittui-cli
had ~30 of its own — leading to a mis-scoped bead and a wrong peer claim.

Feature-gated code is a second false-clean trap: clippy only lints code that is
actually compiled. A crate can be clean under default features yet dirty under a
feature a *dependent* enables. `kittui-quartz` is the canonical example — its
ScreenCaptureKit path is behind `--features sck`/`quartz`, and those lints only
appeared once `kittui-cli` (which enables `sck`) pulled them in (`bd-c42fce`).
Check such crates under their features too, e.g.
`cargo clippy -p kittui-quartz --features sck --all-targets -- -D warnings`.
The guard script does this for `kittui-quartz` automatically.

When reporting strict-clippy status in messages/summaries, quote the exact
`--all-targets` command (the guard prints it per crate) so claims are
reproducible.


## Clean crates (strict-clippy clean, `--all-targets`)

The full workspace is strict-clippy clean: all **16 crates** pass
`cargo clippy -p <crate> --all-targets -- -D warnings` (`bd-dc44f1` complete):

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
- `kittui-quartz` (also under `--features sck`/`--all-features`)
- `kittui-xvfb`
- `kittwm-sdk`
- `kittui-wm`
- `kittui-cli`

Run the guard to confirm the baseline still holds (fails non-zero on regression):

```sh
scripts/kittwm-strict-clippy.sh            # whole baseline
scripts/kittwm-strict-clippy.sh kittui-kitty kittui-core   # subset
```

The script defaults to `LIBGHOSTTY_VT_NO_PKG_CONFIG=1` (stub `libghostty-vt`
build) so it runs without a system libghostty.

## Not-yet-clean crates (excluded from the baseline)

None — every workspace crate is in the baseline. Keep it that way: run
`scripts/kittwm-strict-clippy.sh` (it prints the canonical `--all-targets`
command and feature-checks `kittui-quartz` under `sck`) before landing, and fix
or scope-allow any new lint rather than dropping a crate back out of the
baseline.

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
