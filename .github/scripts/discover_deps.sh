#!/usr/bin/env bash
# Emit (on stdout) the apt -dev packages needed to build this crate, derived from
# the pkg-config modules its dependency tree declares via
# `[package.metadata.system-deps]` / `[package.metadata.pkg-config]`, mapped to
# packages through apt-file. Diagnostics go to stderr.
#
# Requires: cargo, jq, and an up-to-date apt-file database (`apt-file update`).
set -euo pipefail

# Collect pkg-config module names declared by crates in the *resolved* dependency
# graph (so disabled optional crates are skipped), honouring per-feature gating:
#
#   - resolve.nodes gives each package id its set of active feature names.
#   - a system-deps entry with `feature = "x"` is included only when feature `x`
#     is active for that package (e.g. mlua-sys declares both `lua` and `luajit`,
#     gated by features `lua5x` / `luajit`; only the enabled one is emitted).
#   - entries without a `feature` (including `optional = true` ones, e.g. a
#     vendored-fallback libevdev) are always included.
#   - for system-deps the module is the `.name` field (the table key may be a
#     sanitised identifier, e.g. gdk_pixbuf_2_0 -> gdk-pixbuf-2.0); for the older
#     pkg-config convention the table key is the module name.
modules=$(cargo metadata --format-version=1 | jq -r '
  (reduce (.resolve.nodes[]?) as $n ({}; .[$n.id] = ($n.features // []))) as $feat
  | .packages[] as $p
  | select($feat | has($p.id))
  | ($feat[$p.id]) as $active
  | ($p.metadata // {}) as $m
  | (
      ( ($m["system-deps"] // {}) | to_entries[] | . as $e
        | (if ($e.value | type) == "object" then $e.value.feature else null end) as $f
        | select($f == null or ($active | index($f)))
        | (if ($e.value | type) == "object" then ($e.value.name // $e.key) else $e.key end)
      ),
      ( ($m["pkg-config"] // {}) | keys[] )
    )
' | sort -u)

echo "pkg-config modules: $(echo "$modules" | tr '\n' ' ')" >&2

# Resolve each module's <module>.pc file to the package that ships it.
for m in $modules; do
  apt-file search --package-only --regexp "/${m}\.pc\$" \
    || echo "WARN: no apt package ships ${m}.pc (crate may vendor it)" >&2
done | sort -u
