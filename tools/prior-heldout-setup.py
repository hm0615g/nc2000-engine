#!/usr/bin/env python3
"""Arms and cells for the opponent-prior held-out test
(docs/TEAM-POOL-REBUILD-PLAN.md step 3).

    python3 tools/prior-heldout-setup.py --classification FILE --out DIR

Row X = a blind bot whose belief is the arm's prior; column Y = a blind bot
with the new prior (fixed across arms). X's teams and Y's teams are the
same in every arm, so arms differ only in X's belief. Writes:
  DIR/prior-{new,strongonly}.json   weighted priors (new = strong+weak+pending)
  DIR/cells-{collide,weak,offprior,strongnull}.txt   and cells.txt (all)
V1 (data/belief-pool-v1/belief-pool.json) is the shipped reference arm.
"""
import argparse
import collections
import json
import os

REPO = os.path.join(os.path.dirname(__file__), '..')
X_TEAMS = ['sample-07', 'sample-13', 'mjj-2021-q10con', 'mjj-2023-ukethdr']


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--classification', required=True)
    ap.add_argument('--out', required=True)
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    inv = {t['id']: t for t in json.load(open(os.path.join(REPO, 'data/team-inventory-v1/inventory.json')))['teams']}
    cls = json.load(open(a.classification))
    labels = {k: v['label'] for k, v in cls['labels'].items()}
    mf = cls['allocation']['machineFactor']

    def prior(ids):
        cl = collections.Counter(inv[i]['variantCluster'] or i for i in ids)
        return {'teams': [{'id': i, 'weight': (1 / cl[inv[i]['variantCluster'] or i]) * (mf if inv[i]['origin'] == 'machine' else 1),
                           'sets': inv[i]['sets']} for i in ids]}
    order = {'strong': 0, 'pending': 1, 'weak': 2}
    new = sorted([i for i, l in labels.items() if l != 'dominated'], key=lambda i: (order[labels[i]], i))
    strong = [i for i in new if labels[i] == 'strong']
    json.dump(prior(new), open(os.path.join(a.out, 'prior-new.json'), 'w'))
    json.dump(prior(strong), open(os.path.join(a.out, 'prior-strongonly.json'), 'w'))

    sig = {i: inv[i]['previewSignature'] for i in new}
    by_sig = collections.defaultdict(list)
    for i in new:
        by_sig[sig[i]].append(i)
    human = [i for i in new if inv[i]['origin'] == 'human']
    collide = [i for i in human if labels[i] == 'strong' and any(labels[j] != 'strong' for j in by_sig[sig[i]] if j != i)]
    weak = [i for i in human if labels[i] != 'strong']
    off = [l.strip() for f in ('offprior.txt', 'heldout.txt')
           for l in open(os.path.join(REPO, 'data/team-inventory-v1/eval', f)) if l.strip()]
    null = [i for i in human if labels[i] == 'strong' and i not in collide and not by_sig[sig[i]][1:]][:8]
    groups = {'collide': collide, 'weak': weak, 'offprior': off, 'strongnull': null}
    allc = []
    for g, ys in groups.items():
        cells = [f'{x} {y}\n' for x in X_TEAMS for y in ys if x != y]
        open(os.path.join(a.out, f'cells-{g}.txt'), 'w').write(''.join(cells))
        allc += cells
        print(g, len(ys), ys)
    open(os.path.join(a.out, 'cells.txt'), 'w').write(''.join(allc))
    json.dump(groups, open(os.path.join(a.out, 'groups.json'), 'w'), indent=1)
    print('cells', len(allc))


if __name__ == '__main__':
    main()
