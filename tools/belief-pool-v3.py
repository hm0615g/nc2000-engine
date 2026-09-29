#!/usr/bin/env python3
"""Opponent prior v3 and its c = 0.4 held-out arms
(docs/TEAM-POOL-REBUILD-PLAN.md step 2).

    python3 tools/belief-pool-v3.py build           # data/belief-pool-v3/belief-pool.json
    python3 tools/belief-pool-v3.py arms --out DIR  # diagnostic arm priors + cells
    python3 tools/belief-pool-v3.py report --out DIR --runs RUNDIR [--json OUT]

`report` reads RUNDIR/<arm>/cell-*.jsonl (arm `v3` is the reference) and
prints, per opponent group, each arm's score and its paired difference from
v3 over identical (cell, k, side) games, with a bootstrap over seed indices
(`--unit k`) or, when there are too few seeds for that, over cells
(`--unit cell`).

Membership and order come from the frozen belief-pool-v2 file plus the
records restored because no blind c = 0.4 evidence reconfirms their old
exclusion (RESTORED); new selection labels are never read, so relabelling
the own pool cannot move this file.
"""
import argparse
import collections
import json
import os

REPO = os.path.join(os.path.dirname(__file__), '..')
V2 = os.path.join(REPO, 'data/belief-pool-v2/belief-pool.json')
V3 = os.path.join(REPO, 'data/belief-pool-v3/belief-pool.json')
INV = os.path.join(REPO, 'data/team-inventory-v1/inventory.json')
MACHINE_FACTOR = 0.25
RESTORED = ['mjj-2020-kb-2020']
X_TEAMS = ['sample-07', 'sample-13', 'mjj-2021-q10con', 'mjj-2023-ukethdr']


def load(path):
    return json.load(open(path))


def members():
    v2 = load(V2)
    inv = {t['id']: t for t in load(INV)['teams']}
    ids = [t['id'] for t in v2['teams']]
    v1_label = {t['id']: t['label'] for t in v2['teams']}
    for r in RESTORED:
        assert r not in ids and inv[r]['measuredAs'] == r and inv[r]['eligibility']['status'] == 'eligible'
        ids.append(r)
        v1_label[r] = 'dominated'
    return ids, v1_label, inv


def weights(ids, inv, machine_factor=MACHINE_FACTOR):
    key = lambda i: inv[i]['variantCluster'] or i
    size = collections.Counter(key(i) for i in ids)
    return {i: round((1 / size[key(i)]) * (machine_factor if inv[i]['origin'] == 'machine' else 1), 6) for i in ids}


def provenance(t):
    s = t['source']
    return {'family': t['family'], 'origin': t['origin'], 'source': s['name'], 'url': s['url'], 'version': s.get('version')}


def prior(ids, v1_label, inv, w):
    return [{'id': i, 'v1Label': v1_label[i], 'weight': w[i], 'provenance': provenance(inv[i]), 'sets': inv[i]['sets']}
            for i in ids]


def build():
    ids, v1_label, inv = members()
    out = {
        'format': 'nc2000-belief-pool',
        'version': 3,
        'generator': 'tools/belief-pool-v3.py build',
        'note': 'Opponent prior: every eligible measured team of data/team-inventory-v1 (belief-pool-v2 plus '
                + ', '.join(RESTORED) + ', whose open-era dominated verdict is not reconfirmed under blind c = 0.4). '
                + f'weight = 1/(members of its variant cluster in this prior), x{MACHINE_FACTOR} for machine-generated teams; '
                + 'weighted identification and weighted-mode fallback (crates/bot/src/belief.rs). '
                + 'v1Label is the historical open-era label, not the selection label. See data/belief-pool-v3/README.md.',
        'teams': prior(ids, v1_label, inv, weights(ids, inv)),
    }
    os.makedirs(os.path.dirname(V3), exist_ok=True)
    with open(V3, 'w') as f:
        f.write(json.dumps(out, indent=1, ensure_ascii=False) + '\n')
    print(f'{V3}: {len(ids)} teams')


