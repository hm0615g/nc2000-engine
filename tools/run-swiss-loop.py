#!/usr/bin/env python3
"""Fault-tolerant 3-stage Swiss-Cut tournament driver (Local M6 + Colab EPYC).

Stages:
  R1..R6   : All active teams (691) -> parallel across Local M6 + Colab sessions.
             Stops all Colab sessions immediately after R6 (and in finally).
  R7..R12  : Cut to Top 256 -> Local M6 (10 cores).
  R13..R18 : Cut to Top 64  -> Local M6 (10 cores) -> Final report (Top 32/64 + RPS rescue).

Detailed round telemetry is appended to `<dir>/progress.jsonl`; stdout receives
only a single structured JSON summary line at completion.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import shutil
import subprocess
import tarfile
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent


def run_cmd(cmd: list[str], *, env: dict[str, str] | None = None, stdin_text: str | None = None) -> str:
    res = subprocess.run(
        cmd,
        env=env,
        input=stdin_text,
        capture_output=True,
        text=True,
        check=True,
    )
    return res.stdout.strip()


def stop_colab_sessions(sessions: list[str]) -> None:
    for s in sessions:
        subprocess.run(
            ["colab", "--auth", "adc", "stop", "-s", s],
            capture_output=True,
            text=True,
            check=False,
        )


def run_local_shard(
    binary: Path,
    pools: Path,
    shard_file: Path,
    out_dir: Path,
    workers: int,
    preview_iters: int,
    battle_iters: int,
) -> dict[str, Any]:
    if out_dir.exists():
        shutil.rmtree(out_dir)
    env = os.environ.copy()
    env["NASH_POOLS"] = str(pools)
    env["NASH_PREVIEW_ITERS"] = str(preview_iters)
    env["NASH_BATTLE_ITERS"] = str(battle_iters)
    env["NASH_BATTLE_MODE"] = "fixed"
    env["NASH_PREVIEW_CACHE"] = "1"
    out = run_cmd([str(binary), str(shard_file), str(out_dir), str(workers)], env=env)
    return json.loads(out.splitlines()[-1])


def run_colab_shard(
    session: str,
    shard_file: Path,
    out_dir: Path,
    round_num: int,
    preview_iters: int,
    battle_iters: int,
    colab_workers: int,
) -> dict[str, Any]:
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    remote_sched = f"/tmp/swiss-r{round_num}-{session}.jsonl"
    remote_out = f"/tmp/swiss-out-r{round_num}-{session}"
    remote_tar = f"/tmp/swiss-out-r{round_num}-{session}.tar.gz"
    local_tar = out_dir.parent / f"colab-r{round_num}-{session}.tar.gz"

    run_cmd(["colab", "--auth", "adc", "upload", "-s", session, str(shard_file), remote_sched])

    script = f"""
