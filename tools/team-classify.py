#!/usr/bin/env python3
"""Panel scores, calibration and label proposals for the team-pool rebuild
(data/team-inventory-v1/PROTOCOL.md).

    python3 tools/team-classify.py --cells DIR [--cells DIR ...] \
        [--report OUT.json] [--matrix OUT.json]

Reads team_eval cell JSONL (tmp/, gitignored), never writes labels itself:
the report is the evidence a label decision cites.
"""
import argparse
import collections
import glob
import json
import os
import re

import numpy as np

REPO = os.path.join(os.path.dirname(__file__), '..')
INV = os.path.join(REPO, 'data/team-inventory-v1/inventory.json')
EVAL = os.path.join(REPO, 'data/team-inventory-v1/eval')
TRUSTED = {'majinjima', 'hc75-top8', 'smogon-hub-samples'}
BOOT = 2000


def toid(s):
    return re.sub(r'[^a-z0-9]', '', str(s).lower())


def load_games(dirs):
    """{(row, col): [(k, row_p1, score_row, result, rec)]} plus agent label."""
    games = collections.defaultdict(list)
    agents = set()
    for d in dirs:
        for f in glob.glob(os.path.join(d, 'cell-*.jsonl')):
            row, col = os.path.basename(f)[5:-6].split('__')
            for line in open(f):
                r = json.loads(line)
                agents.add(r['agent'])
                games[(row, col)].append(r)
    return games, agents


def strata_of(sets):
    moves = [{toid(m) for m in s['moves']} for s in sets]
    allm = set().union(*moves)
    out = set()
    if (allm & {'meanlook', 'spiderweb'}) and (allm & {'perishsong', 'batonpass'}):
        out.add('trap')
    if sum(1 for m in moves if m & {'explosion', 'selfdestruct'}) >= 2:
        out.add('boom')
    if 'spikes' in allm and (allm & {'roar', 'whirlwind'}):
        out.add('spikes')
    rec = {'rest', 'recover', 'milkdrink', 'softboiled', 'moonlight', 'synthesis', 'morningsun'}
    if sum(1 for m in moves if m & rec) >= 3:
        out.add('recovery')
    for s in sets:
        if s['level'] == 55 and toid(s['species']) in ('zapdos', 'raikou', 'jolteon'):
            out.add('electric55')
        if s['level'] == 55 and toid(s['species']) == 'snorlax':
            out.add('snorlax55')
    return out


class Matrix:
    """Per ordered pair (x, y): score array [k, side] (side 0 = x on p1)."""

    def __init__(self, games):
        self.cells = {}
        self.meta = collections.defaultdict(list)
        ks = set()
        for (row, col), recs in games.items():
            for r in recs:
                ks.add(r['k'])
        self.K = max(ks) + 1 if ks else 0
        for (row, col), recs in games.items():
            a = np.full((self.K, 2), np.nan)
            b = np.full((self.K, 2), np.nan)
            for r in recs:
                side = 0 if r['row_p1'] else 1
                a[r['k'], side] = r['score']
                b[r['k'], 1 - side] = 1.0 - r['score']
                self.meta[(row, col)].append(r)
            self.cells[(row, col)] = a
            self.cells[(col, row)] = b

    def arr(self, x, y):
        return self.cells.get((x, y))


def panel_setup(inv, heldout):
    by = {t['id']: t for t in inv['teams']}
    panel = [t['id'] for t in inv['teams']
             if t['eligibility']['status'] == 'eligible' and t['measuredAs'] == t['id'] and t['origin'] == 'human']
    panel += heldout
    cluster = {tid: (by[tid]['variantCluster'] or f'heldout:{tid}') for tid in panel}
    size = collections.Counter(cluster[t] for t in panel)
    w = {t: 1.0 / size[cluster[t]] for t in panel}
    return panel, cluster, w


def panel_score(M, x, panel, w, own_cluster, cluster, ks=None):
    """Weighted mean over panel teams outside x's cluster; ks = bootstrap k
    multiset (indices) or None for all."""
    num = den = 0.0
    for p in panel:
        if p == x or (own_cluster is not None and cluster[p] == own_cluster):
            continue
        a = M.arr(x, p)
        if a is None:
            continue
        sub = a if ks is None else a[ks]
        m = np.nanmean(sub) if np.any(~np.isnan(sub)) else np.nan
        if np.isnan(m):
            continue
        num += w[p] * m
        den += w[p]
    return num / den if den else np.nan


