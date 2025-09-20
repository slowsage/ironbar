#!/usr/bin/env bash
# Emit (on stdout) the apt -dev packages needed to build this crate, derived from
# the pkg-config modules its dependency tree declares via
# `[package.metadata.system-deps]` / `[package.metadata.pkg-config]`, mapped to
# packages through apt-file. Diagnostics go to stderr.
#
# Requires: cargo, jq, and an up-to-date apt-file database (`apt-file update`).
set -euo pipefail

# pkg-config module names declared by any crate in the dependency tree. For
# system-deps the real module is the `name` field (the table key may be a
# sanitised identifier, e.g. gdk_pixbuf_2_0 -> gdk-pixbuf-2.0); for the older
# pkg-config convention the table key is the module name.
modules=$(cargo metadata --format-version=1 | jq -r '
  .packages[].metadata as $m
  | (($m["system-deps"] // {}) | to_entries[] | .value.name // .key),
    (($m["pkg-config"]   // {}) | keys[])
' | sort -u)

echo "pkg-config modules: $(echo "$modules" | tr '\n' ' ')" >&2

# Resolve each module's <module>.pc file to the package that ships it.
for m in $modules; do
  apt-file search --package-only --regexp "/${m}\.pc\$" \
    || echo "WARN: no apt package ships ${m}.pc (crate may vendor it)" >&2
done | sort -u
