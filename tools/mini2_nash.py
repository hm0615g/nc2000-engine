#!/usr/bin/env python3
"""Project-specific `mini2-barn` dispatch helper for `nc2000-engine` `nash_refresh`.

Runs individual schedule chunks on subordinate node `mini2` via `mini2-barn run`
(hash-verified content-addressed blobs, ephemeral run workspace, atomic collect),
or drives a work-stealing dual-node queue across `mini1` (local) + `mini2`.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import secrets
import shutil
import subprocess
import threading
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_BIN = ROOT / "target/native-pgo/release/examples/nash_refresh"

# Exact data files read by `crates/bot/examples/nash_refresh.rs` when `mixtures` are present.
NASH_REFRESH_DATA_FILES = [
    "data/gen2stadium2.json",
    "data/learnsets-gen2.json",
    "data/live-search.json",
    "data/search-profiles.json",
    "data/kodoku-beq-v1/novel_evolved_teams.json",
    "data/team-pool-v2/team-pool.json",
    "data/kodoku-r3/conclusion-equilibrium-deme.json",
    "data/nash-refresh-v1/candidate.json",
]


def build_mini2_run_cmd(
    *,
    binary: Path,
    pools_path: Path | None,
    schedule_path: Path,
    out_dir: Path,
    workers: int = 11,
    run_id: str | None = None,
    preview_iters: int | None = None,
    battle_iters: int | None = None,
    battle_mode: str | None = None,
    preview_cache: bool = True,
    preview_dist_cache: Path | None = None,
    preview_dist_k: int | None = None,
    preview_dist_only: bool = False,
    timeout: int | None = None,
) -> list[str]:
    cmd: list[str] = [
        "mini2-barn",
        "run",
        "--run-id",
        run_id or f"nc2000-{int(time.time())}-{secrets.token_hex(3)}",
        "--bin",
        f"bin/nash_refresh={binary.resolve()}",
    ]
    for rel in NASH_REFRESH_DATA_FILES:
        cmd.extend(["--file", f"{rel}={(ROOT / rel).resolve()}"])
    if pools_path is not None:
        cmd.extend(["--file", f"pools.json={pools_path.resolve()}"])
    has_dist_file = preview_dist_cache is not None and preview_dist_cache.exists()
    if has_dist_file:
        cmd.extend(["--file", f"preview-dist.json={preview_dist_cache.resolve()}"])
    cmd.extend([
        "--input",
        f"schedule.jsonl={schedule_path.resolve()}",
        "--env",
        "NC2000_ROOT={run}",
        "--env",
        f"NASH_PREVIEW_CACHE={'1' if preview_cache else '0'}",
    ])
    if pools_path is not None:
        cmd.extend(["--env", "NASH_POOLS={run}/pools.json"])
    if has_dist_file:
        cmd.extend(["--env", "NASH_PREVIEW_DIST_CACHE={run}/preview-dist.json"])
    if preview_dist_k is not None:
        cmd.extend(["--env", f"NASH_PREVIEW_DIST_K={preview_dist_k}"])
    if preview_dist_only:
        cmd.extend(["--env", "NASH_PREVIEW_DIST_ONLY=1"])
    if preview_iters is not None:
        cmd.extend(["--env", f"NASH_PREVIEW_ITERS={preview_iters}"])
    if battle_iters is not None:
        cmd.extend(["--env", f"NASH_BATTLE_ITERS={battle_iters}"])
    if battle_mode is not None:
        cmd.extend(["--env", f"NASH_BATTLE_MODE={battle_mode}"])
    if timeout is not None:
        cmd.extend(["--timeout", str(timeout)])
    cmd.extend([
        "--collect",
        f"out={out_dir.resolve()}",
        "--",
        "{run}/bin/nash_refresh",
        "{run}/schedule.jsonl",
        "{run}/out",
        str(workers),
    ])
    return cmd


def run_chunk_mini2(
    *,
    binary: Path,
    pools_path: Path | None,
    schedule_path: Path,
    final_out_dir: Path,
    done_marker: Path,
    workers: int = 11,
    run_id: str | None = None,
    preview_iters: int | None = None,
    battle_iters: int | None = None,
    battle_mode: str | None = None,
    preview_cache: bool = True,
    preview_dist_cache: Path | None = None,
    preview_dist_k: int | None = None,
    preview_dist_only: bool = False,
    timeout: int | None = None,
) -> dict[str, Any]:
    """Execute one chunk on `mini2` and atomically commit `final_out_dir` + `done_marker`."""
    stage_out = final_out_dir.parent / f".{final_out_dir.name}.mini2-{secrets.token_hex(3)}"
    if stage_out.exists():
        shutil.rmtree(stage_out)
    cmd = build_mini2_run_cmd(
        binary=binary,
        pools_path=pools_path,
        schedule_path=schedule_path,
        out_dir=stage_out,
        workers=workers,
        run_id=run_id,
        preview_iters=preview_iters,
        battle_iters=battle_iters,
        battle_mode=battle_mode,
        preview_cache=preview_cache,
        preview_dist_cache=preview_dist_cache,
        preview_dist_k=preview_dist_k,
        preview_dist_only=preview_dist_only,
        timeout=timeout,
    )
    proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        shutil.rmtree(stage_out, ignore_errors=True)
        raise RuntimeError(
            f"mini2-barn run failed (rc={proc.returncode}):\n{proc.stderr or proc.stdout}"
        )
    telemetry = json.loads(proc.stdout.strip().splitlines()[-1])
    if not telemetry.get("ok"):
        shutil.rmtree(stage_out, ignore_errors=True)
        raise RuntimeError(f"mini2-barn remote command failed: {telemetry}")

    if final_out_dir.exists():
        shutil.rmtree(final_out_dir)
    stage_out.rename(final_out_dir)
    done_marker.write_text(json.dumps({
        "host": "mini2",
        "remote_stdout_last": telemetry.get("remote_stdout_last"),
        "remote_wall_ms": telemetry.get("remote_wall_ms"),
        "total_ms": telemetry.get("total_ms"),
    }) + "\n")
    return telemetry


def run_chunk_local(
    *,
    binary: Path,
    pools_path: Path | None,
    schedule_path: Path,
    final_out_dir: Path,
    done_marker: Path,
    workers: int = 10,
    preview_iters: int | None = None,
    battle_iters: int | None = None,
    battle_mode: str | None = None,
    preview_cache: bool = True,
    preview_dist_cache: Path | None = None,
    preview_dist_k: int | None = None,
    preview_dist_only: bool = False,
) -> dict[str, Any]:
    """Execute one chunk locally on `mini1`."""
    env = os.environ.copy()
    if pools_path is not None:
        env["NASH_POOLS"] = str(pools_path.resolve())
    env["NASH_PREVIEW_CACHE"] = "1" if preview_cache else "0"
    if preview_dist_cache is not None and preview_dist_cache.exists():
        env["NASH_PREVIEW_DIST_CACHE"] = str(preview_dist_cache.resolve())
    else:
        env.pop("NASH_PREVIEW_DIST_CACHE", None)
    if preview_dist_k is not None:
        env["NASH_PREVIEW_DIST_K"] = str(preview_dist_k)
    else:
        env.pop("NASH_PREVIEW_DIST_K", None)
    if preview_dist_only:
        env["NASH_PREVIEW_DIST_ONLY"] = "1"
    else:
        env.pop("NASH_PREVIEW_DIST_ONLY", None)
    if preview_iters is not None:
        env["NASH_PREVIEW_ITERS"] = str(preview_iters)
    if battle_iters is not None:
        env["NASH_BATTLE_ITERS"] = str(battle_iters)
    if battle_mode is not None:
        env["NASH_BATTLE_MODE"] = battle_mode

    if final_out_dir.exists():
        shutil.rmtree(final_out_dir)
    if done_marker.exists():
        done_marker.unlink()

    t0 = time.monotonic()
    proc = subprocess.run(
        [str(binary), str(schedule_path), str(final_out_dir), str(workers)],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"local nash_refresh failed (rc={proc.returncode}):\n{proc.stderr or proc.stdout}")
    wall_ms = round((time.monotonic() - t0) * 1000)
    stdout_last = proc.stdout.strip().splitlines()[-1] if proc.stdout.strip() else ""
    done_marker.write_text(stdout_last + "\n")
    return {"host": "mini1", "wall_ms": wall_ms, "stdout_last": stdout_last}


def run_chunks_dual_node(
    chunks: list[dict[str, Any]],
    *,
    binary: Path,
    pools_path: Path | None,
    local_workers: int = 10,
    mini2_workers: int = 11,
    use_mini2: bool = True,
    preview_iters: int | None = None,
    battle_iters: int | None = None,
    battle_mode: str | None = None,
    preview_cache: bool = True,
    preview_dist_cache: Path | None = None,
    preview_dist_k: int | None = None,
    preview_dist_only: bool = False,
) -> dict[str, Any]:
    """Dynamic work-stealing execution of chunk list across `mini1` (+ `mini2` when enabled).

    Each element of `chunks` has: `ci`, `sched` (Path), `out` (Path), `done` (Path), `n_rows` (int).
    """
    pending: list[dict[str, Any]] = []
    skipped = 0
    for ch in chunks:
        if ch["done"].exists() and ch["out"].exists():
            if preview_dist_only:
                if (ch["out"] / "preview-dist.json").exists():
                    skipped += 1
                    continue
            else:
                existing = [p for p in ch["out"].glob("*.json") if p.name not in ("manifest.json", "preview-dist-new.json")]
                if len(existing) == ch["n_rows"]:
                    skipped += 1
                    continue
        pending.append(ch)

    if not pending:
        return {"ran_local": 0, "ran_mini2": 0, "skipped": skipped}

    lock = threading.Lock()
    cursor = [0]
    counts = {"mini1": 0, "mini2": 0}

    def next_chunk() -> dict[str, Any] | None:
        with lock:
            if cursor[0] >= len(pending):
                return None
            ch = pending[cursor[0]]
            cursor[0] += 1
            return ch

    def worker_loop(node: str) -> None:
        while True:
            ch = next_chunk()
            if ch is None:
                return
            if node == "mini2":
                run_chunk_mini2(
                    binary=binary,
                    pools_path=pools_path,
                    schedule_path=ch["sched"],
                    final_out_dir=ch["out"],
                    done_marker=ch["done"],
                    workers=mini2_workers,
                    run_id=f"nc2000-c{ch['ci']:03d}-{secrets.token_hex(3)}",
                    preview_iters=preview_iters,
                    battle_iters=battle_iters,
                    battle_mode=battle_mode,
                    preview_cache=preview_cache,
                    preview_dist_cache=preview_dist_cache,
                    preview_dist_k=preview_dist_k,
                    preview_dist_only=preview_dist_only,
                )
            else:
                run_chunk_local(
                    binary=binary,
                    pools_path=pools_path,
                    schedule_path=ch["sched"],
                    final_out_dir=ch["out"],
                    done_marker=ch["done"],
                    workers=local_workers,
                    preview_iters=preview_iters,
                    battle_iters=battle_iters,
                    battle_mode=battle_mode,
                    preview_cache=preview_cache,
                    preview_dist_cache=preview_dist_cache,
                    preview_dist_k=preview_dist_k,
                    preview_dist_only=preview_dist_only,
                )
            with lock:
                counts[node] += 1

    nodes = ["mini1", "mini2"] if (use_mini2 and len(pending) >= 2) else ["mini1"]
    with concurrent.futures.ThreadPoolExecutor(max_workers=len(nodes)) as pool:
        futs = [pool.submit(worker_loop, n) for n in nodes]
        for fut in futs:
            fut.result()

    return {"ran_local": counts["mini1"], "ran_mini2": counts["mini2"], "skipped": skipped}


def main() -> None:
    """Standalone CLI to offload specific chunk indices of a prepared RR dir to `mini2`."""
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", type=Path, required=True, help="Prepared RR dir with meta.json & schedule.jsonl")
    ap.add_argument("--chunks", type=str, required=True, help="Comma-separated chunk indices, e.g. 6,5,4")
    ap.add_argument("--chunk-games", type=int, default=40)
    ap.add_argument("--workers", type=int, default=11)
    ap.add_argument("--binary", type=Path, default=DEFAULT_BIN)
    ap.add_argument("--preview-iters", type=int, default=None)
    ap.add_argument("--battle-iters", type=int, default=None)
    ap.add_argument("--battle-mode", choices=["adaptive", "fixed"], default=None)
    args = ap.parse_args()

    meta = json.loads((args.dir / "meta.json").read_text())
    pools_path = Path(meta["pools"])
    rows = [l.strip() for l in (args.dir / "schedule.jsonl").open() if l.strip()]
    chunk_size = max(2, args.chunk_games + (args.chunk_games % 2))
    chunks_dir = args.dir / "chunks"
    chunks_dir.mkdir(parents=True, exist_ok=True)

    indices = [int(x.strip()) for x in args.chunks.split(",") if x.strip()]
    t0 = time.monotonic()
    ran = []
    skipped = []
    for ci in indices:
        c_rows = rows[ci * chunk_size : (ci + 1) * chunk_size]
        if not c_rows:
            continue
        c_sched = chunks_dir / f"chunk-{ci:03d}.jsonl"
        c_out = chunks_dir / f"chunk-{ci:03d}-out"
        c_done = chunks_dir / f"chunk-{ci:03d}.done.json"
        # Skip if already done OR already actively being worked on by mini1
        if c_done.exists() or c_out.exists():
            skipped.append(ci)
            continue
        c_sched.write_text("\n".join(c_rows) + "\n")
        run_chunk_mini2(
            binary=args.binary,
            pools_path=pools_path,
            schedule_path=c_sched,
            final_out_dir=c_out,
            done_marker=c_done,
            workers=args.workers,
            run_id=f"nc2000-p0-c{ci:03d}-{secrets.token_hex(3)}",
            preview_iters=args.preview_iters,
            battle_iters=args.battle_iters,
            battle_mode=args.battle_mode,
        )
        ran.append(ci)

    print(json.dumps({
        "host": "mini2",
        "dir": str(args.dir),
        "ran_chunks": ran,
        "skipped_chunks": skipped,
        "elapsed_s": round(time.monotonic() - t0, 2),
    }))


if __name__ == "__main__":
    main()