def boot_ci(fn, K, rng, n=BOOT):
    vals = []
    for _ in range(n):
        ks = rng.integers(0, K, K)
        vals.append(fn(ks))
    vals = np.array(vals)
    return float(np.nanpercentile(vals, 2.5)), float(np.nanpercentile(vals, 97.5))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--cells', action='append', required=True)
    ap.add_argument('--report')
    ap.add_argument('--boot', type=int, default=BOOT)
    a = ap.parse_args()
    inv = json.load(open(INV))
    by = {t['id']: t for t in inv['teams']}
    teams_eval = {t['id']: t for t in json.load(open(os.path.join(EVAL, 'teams.json')))['teams']}
    heldout = [l.strip() for l in open(os.path.join(EVAL, 'heldout.txt')) if l.strip()]
    controls = [l.strip() for l in open(os.path.join(EVAL, 'controls.txt')) if l.strip()]
    measured = [l.strip() for l in open(os.path.join(EVAL, 'measured.txt')) if l.strip()]
    games, agents = load_games(a.cells)
    M = Matrix(games)
    rng = np.random.default_rng(20260929)
    panel, cluster, w = panel_setup(inv, heldout)
    strata = {p: strata_of(teams_eval[p]['sets']) for p in panel}
    tot_w = sum(w.values())
    stratum_w = collections.Counter()
    for p in panel:
        for s in strata[p]:
            stratum_w[s] += w[p]
    important = sorted(s for s, v in stratum_w.items() if v / tot_w >= 0.10)

    rows = {}
    for x in measured + controls:
        own = cluster.get(x) if x in cluster else (by[x]['variantCluster'] if x in by else None)
        if x in controls:
            own = cluster.get('sample-07')
        s = panel_score(M, x, panel, w, own, cluster)
        lo, hi = boot_ci(lambda ks: panel_score(M, x, panel, w, own, cluster, ks), M.K, rng, a.boot)
        n = sum(int(np.sum(~np.isnan(M.arr(x, p)))) for p in panel if M.arr(x, p) is not None and p != x)
        caps = sum(1 for (r, c), recs in M.meta.items() if x in (r, c) for g in recs if g['result'] == 'cap')
        per_stratum = {}
        for st in important:
            ps = [p for p in panel if st in strata[p]]
            ww = {p: w[p] for p in ps}
            per_stratum[st] = panel_score(M, x, ps, ww, own, cluster)
        rows[x] = {'score': s, 'ci': [lo, hi], 'n': n, 'caps': caps, 'strata': per_stratum}

    # calibration
    ceil = rows['sample-07']
    conf = rows['ctl-conf-s07-order']
    half = lambda r: (r['ci'][1] - r['ci'][0]) / 2
    noise = max(half(ceil), half(conf), abs(conf['score'] - ceil['score']))
    brk = min(ceil['score'] - rows['ctl-floor-s07']['score'], ceil['score'] - rows['ctl-del-s07-boom']['score'])
    verdict = 'VALID' if brk >= 2 * noise else 'WEAK' if brk > noise else 'INVALID'
    calib = {'noise': noise, 'break': brk, 'verdict': verdict,
             'gene_deletion_delta': ceil['score'] - rows['ctl-del-s07-gene']['score'],
             'arms': {k: rows[k] for k in ['sample-07'] + controls}}

    # label proposals (discovery stage)
    proposals = {}
    for x in measured:
        r = rows[x]
        lo, hi = r['ci']
        if lo > 0.45:
            proposals[x] = 'strong'
        elif hi < 0.45:
            proposals[x] = 'weak'
        else:
            proposals[x] = 'confirm'

    # dominated screen: trusted same-cluster replacements with same level allocation
    def level_alloc(t):
        return sorted((toid(s['species']), s['level']) for s in t['sets'] if s['level'] > 50)
    dom = []
    for x in measured:
        tx = by[x]
        for y in measured:
            if y == x:
                continue
            ty = by[y]
            if ty['family'] not in TRUSTED or ty['variantCluster'] != tx['variantCluster'] or tx['variantCluster'] is None:
                continue
            if level_alloc(tx) != level_alloc(ty):
                continue
            diff = rows[y]['score'] - rows[x]['score']
            dom.append({'candidate': x, 'replacement': y, 'delta': diff})

    out = {'agents': sorted(agents), 'K': M.K, 'panel': {p: {'weight': w[p], 'cluster': cluster[p], 'strata': sorted(strata[p])} for p in panel},
           'importantStrata': important, 'strataWeight': {k: v / tot_w for k, v in stratum_w.items()},
           'calibration': calib, 'rows': rows, 'proposals': proposals, 'dominatedScreen': dom}
    if a.report:
        json.dump(out, open(a.report, 'w'), indent=1)
    print(f"agents={sorted(agents)} K={M.K} panel={len(panel)} important strata={important}")
    print(f"calibration: noise={noise:.3f} break={brk:.3f} -> {verdict}; gene deletion delta={calib['gene_deletion_delta']:+.3f}")
    for k in ['sample-07'] + controls:
        r = rows[k]
        print(f"  {k:22s} {r['score']:.3f} [{r['ci'][0]:.3f},{r['ci'][1]:.3f}]")
    cnt = collections.Counter(proposals.values())
    print('proposals:', dict(cnt))
    for x in sorted(measured, key=lambda x: -rows[x]['score']):
        r = rows[x]
        print(f"{x:34s} {by[x]['family'][:14]:14s} {r['score']:.3f} [{r['ci'][0]:.3f},{r['ci'][1]:.3f}] n={r['n']:4d} {proposals[x]}")
    print('dominated screen (trusted same-cluster, same level allocation):')
    for d in sorted(dom, key=lambda d: -d['delta']):
        print(f"  {d['candidate']:26s} vs {d['replacement']:26s} delta={d['delta']:+.3f}")


if __name__ == '__main__':
    main()