def arms(out_dir):
    os.makedirs(out_dir, exist_ok=True)
    ids, v1_label, inv = members()
    strong = [i for i in ids if v1_label[i] == 'strong']

    def dump(name, teams):
        json.dump({'teams': teams}, open(os.path.join(out_dir, f'prior-{name}.json'), 'w'))

    dump('strongonly', prior(strong, v1_label, inv, weights(strong, inv)))
    dump('mf1', prior(ids, v1_label, inv, weights(ids, inv, machine_factor=1.0)))
    dump('unweighted', [{k: v for k, v in t.items() if k != 'weight'} for t in prior(ids, v1_label, inv, weights(ids, inv))])

    sig = {i: inv[i]['previewSignature'] for i in ids}
    by_sig = collections.defaultdict(list)
    for i in ids:
        by_sig[sig[i]].append(i)
    human = [i for i in ids if inv[i]['origin'] == 'human']
    collide = [i for i in human if v1_label[i] == 'strong' and any(v1_label[j] != 'strong' for j in by_sig[sig[i]] if j != i)]
    weak = [i for i in human if v1_label[i] != 'strong']
    off = [l.strip() for f in ('offprior.txt', 'heldout.txt')
           for l in open(os.path.join(REPO, 'data/team-inventory-v1/eval', f)) if l.strip()]
    null = [i for i in human if v1_label[i] == 'strong' and i not in collide and not by_sig[sig[i]][1:]][:8]
    groups = {'collide': collide, 'weak': weak, 'offprior': off, 'strongnull': null}
    allc = []
    for g, ys in groups.items():
        cells = [f'{x} {y}\n' for x in X_TEAMS for y in ys if x != y]
        open(os.path.join(out_dir, f'cells-{g}.txt'), 'w').write(''.join(cells))
        allc += cells
        print(g, len(ys), ys)
    open(os.path.join(out_dir, 'cells.txt'), 'w').write(''.join(allc))
    json.dump(groups, open(os.path.join(out_dir, 'groups.json'), 'w'), indent=1)
    print('cells', len(allc))


def load_arm(d):
    import glob
    out = {}
    for f in glob.glob(os.path.join(d, 'cell-*.jsonl')):
        cell = os.path.basename(f)[5:-6]
        for line in open(f):
            r = json.loads(line)
            out[(cell, r['k'], r['row_p1'])] = r
    return out


def report(out_dir, runs, json_out, unit='k'):
    import numpy as np
    groups = json.load(open(os.path.join(out_dir, 'groups.json')))
    arms_ = {a: load_arm(os.path.join(runs, a)) for a in sorted(os.listdir(runs))
             if os.path.isdir(os.path.join(runs, a))}
    assert 'v3' in arms_, 'reference arm v3 missing'
    conds = {a: {json.dumps(r.get('cond', {}).get('belief_col')) for r in g.values()} for a, g in arms_.items()}
    rng = np.random.default_rng(20261101)
    res = {}
    for g, ys in groups.items():
        row = {'opponents': len(ys)}
        for arm, games in arms_.items():
            keys = sorted(k for k in arms_['v3'] if k[0].split('__')[1] in ys and k in games)
            if not keys:
                continue
            sc = np.array([games[k]['score'] for k in keys])
            ent = {'games': len(keys), 'score': float(sc.mean()),
                   'caps': sum(games[k]['result'] == 'cap' for k in keys)}
            if arm != 'v3':
                d = np.array([arms_['v3'][k]['score'] - games[k]['score'] for k in keys])
                byk = collections.defaultdict(list)
                for k, v in zip(keys, d):
                    byk[k[1] if unit == 'k' else k[0]].append(v)
                ks = sorted(byk)
                boots = [np.mean(np.concatenate([byk[x] for x in rng.choice(ks, len(ks))])) for _ in range(4000)]
                same = np.mean([arms_['v3'][k]['score'] == games[k]['score'] and arms_['v3'][k]['turns'] == games[k]['turns']
                                and arms_['v3'][k]['row_moves'] == games[k]['row_moves'] for k in keys])
                ent['v3_minus'] = {'diff': float(d.mean()), 'ci95': [float(np.percentile(boots, 2.5)), float(np.percentile(boots, 97.5))],
                                   'identicalGames': float(same)}
            row[arm] = ent
        res[g] = row
        line = [f"{g:11s} opp={len(ys):2d}"]
        for arm in sorted(row):
            if arm == 'opponents':
                continue
            e = row[arm]
            t = f"{arm} {e['score']:.3f}"
            if 'v3_minus' in e:
                m = e['v3_minus']
                t += f" (v3-{arm} {m['diff']:+.3f} [{m['ci95'][0]:+.3f},{m['ci95'][1]:+.3f}] same {m['identicalGames']:.2f})"
            line.append(t)
        print(' | '.join(line))
    if json_out:
        json.dump({'groups': groups, 'bootstrapUnit': unit,
                   'columnPrior': {a: sorted(c) for a, c in conds.items()}, 'results': res},
                  open(json_out, 'w'), indent=1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('cmd', choices=['build', 'arms', 'report'])
    ap.add_argument('--out')
    ap.add_argument('--runs')
    ap.add_argument('--json')
    ap.add_argument('--unit', choices=['k', 'cell'], default='k')
    a = ap.parse_args()
    if a.cmd == 'build':
        build()
    elif a.cmd == 'arms':
        arms(a.out)
    else:
        report(a.out, a.runs, a.json, a.unit)


if __name__ == '__main__':
    main()
