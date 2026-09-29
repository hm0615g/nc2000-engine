#!/usr/bin/env python3
"""Score opponent-prior candidate pools against real human teams
(docs/TEAM-POOL-REBUILD-PLAN.md step 3): how often the blind belief can
identify a human team, and how well its fallback imputation predicts the
moves humans actually reveal.

    python3 tools/prior-corpus-eval.py --corpus tmp/corpus-spectator \
        --pool NAME=FILE [--pool NAME=FILE ...] [--json OUT]

The corpus is the local, gitignored spectator archive
(data/corpus-spectator-logs.zip unpacked); only aggregate numbers leave
this script — no handles, no per-game records.

Mirrors crates/bot/src/belief.rs:
  identification = a pool team whose six (species, level, has-item) match
    the preview exactly and whose sets contain every move the log reveals;
  fallback (no team survives) = per species, the first pool team in FILE
    ORDER carrying it, else the first valid community rental carrying it,
    else nothing (the learnset default is scored as zero recall).
With --weighted-mode, the fallback instead takes the loadout with the
largest total `weight` among pool teams carrying the species (the
weighted-mode rule this study also evaluates).
"""
import argparse
import collections
import glob
import json
import os
import re
import sys

REPO = os.path.join(os.path.dirname(__file__), '..')


def toid(s):
    return re.sub(r'[^a-z0-9]', '', str(s).lower())


def mv_key(m):
    m = toid(m)
    return 'hiddenpower' if m.startswith('hiddenpower') else m


def parse_game(path):
    sides = {'p1': {}, 'p2': {}}
    nick = {}
    for line in open(path, encoding='utf-8', errors='replace'):
        p = line.rstrip('\n').split('|')
        if len(p) < 3:
            continue
        tag = p[1]
        if tag == 'poke':
            side = p[2]
            det = p[3].split(', ')
            sp = toid(det[0].replace('-*', ''))
            lv = 50
            for d in det[1:]:
                if d.startswith('L'):
                    lv = int(d[1:])
            sides[side][sp] = {'level': lv, 'item': p[4] == 'item' if len(p) > 4 else False, 'moves': set()}
        elif tag in ('switch', 'drag') and len(p) > 3:
            who = p[2]
            side = who[:2]
            nick[(side, who.split(': ', 1)[1])] = toid(p[3].split(', ')[0])
        elif tag == 'move' and len(p) > 3:
            who = p[2]
            side = who[:2]
            sp = nick.get((side, who.split(': ', 1)[1]))
            if sp and sp in sides[side] and '[from]' not in line:
                sides[side][sp]['moves'].add(mv_key(p[3]))
    return [t for t in sides.values() if len(t) == 6]


def load_pool(path):
    d = json.load(open(path))
    teams = d['teams'] if isinstance(d, dict) else d
    out = []
    for t in teams:
        mons = {}
        for s in t['sets']:
            mons[toid(s['species'])] = {
                'level': s.get('level', 55), 'item': bool(s.get('item')),
                'moves': {mv_key(m) for m in s['moves']}, 'loadout': (toid(s.get('item', '')), tuple(sorted(mv_key(m) for m in s['moves']))),
            }
        out.append({'id': t['id'], 'weight': t.get('weight', 1.0), 'mons': mons})
    return out


def rental_sets():
    d = json.load(open(os.path.join(REPO, 'data/community-rentals-v0/teams.json')))
    # belief.rs keeps only format-legal complete teams not tagged for another rule
    bad = {16, 21, 22, 23, 24, 25, 26, 27, 28}
    out = []
    for t in d['teams']:
        if t['cban'] in bad:
            continue
        for s in t['sets']:
            out.append((toid(s['species']), {mv_key(m) for m in s['moves']}))
    return out


def evaluate(teams, pool, rentals, weighted_mode):
    n = len(teams)
    ident = preview_hit = 0
    rec_num = rec_den = 0
    src = collections.Counter()
    for t in teams:
        sig = sorted((sp, m['level'], m['item']) for sp, m in t.items())
        cands = [c for c in pool if sorted((sp, m['level'], m['item']) for sp, m in c['mons'].items()) == sig]
        if cands:
            preview_hit += 1
        alive = [c for c in cands if all(t[sp]['moves'] <= c['mons'][sp]['moves'] for sp in t)]
        if alive:
            ident += 1
            continue
        for sp, m in t.items():
            if not m['moves']:
                continue
            guess = None
            if weighted_mode:
                tally = collections.defaultdict(float)
                first = {}
                for c in pool:
                    if sp in c['mons']:
                        lo = c['mons'][sp]['loadout']
                        tally[lo] += c['weight']
                        first.setdefault(lo, c['mons'][sp]['moves'])
                if tally:
                    best = max(tally, key=lambda k: tally[k])
                    guess = first[best]
                    src['pool'] += 1
            else:
                for c in pool:
                    if sp in c['mons']:
                        guess = c['mons'][sp]['moves']
                        src['pool'] += 1
                        break
            if guess is None:
                for rsp, mv in rentals:
                    if rsp == sp:
                        guess = mv
                        src['rental'] += 1
                        break
            if guess is None:
                src['learnset'] += 1
                guess = set()
            rec_num += len(m['moves'] & guess)
            rec_den += len(m['moves'])
    return {
        'teams': n,
        'previewMatch': preview_hit / n,
        'identified': ident / n,
        'fallbackMoveRecall': rec_num / rec_den if rec_den else None,
        'fallbackRevealedMoves': rec_den,
        'fallbackSources': dict(src),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--corpus', required=True)
    ap.add_argument('--pool', action='append', required=True, help='NAME=FILE')
    ap.add_argument('--weighted-mode', action='append', default=[], help='NAME: evaluate NAME with the weighted-mode fallback')
    ap.add_argument('--half', choices=['odd', 'even'], help='score only one half of the games (sorted by file name)')
    ap.add_argument('--json')
    a = ap.parse_args()
    files = sorted(glob.glob(os.path.join(a.corpus, '*.raw.log')))
    if a.half:
        files = files[(0 if a.half == 'even' else 1)::2]
    teams = [t for f in files for t in parse_game(f)]
    rentals = rental_sets()
    res = {}
    for spec in a.pool:
        name, path = spec.split('=', 1)
        pool = load_pool(path)
        res[name] = evaluate(teams, pool, rentals, weighted_mode=False)
        if name in a.weighted_mode:
            res[name + '+wmode'] = evaluate(teams, pool, rentals, weighted_mode=True)
    for k, v in res.items():
        print(f"{k:28s} teams={v['teams']} preview={v['previewMatch']:.3f} identified={v['identified']:.3f} "
              f"recall={v['fallbackMoveRecall']:.3f} (n={v['fallbackRevealedMoves']}) {v['fallbackSources']}")
    if a.json:
        json.dump({'games': len(files), 'results': res}, open(a.json, 'w'), indent=1)


if __name__ == '__main__':
    sys.exit(main())
