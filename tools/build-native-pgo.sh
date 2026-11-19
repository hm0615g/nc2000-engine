#!/usr/bin/env bash
# Build machine-specialized native self-play binaries for this host using:
#   - fat LTO + codegen-units=1 + mimalloc (workspace release defaults)
#   - CFLAGS="-O3 -mcpu=native" (native-tuned C build for libmimalloc-sys)
#   - -C target-cpu=apple-m5 (when supported by rustc on arm64 macOS; else native)
#   - -C panic=abort -C llvm-args=-hot-cold-split
#   - 2-stage LLVM CS-PGO (stage-1 PGO + stage-2 context-sensitive PGO)
#
# Output binaries live under: target/native-pgo/release/examples/
# Leaves Cargo.toml, Cargo.lock, .cargo/config.toml, crates/*/src, and target/release untouched.
#
# Usage:
#   tools/build-native-pgo.sh [example ...]
# Optional env vars:
#   REUSE_PGO=1   Skip stages 1-2 and reuse existing target/native-pgo/cspgo.profdata
#   NO_PGO=1      Build with native CPU + LTO + panic=abort only (no llvm-profdata needed)
# Default examples:
#   nash_refresh arena team_eval budget_probe meta_eco adaptive_budget_probe perf_probe omniscient_audit
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [ "$#" -gt 0 ]; then
  EXAMPLES=("$@")
else
  EXAMPLES=(
    nash_refresh
    arena
    team_eval
    budget_probe
    meta_eco
    adaptive_budget_probe
    perf_probe
    omniscient_audit
  )
fi

REUSE_PGO="${REUSE_PGO:-0}"
NO_PGO="${NO_PGO:-0}"

# Pick the best host CPU model known to rustc and matching C compiler flag.
ARCH="$(uname -m)"
TARGET_CPU="${TARGET_CPU:-}"
if [ -z "$TARGET_CPU" ]; then
  if [ "$(uname -s)" = "Darwin" ] && [ "$ARCH" = "arm64" ] && \
     rustc --print target-cpus | grep -qE '^[[:space:]]+apple-m5([[:space:]]|$)'; then
    TARGET_CPU="apple-m5"
  else
    TARGET_CPU="native"
  fi
fi

if [ "$ARCH" = "arm64" ] || [ "$ARCH" = "aarch64" ]; then
  export CFLAGS="${CFLAGS:--O3 -mcpu=native}"
else
  export CFLAGS="${CFLAGS:--O3 -march=native}"
fi
export CARGO_PROFILE_RELEASE_LTO=fat
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1

OUT_ROOT="$ROOT/target/native-pgo"
mkdir -p "$OUT_ROOT"

BASE_FLAGS="-C target-cpu=${TARGET_CPU} -C panic=abort"
EXAMPLE_ARGS=()
for ex in "${EXAMPLES[@]}"; do
  EXAMPLE_ARGS+=(--example "$ex")
done

t0="$(python3 -c 'import time; print(time.time())')"
LLVM_PROFDATA="${LLVM_PROFDATA:-}"

if [ "$NO_PGO" = "1" ]; then
  CARGO_TARGET_DIR="$OUT_ROOT" \
    RUSTFLAGS="${BASE_FLAGS}" \
    cargo build --release -p nc2000-bot "${EXAMPLE_ARGS[@]}" >/dev/null
