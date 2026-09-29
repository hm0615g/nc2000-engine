#!/usr/bin/env python3
"""Challenge a solved mixture with teams outside its solved game
(docs/TEAM-POOL-REBUILD-PLAN.md step 4).

    python3 tools/nash-challenge.py setup --solution SOL.json --challengers IDS... --out DIR
    python3 tools/nash-challenge.py report --solution SOL.json --dir DIR [--json OUT]

`setup` writes DIR/cells.txt (challenger x support team). `report` scores
each challenger against the mixture: sum over support s of w_s * score(c vs s),
with a seed-index bootstrap interval; a challenger at or above 0.5 exposes a
gap the solved game did not contain.
"""
import argparse
import collections
import glob
import json
import os

import numpy as np


def support(sol, floor=0.01):
    w = {t: v for t, v in sol['weights'].items() if v >= floor}
    z = sum(w.values())
    return {t: v / z for t, v in w.items()}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('mode', choices=['setup', 'report'])
    ap.add_argument('--solution', required=True)
    ap.add_argument('--challengers', nargs='*', default=[])
    ap.add_argument('--out')
    ap.add_argument('--dir')
    ap.add_argument('--json')
    a = ap.parse_args()
    sup = support(json.load(open(a.solution)))
    if a.mode == 'setup':
        os.makedirs(a.out, exist_ok=True)
        cells = [f'{c} {s}\n' for c in a.challengers for s in sup if c != s]
        open(os.path.join(a.out, 'cells.txt'), 'w').write(''.join(cells))
        print(len(a.challengers), 'challengers x', len(sup), 'support =', len(cells), 'cells')
        return
    games = collections.defaultdict(lambda: collections.defaultdict(list))  # c -> s -> [(k, score)]
    for f in glob.glob(os.path.join(a.dir, 'cell-*.jsonl')):
        c, s = os.path.basename(f)[5:-6].split('__')
        for line in open(f):
            r = json.loads(line)
            games[c][s].append((r['k'], r['score']))
    rng = np.random.default_rng(15)
    res = {}
    for c, by_s in games.items():
        ks = sorted({k for g in by_s.values() for k, _ in g})

        def val(sample):
            tot = 0.0
            for s, w in sup.items():
                cnt = collections.Counter(sample)
                vals = [v for k, v in by_s.get(s, []) for _ in range(cnt[k])]
                tot += w * (np.mean(vals) if vals else 0.5)
            return tot
        point = val(ks)
        boots = [val(list(rng.choice(ks, len(ks)))) for _ in range(1000)]
        res[c] = {'score': point, 'ci95': [float(np.percentile(boots, 2.5)), float(np.percentile(boots, 97.5))],
                  'games': sum(len(v) for v in by_s.values()),
                  'perSupport': {s: float(np.mean([v for _, v in by_s[s]])) for s in by_s}}
    for c in sorted(res, key=lambda c: -res[c]['score']):
        r = res[c]
        flag = '  <-- at or above 0.5' if r['score'] >= 0.5 else ''
        print(f"{c:34s} {r['score']:.3f} [{r['ci95'][0]:.3f},{r['ci95'][1]:.3f}] n={r['games']}{flag}")
    if a.json:
        json.dump({'support': sup, 'results': res}, open(a.json, 'w'), indent=1)


if __name__ == '__main__':
    main()
