#!/usr/bin/env python3
"""Solve the finite team game from team_eval cells (docs/TEAM-POOL-REBUILD-PLAN.md
step 4): symmetric zero-sum matrix A[i][j] = mean score of i against j,
RM+ equilibrium, best-response margin, and a seed bootstrap for support
stability.

    python3 tools/nash-solve.py --cells DIR [--cells DIR ...] --ids FILE \
        [--agent LABEL] [--boot 300] [--out solution.json]

Cells from several runs are pooled per ordered pair only when they carry the
same agent label (--agent selects it); a refinement run with fresh seeds
therefore adds games to the matrix of record under the same condition.
"""
import argparse
import collections
import glob
import json
import os

import numpy as np


def load(dirs, agent):
    games = collections.defaultdict(list)  # (a, b) -> [(k_uid, score_a)]
    labels = collections.Counter()
    for d in dirs:
        for f in glob.glob(os.path.join(d, 'cell-*.jsonl')):
            row, col = os.path.basename(f)[5:-6].split('__')
            for line in open(f):
                r = json.loads(line)
                labels[r['agent']] += 1
                if agent and r['agent'] != agent:
                    continue
                uid = (d, r['k'])
                games[(row, col)].append((uid, r['score']))
                games[(col, row)].append((uid, 1.0 - r['score']))
    return games, labels


def tensors(ids, games):
    """Per ordered pair and seed uid: score sum and game count, so a
    bootstrap resample is two dot products."""
    uids = sorted({u for g in games.values() for u, _ in g})
    ui = {u: i for i, u in enumerate(uids)}
    n = len(ids)
    S = np.zeros((n, n, len(uids)))
    C = np.zeros((n, n, len(uids)))
    for i, a in enumerate(ids):
        for j, b in enumerate(ids):
            if i == j:
                continue
            for u, sc in games.get((a, b), []):
                S[i, j, ui[u]] += sc
                C[i, j, ui[u]] += 1
    return uids, S, C


def build(S, C, m=None):
    if m is None:
        m = np.ones(S.shape[2])
    num = S @ m
    den = C @ m
    A = np.where(den > 0, num / np.maximum(den, 1e-12), 0.5)
    np.fill_diagonal(A, 0.5)
    return A, den


def rm_plus(A, iters=20000):
    """Row maximizes x^T A y, column minimizes; alternating RM+ with linear
    averaging. Returns the averaged row strategy (symmetric game: the
    column's is the same up to noise) and the column average."""
    n = A.shape[0]
    Rx = np.zeros(n)
    Ry = np.zeros(n)
    sx = np.zeros(n)
    sy = np.zeros(n)
    x = np.ones(n) / n
    y = np.ones(n) / n
    for t in range(1, iters + 1):
        ux = A @ y
        Rx = np.maximum(Rx + ux - x @ ux, 0)
        x = Rx / Rx.sum() if Rx.sum() > 0 else np.ones(n) / n
        uy = -(x @ A)
        Ry = np.maximum(Ry + uy - y @ uy, 0)
        y = Ry / Ry.sum() if Ry.sum() > 0 else np.ones(n) / n
        sx += t * x
        sy += t * y
    x = sx / sx.sum()
    y = sy / sy.sum()
    s = (x + y) / 2
    return s


def margin(A, x):
    return float(np.max(A @ x) - 0.5), int(np.argmax(A @ x))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--cells', action='append', required=True)
    ap.add_argument('--ids', required=True)
    ap.add_argument('--agent')
    ap.add_argument('--boot', type=int, default=300)
    ap.add_argument('--iters', type=int, default=20000)
    ap.add_argument('--floor', type=float, default=0.01)
    ap.add_argument('--out')
    a = ap.parse_args()
    ids = [l.strip() for l in open(a.ids) if l.strip() and not l.startswith('#')]
    games, labels = load(a.cells, a.agent)
    print('agent labels seen:', dict(labels))
    uid_list, S, C = tensors(ids, games)
    A, N = build(S, C)
    missing = [(ids[i], ids[j]) for i in range(len(ids)) for j in range(len(ids)) if i != j and N[i, j] == 0]
    if missing:
        print(f'WARNING {len(missing)} ordered pairs have no games, e.g. {missing[:3]}')
    x = rm_plus(A, a.iters)
    m, br = margin(A, x)
    support = sorted(((ids[i], float(x[i])) for i in range(len(ids)) if x[i] >= a.floor), key=lambda t: -t[1])
    print(f'solution margin {m:+.4f} (best response {ids[br]}); support >= {a.floor}:')
    for tid, w in support:
        print(f'  {tid:34s} {w:.3f}')
    # bootstrap over the seed index (per run)
    rng = np.random.default_rng(1)
    freq = collections.Counter()
    wsum = collections.defaultdict(list)
    for _ in range(a.boot):
        mult = np.bincount(rng.integers(0, len(uid_list), len(uid_list)), minlength=len(uid_list)).astype(float)
        Ab, _ = build(S, C, mult)
        xb = rm_plus(Ab, max(2000, a.iters // 5))
        for i, tid in enumerate(ids):
            wsum[tid].append(float(xb[i]))
            if xb[i] >= a.floor:
                freq[tid] += 1
    boot = {tid: {'supportRate': freq[tid] / a.boot, 'medianWeight': float(np.median(wsum[tid])),
                  'p90Weight': float(np.percentile(wsum[tid], 90))} for tid in ids}
    print('bootstrap support rate (>=0.05 shown):')
    for tid in sorted(ids, key=lambda t: -boot[t]['supportRate']):
        if boot[tid]['supportRate'] >= 0.05:
            print(f"  {tid:34s} {boot[tid]['supportRate']:.2f} median w {boot[tid]['medianWeight']:.3f}")
    if a.out:
        json.dump({
            'ids': ids, 'agent': a.agent, 'cells': a.cells,
            'matrix': A.tolist(), 'games': N.tolist(),
            'weights': {ids[i]: float(x[i]) for i in range(len(ids))},
            'margin': m, 'bestResponse': ids[br],
            'bootstrap': boot, 'bootSamples': a.boot,
        }, open(a.out, 'w'), indent=1)


if __name__ == '__main__':
    main()
