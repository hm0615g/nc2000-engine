#!/usr/bin/env bash
# Canonical wasm build (M9c tuned profile).
#
# wasm-pack 0.13 has no custom-profile flag, so the wasm-only rustc profile
# (fat LTO + 1 codegen unit; native `release` untouched) rides in as cargo
# env overrides. wasm-opt level comes from [package.metadata.wasm-pack]
# in Cargo.toml (-O3; -O4 measured equal, default -O slower).
#
#   crates/wasm/build.sh          # web target  -> pkg-web  (the Vite app)
#   crates/wasm/build.sh nodejs   # node target -> pkg-node (tests-node/*)
set -euo pipefail
cd "$(dirname "$0")"
target="${1:-web}"
case "$target" in
  web) out=pkg-web ;;
  nodejs) out=pkg-node ;;
  *) echo "usage: build.sh [web|nodejs]" >&2; exit 2 ;;
esac
target_libdir="$(rustc --print target-libdir --target wasm32-unknown-unknown 2>/dev/null || true)"
if [ -z "$target_libdir" ] || [ ! -d "$target_libdir" ]; then
  if command -v rustup >/dev/null 2>&1; then
    rustup_rustc="$(rustup which rustc)"
    rustup_bin="$(dirname "$rustup_rustc")"
    rustup_root="$(cd "$rustup_bin/.." && pwd)"
    export PATH="$rustup_bin:$PATH"
    export DYLD_FALLBACK_LIBRARY_PATH="$rustup_root/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"
  fi
fi
export CARGO_PROFILE_RELEASE_LTO=fat
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
exec wasm-pack build --release --target "$target" --out-dir "$out"
