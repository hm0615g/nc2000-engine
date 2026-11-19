#!/usr/bin/env bash
# Build native x86_64/arm64 `nash_refresh` (fast NO_PGO default on Colab, or PGO
# with USE_PGO=1) and run a single schedule shard.
#
# Usage:
#   NASH_POOLS=path/to/pools.json tools/run-colab-shard.sh SHARD.jsonl OUT_DIR [workers]
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo "usage: NASH_POOLS=FILE $0 SHARD.jsonl OUT_DIR [workers]" >&2
  exit 2
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SHARD="$1"
OUT_DIR="$2"
WORKERS="${3:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
BIN="$ROOT/target/native-pgo/release/examples/nash_refresh"

if [ ! -x "$BIN" ]; then
  if [ "${USE_PGO:-0}" = "1" ]; then
    "$ROOT/tools/build-native-pgo.sh" nash_refresh >/dev/null
  else
    NO_PGO=1 "$ROOT/tools/build-native-pgo.sh" nash_refresh >/dev/null
  fi
fi

exec "$BIN" "$SHARD" "$OUT_DIR" "$WORKERS"