import json, subprocess, time
t0 = time.time()
cmd = r'''
set -euo pipefail
rm -rf {remote_out} {remote_tar}
NASH_POOLS=/tmp/pools.json NASH_PREVIEW_ITERS={preview_iters} NASH_BATTLE_ITERS={battle_iters} NASH_BATTLE_MODE=fixed NASH_PREVIEW_CACHE=1 \\
  /content/nc2000/target/native-pgo/release/examples/nash_refresh {remote_sched} {remote_out} {colab_workers} > /tmp/shard-run.json
tar -czf {remote_tar} -C {remote_out} .
'''
subprocess.run(["bash", "-c", cmd], check=True)
meta = json.loads(open("/tmp/shard-run.json").read().strip())
meta["session"] = {session!r}
meta["total_s"] = round(time.time() - t0, 1)
print(json.dumps(meta))
"""
    stdout = run_cmd(
        ["colab", "--auth", "adc", "exec", "-s", session, "--timeout", "1800"],
        stdin_text=script,
    )
    run_cmd(["colab", "--auth", "adc", "download", "-s", session, remote_tar, str(local_tar)])
    with tarfile.open(local_tar, "r:gz") as tf:
        tf.extractall(out_dir)
    local_tar.unlink(missing_ok=True)

    # Parse last JSON line from colab stdout
    for line in reversed(stdout.splitlines()):
        line = line.strip()
        if line.startswith("{") and line.endswith("}"):
            return json.loads(line)
    return {"session": session, "games": sum(1 for _ in shard_file.open())}


def split_pairs(sched_path: Path, fracs: list[float], out_files: list[Path]) -> list[int]:
    """Split schedule into pair-preserving chunks (always even line counts)."""
    lines = [l.strip() for l in sched_path.open() if l.strip()]
    assert len(lines) % 2 == 0
    n_pairs = len(lines) // 2
    total_f = sum(fracs)
    pair_counts = []
    used = 0
    for i, f in enumerate(fracs):
        if i == len(fracs) - 1:
            c = n_pairs - used
        else:
            c = int(round(n_pairs * f / total_f))
            c = max(0, min(c, n_pairs - used))
        pair_counts.append(c)
        used += c
    cursor = 0
    game_counts = []
    for c, path in zip(pair_counts, out_files):
        chunk = lines[cursor : cursor + 2 * c]
        cursor += 2 * c
        path.write_text("\n".join(chunk) + ("\n" if chunk else ""))
        game_counts.append(len(chunk))
    return game_counts


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", type=Path, required=True)
    ap.add_argument("--binary", type=Path, default=ROOT / "target/native-pgo/release/examples/nash_refresh")
    ap.add_argument("--local-workers", type=int, default=10)
    ap.add_argument("--colab-sessions", nargs="*", default=[])
    ap.add_argument("--colab-workers", type=int, default=8)
    ap.add_argument("--local-weight", type=float, default=0.58)
    ap.add_argument("--colab-weight", type=float, default=0.21)
    ap.add_argument("--preview-iters", type=int, default=81000)
    ap.add_argument("--battle-iters", type=int, default=9000)
    ap.add_argument("--start-round", type=int, default=1)
    ap.add_argument("--end-round", type=int, default=18)
    ap.add_argument("--cut-r7", type=int, default=256)
    ap.add_argument("--cut-r13", type=int, default=64)
    ap.add_argument("--colab-until-round", type=int, default=6)
    args = ap.parse_args()

    pools = args.dir / "pools.json"
    progress_log = args.dir / "progress.jsonl"
    active_colab = list(args.colab_sessions)
    t_all = time.monotonic()

    try:
        for r in range(args.start_round, args.end_round + 1):
            t_r = time.monotonic()
            rdir = args.dir / f"round-{r}"
            sched = rdir / "schedule.jsonl"
            if not sched.exists():
                cut = args.cut_r7 if r == 7 else (args.cut_r13 if r == 13 else 0)
                cmd = ["python3", str(ROOT / "tools/nash-swiss.py"), "pair", "--dir", str(args.dir), "--round", str(r)]
                if cut:
                    cmd += ["--cut", str(cut)]
                run_cmd(cmd)

            use_colab = active_colab if r <= args.colab_until_round else []
            shard_dirs: list[Path] = []
            worker_metas: list[dict[str, Any]] = []

            if not use_colab:
                out_local = rdir / "out-local"
                m = run_local_shard(
                    args.binary, pools, sched, out_local,
                    args.local_workers, args.preview_iters, args.battle_iters,
                )
                shard_dirs.append(out_local)
                worker_metas.append({"worker": "local", **m})
            else:
                fracs = [args.local_weight] + [args.colab_weight] * len(use_colab)
                shard_files = [rdir / "shard-local.jsonl"] + [rdir / f"shard-{s}.jsonl" for s in use_colab]
                counts = split_pairs(sched, fracs, shard_files)

                futs: dict[concurrent.futures.Future[Any], tuple[str, Path, Path]] = {}
                with concurrent.futures.ThreadPoolExecutor(max_workers=1 + len(use_colab)) as ex:
                    if counts[0] > 0:
                        out_local = rdir / "out-local"
                        fut = ex.submit(
                            run_local_shard,
                            args.binary, pools, shard_files[0], out_local,
                            args.local_workers, args.preview_iters, args.battle_iters,
                        )
                        futs[fut] = ("local", shard_files[0], out_local)
                    for idx, sess in enumerate(use_colab, 1):
                        if counts[idx] > 0:
                            out_c = rdir / f"out-{sess}"
                            fut = ex.submit(
                                run_colab_shard,
                                sess, shard_files[idx], out_c, r,
                                args.preview_iters, args.battle_iters, args.colab_workers,
                            )
                            futs[fut] = (sess, shard_files[idx], out_c)

                    failed_shards: list[tuple[str, Path, Path]] = []
                    for fut in concurrent.futures.as_completed(futs):
                        name, sf, od = futs[fut]
                        try:
                            m = fut.result()
                            shard_dirs.append(od)
                            worker_metas.append({"worker": name, **m})
                        except Exception as e:
                            worker_metas.append({"worker": name, "error": str(e)})
                            failed_shards.append((name, sf, od))
                            if name in active_colab:
                                active_colab.remove(name)

                # Graceful local fallback for any failed Colab shard
                for name, sf, od in failed_shards:
                    od_fb = rdir / f"out-fallback-{name}"
                    m = run_local_shard(
                        args.binary, pools, sf, od_fb,
                        args.local_workers, args.preview_iters, args.battle_iters,
                    )
                    shard_dirs.append(od_fb)
                    worker_metas.append({"worker": f"fallback-{name}", **m})

            ing_out = run_cmd([
                "python3", str(ROOT / "tools/nash-swiss.py"), "ingest",
                "--dir", str(args.dir), "--round", str(r),
                *[str(d) for d in shard_dirs],
            ])
            ing = json.loads(ing_out.splitlines()[-1])

            rec = {
                "round": r,
                "wall_s": round(time.monotonic() - t_r, 1),
                "workers": worker_metas,
                "ingest": ing,
            }
            with progress_log.open("a") as fh:
                fh.write(json.dumps(rec) + "\n")

            # Stop Colab VMs right after colab_until_round finishes
            if r == args.colab_until_round and args.colab_sessions:
                stop_colab_sessions(args.colab_sessions)
                active_colab.clear()

        rep_out = run_cmd([
            "python3", str(ROOT / "tools/nash-swiss.py"), "report",
            "--dir", str(args.dir), "--top", "64",
        ])
        rep = json.loads(rep_out.splitlines()[-1])
        print(json.dumps({
            "ok": True,
            "total_wall_s": round(time.monotonic() - t_all, 1),
            "report": rep,
        }))
    finally:
        if args.colab_sessions:
            stop_colab_sessions(args.colab_sessions)


if __name__ == "__main__":
    main()
