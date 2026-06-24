# System package inference for CI

Goal: install the apt `-dev` packages needed to build, **derived from the crate's
dependency tree** instead of a hand-maintained list, with no trial-and-error.
Heaviest tool permitted: `apt-file`. Do not introduce a mapping crate or parse
distro `Contents` indexes by hand — `apt-file` already does that lookup.

## The inference chain

```
cargo metadata  ->  pkg-config module names  ->  apt-file  ->  apt -dev packages  ->  apt-get install
```

`cargo metadata` can name the pkg-config **module** (`gtk4`, `dbus-1`, `libinput`);
it can never name the apt **package** (`libgtk-4-dev`). The module->package hop is
a reverse lookup over "which package ships `/usr/.../pkgconfig/<module>.pc`", which
is exactly `apt-file search`. That makes `apt-file` the one irreducible layer:
nothing in `Cargo.toml` can replace it, because package names are distro/version
specific while module names are not.

## Step 1 - module names from cargo metadata

Two declarative conventions, both emitted verbatim under each package's `metadata`:

1. `[package.metadata.system-deps]` (gtk-rs stack, and any crate using `system-deps`).
   The real module is the `.name` field — the table key may be a sanitised
   identifier (`gdk_pixbuf_2_0` -> name `gdk-pixbuf-2.0`). Entries may carry
   `feature = "..."` (only needed when that cargo feature is active) and/or
   `optional = true`.
2. `[package.metadata.pkg-config]` (older convention, e.g. `libdbus-sys`:
   `dbus-1 = "1.6"`). Here the table **key** is the module name.

Extraction (read both conventions):

```bash
cargo metadata --format-version=1 | jq -r '
  .packages[].metadata as $m
  | (($m["system-deps"] // {}) | to_entries[] | .value.name // .key),
    (($m["pkg-config"]   // {}) | keys[])' | sort -u
```

Note: `links` (e.g. `pulse`, `dbus`, `lua`) is a bare library token, NOT the
pkg-config module name — do not use it for inference.

## Step 2 - module -> package with apt-file

```bash
apt-file search --package-only --regexp "/<module>\.pc$"   # gtk4 -> libgtk-4-dev
```

Deterministic: each `.pc` is shipped by (usually) exactly one `-dev` package.
Needs `apt-file update` once to pull `Contents-<arch>.gz`. On native runners
(including `ubuntu-24.04-arm`) that is a cheap download; under QEMU/`act` it is
slower but I/O-bound, not CPU-bound.

## The gap (and how it closes itself)

Crates that probe pkg-config in `build.rs` but expose no metadata table
contribute nothing to `cargo metadata`. For this repo that is currently:

| module   | crate         | closes when                              |
|----------|---------------|------------------------------------------|
| libinput | colpetto      | system-deps metadata PR lands + bump     |
| libpulse | libpulse-sys  | system-deps metadata PR lands + bump     |
| libevdev | evdev-sys     | system-deps metadata PR lands + bump     |
| luajit   | mlua-sys      | mlua PR #660 lands + bump                |

Bridge until then: keep a small explicit **supplement** list in CI for exactly
these. The supplement is self-shrinking — when a crate's metadata lands and this
repo bumps to that version, the module appears in `cargo metadata` and the
matching supplement line can be deleted. Comment each supplement entry with the
PR that retires it.

When fixing a crate upstream, prefer migrating its `build.rs` to consume
`system-deps` **only where clean** (no MSRV bump — `system-deps` needs Rust 1.78;
preserve any vendored/link-anyway fallback). Otherwise add the
`[package.metadata.system-deps]` table additively and leave `build.rs` untouched;
`cargo metadata` exposes the table regardless of whether the crate's own build
consumes it.

## Implementation in this repo

- `.github/scripts/discover_deps.sh` - emits packages (`cargo metadata` + `jq` ->
  `apt-file`); all diagnostics go to stderr, package list to stdout.
- `.github/workflows/binary.yml` install step - install `pkg-config jq apt-file`,
  run `apt-file update` (root/sudo-shim aware so it works under `act`), discover,
  then install `<discovered> <supplement>`; fail fast if discovery is empty.
- `.github/scripts/ubuntu_setup.sh` - the install primitive: env prep (QEMU
  `RUSTFLAGS`, sudo shim, apt noise suppression) + `apt-get install "$@"`. Also
  executed arg-less by the `Dockerfile`, so do NOT add `cargo metadata` discovery
  into it (no crate is checked out in that context).

## Rules and edge cases

- `apt-file` is the ceiling. No mapping crate, no hand-rolled `Contents` parsing.
- Over-inclusion is safe (extra `-dev` packages do not hurt the build);
  under-inclusion breaks it. Never silently drop a module that failed to resolve —
  warn loudly.
- `optional = true` entries mean the crate has a vendored/source fallback
  (e.g. `evdev-sys` compiles libevdev from source). Installing them is a
  build-speed optimisation, not a correctness requirement — decide per case.
  A pure-discovery consumer may skip them; this repo installs `libevdev-dev`
  anyway to avoid the in-emulation source compile.
- A package is only redundant in the explicit list if another listed package
  pulls it transitively (e.g. `libgtk-4-dev` already depends on
  `libgraphene-1.0-dev`). That is apt dependency transitivity, distinct from
  metadata discoverability — do not conflate the two when trimming.
- Scope is apt (Debian/Ubuntu) by design. Module names are distro-agnostic; the
  `.pc`->package mapping is not.
