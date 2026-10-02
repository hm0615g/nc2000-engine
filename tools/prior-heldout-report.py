#!/usr/bin/env python3
"""Summarize the opponent-prior held-out test (tools/prior-heldout-setup.py):
X's score per opponent group and arm, paired arm differences with a
seed-index bootstrap, and the share of bit-identical games.

    python3 tools/prior-heldout-report.py --dir DIR [--json OUT]
"""
import argparse
import collections
import glob
import json
import os

import numpy as np


def load(d):
    out = {}
    for f in glob.glob(os.path.join(d, 'cell-*.jsonl')):
        cell = os.path.basename(f)[5:-6]
        for line in open(f):
            r = json.loads(line)
            out[(cell, r['k'], r['row_p1'])] = r
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--dir', required=True)
    ap.add_argument('--json')
    a = ap.parse_args()
    groups = json.load(open(os.path.join(a.dir, 'groups.json')))
    arms = {arm: load(os.path.join(a.dir, arm)) for arm in ('new', 'strongonly', 'v1')}
    rng = np.random.default_rng(3)
    res = {}
    for g, ys in groups.items():
        if not ys:
            res[g] = {'opponents': 0}
            continue
        keys = sorted(k for k in arms['new'] if k[0].split('__')[1] in ys and all(k in arms[x] for x in arms))
        row = {'opponents': len(ys), 'games': len(keys)}
        for arm in arms:
            row[arm] = float(np.mean([arms[arm][k]['score'] for k in keys]))
        for other in ('strongonly', 'v1'):
            d = np.array([arms['new'][k]['score'] - arms[other][k]['score'] for k in keys])
            byk = collections.defaultdict(list)
            for k, v in zip(keys, d):
                byk[k[1]].append(v)
            ks = sorted(byk)
            boots = [np.mean(np.concatenate([byk[x] for x in rng.choice(ks, len(ks))])) for _ in range(4000)]
            same = np.mean([arms['new'][k]['score'] == arms[other][k]['score'] and arms['new'][k]['turns'] == arms[other][k]['turns']
                            and arms['new'][k]['row_moves'] == arms[other][k]['row_moves'] for k in keys])
            row[f'new_minus_{other}'] = {'diff': float(d.mean()), 'ci95': [float(np.percentile(boots, 2.5)), float(np.percentile(boots, 97.5))],
                                         'identicalGames': float(same)}
        res[g] = row
        print(f"{g:11s} n={len(keys):4d} new {row['new']:.3f} strongonly {row['strongonly']:.3f} v1 {row['v1']:.3f} | "
              + ' | '.join(f"new-{o} {row[f'new_minus_{o}']['diff']:+.3f} [{row[f'new_minus_{o}']['ci95'][0]:+.3f},{row[f'new_minus_{o}']['ci95'][1]:+.3f}] same {row[f'new_minus_{o}']['identicalGames']:.2f}" for o in ('strongonly', 'v1')))
    if a.json:
        json.dump({'groups': groups, 'results': res}, open(a.json, 'w'), indent=1)


if __name__ == '__main__':
    main()
