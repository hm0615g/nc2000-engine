#!/usr/bin/env python3
"""Execution audit for the team-pool rebuild (PROTOCOL.md, dominated
condition 3): from team_eval games, how often the bot selects each mon of a
team (the recorded preview answer; runs made before that field existed
fall back to first appearance, a lower bound) and uses each of its moves
once that mon is on the field.

    python3 tools/team-exec-audit.py --cells DIR [--cells DIR ...] \
        [--team ID ...] [--json OUT]
"""
import argparse
import collections
import glob
import json
import os
import re

REPO = os.path.join(os.path.dirname(__file__), '..')


def toid(s):
    return re.sub(r'[^a-z0-9]', '', str(s).lower())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--cells', action='append', required=True)
    ap.add_argument('--team', action='append')
    ap.add_argument('--json')
    a = ap.parse_args()
    teams = {t['id']: t for t in json.load(open(os.path.join(REPO, 'data/team-inventory-v1/eval/teams.json')))['teams']}
    games = collections.Counter()
    picked = collections.defaultdict(collections.Counter)
    appeared = collections.defaultdict(collections.Counter)
    lead = collections.defaultdict(collections.Counter)
    used = collections.defaultdict(collections.Counter)  # (team) -> (species, move) games used
    for d in a.cells:
        for f in glob.glob(os.path.join(d, 'cell-*.jsonl')):
            row, col = os.path.basename(f)[5:-6].split('__')
            for line in open(f):
                r = json.loads(line)
                for tid, picks, moves in ((row, r.get('row_selected') or r['row_picks'], r['row_moves']),
                                          (col, r.get('col_selected') or r['col_picks'], r['col_moves'])):
                    if a.team and tid not in a.team:
                        continue
                    games[tid] += 1
                    for sp in picks:
                        picked[tid][toid(sp)] += 1
                    for sp in (r['row_picks'] if tid == row else r['col_picks']):
                        appeared[tid][toid(sp)] += 1
                    if picks:
                        lead[tid][toid(picks[0])] += 1
                    for sp, mv in moves.items():
                        for m in mv:
                            used[tid][(toid(sp), toid(m))] += 1
    out = {}
    for tid in sorted(games):
        n = games[tid]
        mons = {}
        for s in teams[tid]['sets']:
            sp = toid(s['species'])
            pk = picked[tid][sp]
            ap = appeared[tid][sp]
            mons[s['species']] = {
                'pickRate': pk / n, 'leadRate': lead[tid][sp] / n, 'appearRate': ap / n,
                'moveUseRate': {m: (used[tid][(sp, toid(m))] / ap if ap else None) for m in s['moves']},
            }
        out[tid] = {'games': n, 'mons': mons}
    if a.json:
        json.dump(out, open(a.json, 'w'), indent=1)
    for tid, v in out.items():
        print(f"{tid} ({v['games']} games)")
        for sp, m in v['mons'].items():
            mv = ', '.join(f"{k} {x:.2f}" if x is not None else f"{k} -" for k, x in m['moveUseRate'].items())
            print(f"   {sp:12s} pick {m['pickRate']:.2f} lead {m['leadRate']:.2f} | {mv}")


if __name__ == '__main__':
    main()
