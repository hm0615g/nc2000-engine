#!/usr/bin/env bash
# Cross-compile an optimized Linux x86_64 binary (`mimalloc` + fat LTO + x86-64-v3 + glibc 2.35)
# 100% LOCALLY on macOS arm64 using `cargo-zigbuild` (zero Colab VM, zero CU spend).
#
# Output binary lives at: target/colab-x86_64/<example>
# Compatible with all Colab tiers (G4 Zen 5, V5E1 Zen 3, L4 Xeon).
#
# Usage:
#   tools/build-colab-x86.sh [example]
# Optional env vars:
#   TARGET_CPU=x86-64-v3   (default; or znver5 if policy.json restricts to G4-only)
#   GLIBC_VER=2.35         (default; Ubuntu 22.04+ / Debian 12+ / all Colab VMs)
#   LTO=fat                (default; set LTO=thin for faster dev iterations)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXAMPLE="${1:-nash_refresh}"
OUT_DIR="${OUT_DIR:-$ROOT/target/colab-x86_64}"
TARGET_CPU="${TARGET_CPU:-x86-64-v3}"
GLIBC_VER="${GLIBC_VER:-2.35}"
LTO="${LTO:-fat}"
CGU="${CODEGEN_UNITS:-$([ "$LTO" = "fat" ] && echo 1 || echo 16)}"

RUSTUP_BIN="$(dirname "$(rustup which cargo)")"
export PATH="$RUSTUP_BIN:/opt/homebrew/bin:$PATH"

command -v cargo-zigbuild >/dev/null 2>&1 || { echo '{"ok":false,"error":"cargo-zigbuild not installed; brew install zig cargo-zigbuild"}' >&2; exit 2; }
command -v zig >/dev/null 2>&1 || { echo '{"ok":false,"error":"zig not installed; brew install zig"}' >&2; exit 2; }

if ! rustup target list --installed | grep -qx "x86_64-unknown-linux-gnu"; then
  rustup target add x86_64-unknown-linux-gnu >/dev/null
fi

mkdir -p "$OUT_DIR"
t0="$(python3 -c 'import time; print(time.time())')"

cd "$ROOT"
CARGO_TARGET_DIR="$ROOT/target/colab-x86_64-zig" \
  CARGO_PROFILE_RELEASE_LTO="$LTO" \
  CARGO_PROFILE_RELEASE_CODEGEN_UNITS="$CGU" \
  CFLAGS="-O3 -march=${TARGET_CPU}" \
  RUSTFLAGS="-C target-cpu=${TARGET_CPU} -C panic=abort" \
  cargo zigbuild --release \
    --target "x86_64-unknown-linux-gnu.${GLIBC_VER}" \
    -p nc2000-bot --example "$EXAMPLE" >/dev/null 2>&1

BUILT="$ROOT/target/colab-x86_64-zig/x86_64-unknown-linux-gnu/release/examples/$EXAMPLE"
cp -f "$BUILT" "$OUT_DIR/$EXAMPLE.tmp"
chmod 0755 "$OUT_DIR/$EXAMPLE.tmp"
mv -f "$OUT_DIR/$EXAMPLE.tmp" "$OUT_DIR/$EXAMPLE"

python3 - "$t0" "$OUT_DIR/$EXAMPLE" "$TARGET_CPU" "$GLIBC_VER" "$LTO" <<'PY'
import hashlib, json, os, sys, time
t0, path, cpu, glibc, lto = float(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]
data = open(path, "rb").read()
print(json.dumps({
    "ok": True,
    "binary": path,
    "bytes": len(data),
    "sha256": hashlib.sha256(data).hexdigest(),
    "target": f"x86_64-unknown-linux-gnu.{glibc}",
    "target_cpu": cpu,
    "lto": lto,
    "elapsed_s": round(time.time() - t0, 2),
}, separators=(",", ":")))
PY
