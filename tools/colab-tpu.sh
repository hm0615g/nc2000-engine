#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COLAB_HOME="${NC2000_COLAB_HOME:-$REPO_ROOT/tmp/colab-home}"
mkdir -p "$COLAB_HOME/.config/colab-cli"

if [[ ! -f "$COLAB_HOME/.config/colab-cli/settings.json" ]]; then
  printf '{"enable_update_check": false}\n' > "$COLAB_HOME/.config/colab-cli/settings.json"
fi

if [[ -z "${GOOGLE_APPLICATION_CREDENTIALS:-}" ]]; then
  REAL_HOME="${HOME:-}"
  if [[ -n "$REAL_HOME" && -f "$REAL_HOME/.config/gcloud/application_default_credentials.json" ]]; then
    export GOOGLE_APPLICATION_CREDENTIALS="$REAL_HOME/.config/gcloud/application_default_credentials.json"
  fi
fi

COLAB_BIN="${COLAB_BIN:-$(command -v colab || echo "$HOME/.local/bin/colab")}"
CONFIG_FILE="$COLAB_HOME/.config/colab-cli/sessions.json"
LOG_FILE="$COLAB_HOME/colab.log"
BUNDLE_PATH="$COLAB_HOME/gen2_tpu_bundle.tar.gz"

run_colab() {
  HOME="$COLAB_HOME" "$COLAB_BIN" --auth="${NC2000_COLAB_AUTH:-adc}" --config "$CONFIG_FILE" "$@"
}

cmd="${1:-help}"
shift || true

case "$cmd" in
  bundle)
    out="${1:-$BUNDLE_PATH}"
    uv run --project "$REPO_ROOT/tpu" gen2-tpu bundle-colab --out "$out" >>"$LOG_FILE" 2>&1
    python3 -c 'import json, os, sys; p=sys.argv[1]; print(json.dumps({"cmd":"bundle","ok":True,"bundle":p,"bytes":os.path.getsize(p)}))' "$out"
    ;;
  sessions)
    t0="$(python3 -c 'import time; print(time.perf_counter())')"
    set +e
    run_colab sessions >"$LOG_FILE" 2>&1
    rc=$?
    set -e
    python3 - "$rc" "$t0" "$LOG_FILE" <<'PY'
import json, sys, time
rc = int(sys.argv[1])
dt = round(time.perf_counter() - float(sys.argv[2]), 3)
lines = [ln.strip() for ln in open(sys.argv[3], encoding="utf-8", errors="replace").read().splitlines() if ln.strip() and not ln.startswith("[colab]")]
print(json.dumps({"cmd": "sessions", "ok": rc == 0, "rc": rc, "sec": dt, "count": len(lines), "log": sys.argv[3]}))
sys.exit(rc)
PY
    ;;
  pytest)
    uv run --project "$REPO_ROOT/tpu" gen2-tpu bundle-colab --out "$BUNDLE_PATH" >>"$LOG_FILE" 2>&1
    RUNNER_PY="$COLAB_HOME/remote_pytest_runner.py"
    python3 - "$BUNDLE_PATH" "$RUNNER_PY" <<'PY'
import base64, pathlib, sys
b64 = base64.b64encode(pathlib.Path(sys.argv[1]).read_bytes()).decode("ascii")
script = f'''import base64, io, json, subprocess, sys, tarfile, tempfile, time
t0 = time.perf_counter()
raw = base64.b64decode({b64!r})
with tempfile.TemporaryDirectory(prefix="nc2000_tpu_") as d:
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as tf:
        try:
            tf.extractall(d, filter="data")
        except TypeError:
            tf.extractall(d)
    import jax
    proc = subprocess.run(
        [sys.executable, "-m", "pytest", f"{{d}}/tests", "-q", *sys.argv[1:]],
        env={{**dict(__import__("os").environ), "PYTHONPATH": f"{{d}}/src"}},
        capture_output=True,
        text=True,
    )
    dt = round(time.perf_counter() - t0, 3)
    tail = (proc.stdout.strip().splitlines() or [""])[-1]
    print("NC2000_COLAB_JSON:" + json.dumps({{
        "ok": proc.returncode == 0,
        "rc": proc.returncode,
        "remote_sec": dt,
        "backend": jax.default_backend(),
        "devices": len(jax.devices()),
        "summary": tail,
    }}))
    if proc.returncode != 0:
        sys.stderr.write(proc.stdout + "\\n" + proc.stderr)
        sys.exit(proc.returncode)
'''
pathlib.Path(sys.argv[2]).write_text(script, encoding="utf-8")
PY
    accel_args=()
    if [[ -n "${COLAB_ACCEL:-}" ]]; then
      # shellcheck disable=SC2206
      accel_args=($COLAB_ACCEL)
    fi
    timeout_sec="${COLAB_TIMEOUT:-600}"
    t0="$(python3 -c 'import time; print(time.perf_counter())')"
    set +e
    if (( ${#accel_args[@]} > 0 )); then
      run_colab run "${accel_args[@]}" --timeout "$timeout_sec" "$RUNNER_PY" "$@" >"$LOG_FILE" 2>&1
    else
      run_colab run --timeout "$timeout_sec" "$RUNNER_PY" "$@" >"$LOG_FILE" 2>&1
    fi
    rc=$?
    set -e
    python3 - "$rc" "$t0" "$LOG_FILE" <<'PY'
import json, sys, time
rc = int(sys.argv[1])
wall = round(time.perf_counter() - float(sys.argv[2]), 3)
text = open(sys.argv[3], encoding="utf-8", errors="replace").read()
remote = None
for line in text.splitlines():
    if "NC2000_COLAB_JSON:" in line:
        remote = json.loads(line.split("NC2000_COLAB_JSON:", 1)[1].strip())
out = {"cmd": "pytest", "ok": rc == 0 and (remote is None or remote.get("ok", False)), "rc": rc, "wall_sec": wall, "remote": remote, "log": sys.argv[3]}
print(json.dumps(out))
sys.exit(0 if out["ok"] else (rc or 1))
PY
    ;;
  raw)
    run_colab "$@"
    ;;
  *)
    cat <<'EOF'
Usage: ./tools/colab-tpu.sh <subcommand> [args...]

Subcommands:
  bundle [out.tar.gz]   Build self-contained gen2-tpu bundle (includes dex & learnsets)
  sessions              List active Colab sessions via headless ADC (logs in tmp/colab-home/colab.log)
  pytest [pytest-args]  Run gen2-tpu pytest on an ephemeral Colab VM (set COLAB_ACCEL="--tpu v6e1" or "--gpu T4" if needed)
  raw <colab-args...>   Invoke `colab --auth=adc` directly with repo-local session/config state
EOF
    ;;
esac