else
  if [ "$REUSE_PGO" = "1" ] && [ -f "$OUT_ROOT/cspgo.profdata" ]; then
    LLVM_PROFDATA="reused:${OUT_ROOT}/cspgo.profdata"
  else
    # Locate llvm-profdata matching rustc's LLVM major version.
    LLVM_MAJOR="$(rustc -vV | awk '/LLVM version:/ { split($3, a, "."); print a[1] }')"
    if [ -z "$LLVM_PROFDATA" ]; then
      for cand in \
        "/opt/homebrew/opt/llvm@${LLVM_MAJOR}/bin/llvm-profdata" \
        "/usr/local/opt/llvm@${LLVM_MAJOR}/bin/llvm-profdata" \
        "/opt/homebrew/opt/llvm/bin/llvm-profdata" \
        "$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | awk '/host:/ { print $2 }')/bin/llvm-profdata" \
        "$(command -v llvm-profdata 2>/dev/null || true)"; do
        if [ -n "$cand" ] && [ -x "$cand" ]; then
          LLVM_PROFDATA="$cand"
          break
        fi
      done
    fi
    if [ -z "$LLVM_PROFDATA" ] || [ ! -x "$LLVM_PROFDATA" ]; then
      echo "error: llvm-profdata for LLVM ${LLVM_MAJOR} not found; set LLVM_PROFDATA=... or NO_PGO=1" >&2
      exit 1
    fi

    WORK="$OUT_ROOT/pgo-work"
    rm -rf "$WORK"
    mkdir -p "$WORK/pgo1" "$WORK/cspgo"

    # Prepare a compact 2-game full-protocol training schedule for PGO.
    TRAIN_TASKS="$WORK/train-tasks.jsonl"
    head -n 2 "$ROOT/data/nash-camouflage-v1/apex-controls.jsonl" > "$TRAIN_TASKS"

    run_training() {
      local bin_dir="$1"
      local tag="$2"
      "$bin_dir/perf_probe" blind 27000 4 > /dev/null 2>&1
      NASH_POOLS="$ROOT/data/nash-camouflage-v1/screen-pools.json" \
        "$bin_dir/nash_refresh" "$TRAIN_TASKS" "$WORK/out-${tag}" 2 > /dev/null 2>&1
    }

    # Stage 1: instrumented PGO build + profile collection
    CARGO_TARGET_DIR="$WORK/t-pgo1" \
      RUSTFLAGS="${BASE_FLAGS} -C profile-generate=${WORK}/pgo1" \
      cargo build --release -p nc2000-bot --example perf_probe --example nash_refresh >/dev/null

    run_training "$WORK/t-pgo1/release/examples" "pgo1"
    "$LLVM_PROFDATA" merge -o "$WORK/pgo1.profdata" "$WORK/pgo1"

    # Stage 2: context-sensitive PGO instrumentation on top of Stage 1
    CARGO_TARGET_DIR="$WORK/t-cspgo" \
      RUSTFLAGS="${BASE_FLAGS} -C profile-use=${WORK}/pgo1.profdata -C llvm-args=-cs-profile-generate -C llvm-args=-cs-profile-path=${WORK}/cspgo/default_%m_%p.profraw" \
      cargo build --release -p nc2000-bot --example perf_probe --example nash_refresh >/dev/null

    run_training "$WORK/t-cspgo/release/examples" "cspgo"
    "$LLVM_PROFDATA" merge -o "$OUT_ROOT/cspgo.profdata" "$WORK/pgo1.profdata" "$WORK/cspgo"

    # Clean heavy intermediate build dirs, keeping merged profile + final binaries.
    rm -rf "$WORK"
  fi

  # Final build: all target self-play binaries using merged CS-PGO profile
  CARGO_TARGET_DIR="$OUT_ROOT" \
    RUSTFLAGS="${BASE_FLAGS} -C profile-use=${OUT_ROOT}/cspgo.profdata -C llvm-args=-hot-cold-split" \
    cargo build --release -p nc2000-bot "${EXAMPLE_ARGS[@]}" >/dev/null
fi

python3 - "$t0" "$TARGET_CPU" "${LLVM_PROFDATA:-none}" "$OUT_ROOT/release/examples" "${EXAMPLES[@]}" <<'PY'
import json, os, sys, time
t0 = float(sys.argv[1])
cpu, profdata, out_dir = sys.argv[2], sys.argv[3], sys.argv[4]
examples = sys.argv[5:]
missing = [e for e in examples if not os.path.isfile(os.path.join(out_dir, e))]
if missing:
    sys.exit(f"missing built binaries: {missing}")
print(json.dumps({
    "ok": True,
    "target_cpu": cpu,
    "llvm_profdata": profdata,
    "out_dir": out_dir,
    "examples": examples,
    "build_seconds": round(time.time() - t0, 1),
}))
PY
