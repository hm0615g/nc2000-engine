"""Information-Constrained Oracle (ICO), finite perfect-recall game solver.

The input is a sequence-form game, NOT a fully observed battle state. An
information set has one policy shared by every compatible hidden history.
JAX CFR+ supplies strategies; independent float64 best responses measure the
remaining saddle-point gap. Sampled chance and heuristic frontier values are
explicit approximations, not certificates for the full battle game.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import time
from pathlib import Path

import numpy as np


def validate(game: dict) -> None:
    if game.get("format") != "nc2000-ico-sequence-v1":
        raise ValueError("unsupported ICO game format")
    if len(game["players"]) != 2:
        raise ValueError("ICO requires two players")
    for player in game["players"]:
        n = player["sequence_count"]
        seen, keys = {0}, set()
        for info in player["infosets"]:
            if info["key"] in keys:
                raise ValueError("duplicate information key: policies must be shared")
            keys.add(info["key"])
            if info["parent"] not in seen:
                raise ValueError("parent must precede child (perfect recall)")
            seq = info["sequences"]
            if not seq or len(seq) != len(info["actions"]):
                raise ValueError("invalid information-set actions")
            if len(set(info["actions"])) != len(seq):
                raise ValueError("duplicate action")
            for s in seq:
                if s in seen or not 0 < s < n:
                    raise ValueError("sequence must have exactly one information-set owner")
                seen.add(s)
            if "baseline" in info:
                p = np.asarray(info["baseline"], dtype=float)
                if p.shape != (len(seq),) or not np.isfinite(p).all() or (p < 0).any() or abs(p.sum()-1) > 1e-9:
                    raise ValueError("invalid baseline distribution")
        if seen != set(range(n)):
            raise ValueError("orphan sequence")
    for s0, s1, mass, estimate, lo, hi in game["payoffs"]:
        if not (0 <= s0 < game["players"][0]["sequence_count"] and
                0 <= s1 < game["players"][1]["sequence_count"]):
            raise ValueError("invalid payoff sequence")
        if not np.isfinite([mass, estimate, lo, hi]).all() or mass < 0 or not 0 <= lo <= estimate <= hi <= 1:
            raise ValueError("invalid payoff interval")
    # Completeness check independent of rewards. A truncated chance tree or
    # omitted legal branch must not masquerade as a normalized finite game.
    aa = [arrays(p) for p in game["players"]]
    rng = np.random.default_rng(17011)
    for _ in range(3):
        policies = []
        for a in aa:
            p = (rng.random(a["mask"].shape)+.1)*a["mask"]
            policies.append(p/np.maximum(p.sum(axis=1,keepdims=True),1e-30))
        r0,r1 = [realization(a,p) for a,p in zip(aa,policies)]
        mass = sum(row[2]*r0[row[0]]*r1[row[1]] for row in game["payoffs"])
        if abs(mass-1.) > 1e-8:
            raise ValueError(f"incomplete or unnormalized game: terminal mass={mass}")


def arrays(player: dict) -> dict:
    infos = player["infosets"]
    width = max((len(i["sequences"]) for i in infos), default=1)
    # One inert dummy row supports players with no decisions.
    seq = np.zeros((max(1, len(infos)), width), np.int32)
    mask = np.zeros_like(seq, dtype=bool)
    parent = np.zeros(len(seq), np.int32)
    depth_seq = np.zeros(player["sequence_count"], np.int32)
    depths = np.zeros(len(seq), np.int32)
    for j, info in enumerate(infos):
        k = len(info["sequences"])
        seq[j, :k], mask[j, :k], parent[j] = info["sequences"], True, info["parent"]
        depths[j] = depth_seq[parent[j]]
        depth_seq[seq[j, :k]] = depths[j] + 1
    return dict(seq=seq, mask=mask, parent=parent, depths=depths,
                levels=int(depths.max(initial=0)) + 1,
                n=player["sequence_count"], infos=infos)


def realization(a: dict, policy: np.ndarray) -> np.ndarray:
    r = np.zeros(a["n"], np.float64)
    r[0] = 1
    for j, info in enumerate(a["infos"]):
        r[info["sequences"]] = r[info["parent"]] * policy[j, :len(info["sequences"])]
    return r


def best_response(a: dict, immediate: np.ndarray, fixed: dict[int, np.ndarray] | None = None) -> float:
    q = np.asarray(immediate, dtype=np.float64).copy()
    for j in range(len(a["infos"]) - 1, -1, -1):
        info = a["infos"][j]
        values = q[info["sequences"]]
        v = np.dot(fixed[j], values) if fixed and j in fixed else values.max()
        q[info["parent"]] += v
    return float(q[0])


def payoff_arrays(game: dict, column: int = 3):
    # Coalesce identical sequence pairs on the host, keeping float64 validation.
    cells = {}
    for row in game["payoffs"]:
        key = (row[0], row[1])
        cells[key] = cells.get(key, 0.0) + row[2] * row[column]
    pairs = sorted(cells)
    return (np.array([p[0] for p in pairs], np.int32),
            np.array([p[1] for p in pairs], np.int32),
            np.array([cells[p] for p in pairs], np.float64))


def evaluate(game: dict, policies, column=3, fixed0=None) -> dict:
    aa = [arrays(p) for p in game["players"]]
    r0, r1 = [realization(a, np.asarray(p)) for a, p in zip(aa, policies)]
    s0, s1, val = payoff_arrays(game, column)
    u0 = np.bincount(s0, val*r1[s1], minlength=aa[0]["n"])
    u1 = np.bincount(s1, -val*r0[s0], minlength=aa[1]["n"])
    lower = -best_response(aa[1], u1)
    upper = best_response(aa[0], u0, fixed0)
    return dict(value=float(np.dot(r0, u0)), lower=lower, upper=upper,
                saddle_gap=max(0.0, upper-lower))


def solve(game: dict, iterations: int = 4096, column: int = 3,
          freeze_baseline: bool = False, checkpoints=()) -> dict:
    """CFR+ with linearly weighted, own-reach-weighted average strategies.

    A frozen baseline restricts only declared root decisions; later play is
    optimized. Its value loss is a root-decision diagnostic, not a match win rate.
    """
    import jax
    import jax.numpy as jnp

    validate(game)
    if iterations < 1:
        raise ValueError("iterations must be positive")
    aa = [arrays(p) for p in game["players"]]
    ss0, ss1, vals = payoff_arrays(game, column)
    s0, s1, val = jnp.asarray(ss0), jnp.asarray(ss1), jnp.asarray(vals, jnp.float32)
    fixed0 = {}
    fixed_masks, fixed_policies = [], []
    for side, a in enumerate(aa):
        fm, fp = np.zeros(len(a["seq"]), bool), np.zeros_like(a["seq"], np.float32)
        for j, info in enumerate(a["infos"]):
            if freeze_baseline and side == 0 and "baseline" in info:
                fm[j] = True
                fp[j, :len(info["baseline"])] = info["baseline"]
                fixed0[j] = np.array(info["baseline"])
        fixed_masks.append(jnp.asarray(fm))
        fixed_policies.append(jnp.asarray(fp))
    ja = [{k: jnp.asarray(a[k]) for k in ("seq", "mask", "parent", "depths")} for a in aa]

    def bucketed_reduce(rows, columns, weights, count):
        """Sparse matvec as bounded gathers/reductions, without contended adds.

        The same empty sequence may have thousands of child information sets.
        Scatter-add on that address serializes badly on TPU. Degree buckets
        preserve the exact sparse operator with <2x padding and unique outputs.
        """
        entries = [[] for _ in range(count)]
        for r, c, w in zip(rows, columns, weights):
            entries[int(r)].append((int(c), float(w)))
        groups = {}
        for row, items in enumerate(entries):
            width = 0 if not items else 1 << (len(items)-1).bit_length()
            groups.setdefault(width, []).append(row)
        buckets, order = [], []
        for width, row_ids in sorted(groups.items()):
            order.extend(row_ids)
            cols = np.zeros((len(row_ids), max(1,width)), np.int32)
            vals = np.zeros_like(cols, np.float32)
            for j, row in enumerate(row_ids):
                for k, (c,w) in enumerate(entries[row]):
                    cols[j,k], vals[j,k] = c,w
            buckets.append((jnp.asarray(cols), jnp.asarray(vals)))
        inverse = jnp.asarray(np.argsort(order), jnp.int32)
        def multiply(vector):
            parts = [jnp.sum(vector[c]*w, axis=1) for c,w in buckets]
            return jnp.concatenate(parts)[inverse]
        return multiply

    payoff_ops = [bucketed_reduce(ss0, ss1, vals, aa[0]["n"]),
                  bucketed_reduce(ss1, ss0, -vals, aa[1]["n"])]
    child_ops, reach_meta = [], []
    for a in aa:
        child_ops.append(bucketed_reduce(a["parent"], np.arange(len(a["parent"])),
                                         np.ones(len(a["parent"])), a["n"]))
        owner, action, parents, depths = [np.zeros(a["n"], np.int32) for _ in range(4)]
        for j, info in enumerate(a["infos"]):
            for k, sequence in enumerate(info["sequences"]):
                owner[sequence], action[sequence] = j,k
                parents[sequence], depths[sequence] = info["parent"], a["depths"][j]+1
        reach_meta.append(tuple(jnp.asarray(v) for v in (owner,action,parents,depths)))

    def policy(regret, side):
        mask = ja[side]["mask"]
        p = jnp.where(mask, regret, 0)
        total = p.sum(axis=1, keepdims=True)
        uniform = mask / jnp.maximum(1, mask.sum(axis=1, keepdims=True))
        p = jnp.where(total > 0, p/jnp.maximum(total, 1e-30), uniform)
        return jnp.where(fixed_masks[side][:, None], fixed_policies[side], p)

    def reach(pol, side):
        a, meta = ja[side], aa[side]
        owner, action, parents, depths = reach_meta[side]
        r = jnp.zeros(meta["n"], jnp.float32).at[0].set(1)
        def level(d, r):
            return jnp.where(depths==d+1, r[parents]*pol[owner,action], r)
        return jax.lax.fori_loop(0, meta["levels"], level, r)

    def advantage(immediate, pol, side):
        a, meta = ja[side], aa[side]
        adv = jnp.zeros_like(pol)
        def level(d, carry):
            q, adv = carry
            active = a["depths"] == meta["levels"] - 1 - d
            q_actions = q[a["seq"]]
            v = (pol*q_actions).sum(axis=1)
            adv = jnp.where(active[:, None] & a["mask"], q_actions-v[:, None], adv)
            q = q + child_ops[side](jnp.where(active, v, 0))
            return q, adv
        return jax.lax.fori_loop(0, meta["levels"], level, (immediate, adv))[1]

    @jax.jit
    def run(state, start, stop):
        def iteration(t, state):
            regrets, sums = state
            pol = tuple(policy(regrets[s], s) for s in range(2))
            r = tuple(reach(pol[s], s) for s in range(2))
            immediate = (payoff_ops[0](r[1]), payoff_ops[1](r[0]))
            updated = tuple(jnp.maximum(0, regrets[s]+advantage(immediate[s], pol[s], s)) for s in range(2))
            averaged = tuple(sums[s]+(t+1)*r[s][ja[s]["parent"]][:, None]*pol[s] for s in range(2))
            return updated, averaged
        return jax.lax.fori_loop(start, stop, iteration, state)

    zeros = tuple(jnp.zeros_like(a["seq"], dtype=jnp.float32) for a in ja)
    state = (zeros, zeros)
    start, reports = 0, []
    started = time.monotonic()
    for stop in sorted(set([iterations]+[int(k) for k in checkpoints if 0 < k <= iterations])):
        state = run(state, start, stop)
        policies = [np.asarray(policy(state[1][s], s), dtype=np.float64) for s in range(2)]
        # Re-normalize after device float32 reductions before float64 BR auditing.
        policies = [p/np.maximum(p.sum(axis=1, keepdims=True), 1e-30) for p in policies]
        report = evaluate(game, policies, column, fixed0)
        report.update(iterations=stop, elapsed_seconds=time.monotonic()-started)
        reports.append(report)
        start = stop
    return dict(backend=jax.default_backend(), checkpoints=reports,
                policies=[p.tolist() for p in policies], **reports[-1])


def baseline_game(game):
    """Keep actual per-world native choices, even if native violates invariance.

    The native first choice becomes an exogenous transition. Remove its decision
    sequences; retain its observed action in later information keys. This avoids
    pretending a hidden-dependent native policy was a legal ICO policy.
    """
    g = copy.deepcopy(game)
    player = g["players"][0]
    root_ids = {s for i in player["infosets"] if "baseline" in i for s in i["sequences"]}
    remap = {0: 0, **{s: 0 for s in root_ids}}
    infos = []
    n = 1
    for info in player["infosets"]:
        if "baseline" in info:
            if info["parent"] != 0:
                raise ValueError("native baseline must be at the root")
            continue
        info["parent"] = remap[info["parent"]]
        children = []
        for s in info["sequences"]:
            remap[s] = n
            children.append(n)
            n += 1
        info["sequences"] = children
        infos.append(info)
    player.update(infosets=infos, sequence_count=n)
    g["payoffs"] = [[remap[row[0]], *row[1:]] for row in g.pop("baseline_payoffs")]
    validate(g)
    return g


def compare(game, iterations=4096):
    checkpoints = [64, 256, 1024, iterations]
    free = solve(game, iterations, checkpoints=checkpoints)
    actual = "baseline_payoffs" in game
    frozen = solve(baseline_game(game), iterations, checkpoints=checkpoints) if actual else solve(
        game, iterations, freeze_baseline=True, checkpoints=checkpoints)
    # Frontier interval under sampled chance, using independent best responses.
    lo = evaluate(game, free["policies"], 4)["lower"]
    hi = evaluate(game, free["policies"], 5)["upper"]
    free_summary = {k: v for k, v in free.items() if k != "policies"}
    frozen_summary = {k: v for k, v in frozen.items() if k != "policies"}
    return dict(format="nc2000-ico-result-v1", metadata=game.get("metadata", {}),
                unrestricted=free_summary, baseline_root=frozen_summary,
                baseline_kind="actual-per-world-native-root" if actual else "information-set-fixed-root",
                estimated_root_loss_interval=[free["lower"]-frozen["upper"],
                                               free["upper"]-frozen["lower"]],
                sampled_game_frontier_interval=[lo, hi],
                full_game_certified=False,
                root_policies=[dict(key=i["key"], actions=i["actions"],
                                    baseline=i["baseline"], ico=free["policies"][0][j][:len(i["actions"])])
                               for j, i in enumerate(game["players"][0]["infosets"]) if "baseline" in i])


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("game", type=Path)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--iterations", type=int, default=4096)
    args = p.parse_args()
    raw = args.game.read_bytes()
    result = compare(json.loads(raw), args.iterations)
    result["game_sha256"] = hashlib.sha256(raw).hexdigest()
    args.out.write_text(json.dumps(result, indent=2)+"\n")
    print(json.dumps({k: v for k, v in result.items() if k not in ("root_policies", "metadata")}))


if __name__ == "__main__":
    main()
