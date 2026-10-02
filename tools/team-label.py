#!/usr/bin/env python3
"""Apply the frozen classification protocol (data/team-inventory-v1/PROTOCOL.md)
mechanically and write data/team-inventory-v1/classification.json.

    python3 tools/team-label.py --discovery DIR --confirm DIR [--confirm DIR ...] \
        --exec-audit AUDIT.json --manual MANUAL.json --allocation ALLOC.json \
        [--out data/team-inventory-v1/classification.json] [--report OUT.json]

--manual carries the one human judgement the protocol asks for (condition
3's log read): {candidate: {"replacement": id, "logs": [...], "established":
bool, "note": str}}. Everything else is computed.
"""
import argparse
import collections
import importlib.util
import json
import os
import re

import numpy as np

REPO = os.path.join(os.path.dirname(__file__), '..')
spec = importlib.util.spec_from_file_location('tc', os.path.join(REPO, 'tools/team-classify.py'))
tc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tc)

BAR = 0.45
MATERIAL = 0.05
UNIQUE = 0.10
PICK_FLOOR = 0.10


def toid(s):
    return re.sub(r'[^a-z0-9]', '', str(s).lower())


def paired(M, x, r, panel, w, cluster, own, rng, boot):
    """(r − x) panel difference over panel teams outside the shared cluster,
    each game paired by (opponent, k, side); k-bootstrap interval."""
    def diff(ks=None):
        num = den = 0.0
        for p in panel:
            if p in (x, r) or cluster[p] == own:
                continue
            ax, ar = M.arr(x, p), M.arr(r, p)
            if ax is None or ar is None:
                continue
            d = ar - ax
            sub = d if ks is None else d[ks]
            if not np.any(~np.isnan(sub)):
                continue
            num += w[p] * np.nanmean(sub)
            den += w[p]
        return num / den if den else np.nan
    lo, hi = tc.boot_ci(diff, M.K, rng, boot)
    return diff(), (lo, hi)


def stratum_diff(M, x, r, panel, w, cluster, own, strata, st, rng, boot):
    ps = [p for p in panel if st in strata[p]]
    d, ci = paired(M, r, x, ps, {p: w[p] for p in ps}, cluster, own, rng, boot)  # x − r
    return d, ci


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--discovery', required=True)
    ap.add_argument('--confirm', action='append', default=[])
    ap.add_argument('--exec-audit', required=True)
    ap.add_argument('--manual', required=True)
    ap.add_argument('--allocation', required=True)
    ap.add_argument('--out', default=os.path.join(REPO, 'data/team-inventory-v1/classification.json'))
    ap.add_argument('--report')
    ap.add_argument('--boot', type=int, default=2000)
    a = ap.parse_args()

    inv = json.load(open(tc.INV))
    by = {t['id']: t for t in inv['teams']}
    heldout = [l.strip() for l in open(os.path.join(tc.EVAL, 'heldout.txt')) if l.strip()]
    measured = [l.strip() for l in open(os.path.join(tc.EVAL, 'measured.txt')) if l.strip()]
    teams_eval = {t['id']: t for t in json.load(open(os.path.join(tc.EVAL, 'teams.json')))['teams']}
    panel, cluster, w = tc.panel_setup(inv, heldout)
    strata = {p: tc.strata_of(teams_eval[p]['sets']) for p in panel}
    tot = sum(w.values())
    sw = collections.Counter()
    for p in panel:
        for s in strata[p]:
            sw[s] += w[p]
    important = sorted(s for s, v in sw.items() if v / tot >= 0.10)
    rng = np.random.default_rng(20261001)

    Md = tc.Matrix(tc.load_games([a.discovery])[0])
    games_c, agents_c = tc.load_games(a.confirm) if a.confirm else ({}, set())
    Mc = tc.Matrix(games_c) if games_c else None
    audit = json.load(open(a.exec_audit))
    manual = json.load(open(a.manual))

    def own_of(x):
        return cluster.get(x, by[x]['variantCluster'])

    def score(M, x):
        s = tc.panel_score(M, x, panel, w, own_of(x), cluster)
        ci = tc.boot_ci(lambda ks: tc.panel_score(M, x, panel, w, own_of(x), cluster, ks), M.K, rng, a.boot)
        return s, ci

    def confirmed(x):
        return Mc is not None and any(Mc.arr(x, p) is not None for p in panel)

    labels = {}
    for x in measured:
        ds, dci = score(Md, x)
        rec = {'discovery': {'score': ds, 'ci': dci}}
        if dci[0] > BAR:
            rec.update(label='strong', stage='discovery', score=ds, ci=list(dci))
        elif dci[1] < BAR:
            rec.update(label='weak', stage='discovery', score=ds, ci=list(dci))
        elif confirmed(x):
            cs, cci = score(Mc, x)
            rec['confirmation'] = {'score': cs, 'ci': cci, 'seeds': Mc.K}
            if cci[0] >= BAR:
                rec.update(label='strong', stage='confirmation', score=cs, ci=list(cci))
            elif cci[1] < BAR:
                rec.update(label='weak', stage='confirmation', score=cs, ci=list(cci))
            else:
                rec.update(label='pending', stage='confirmation', score=cs, ci=list(cci),
                           note=f'confirmation interval still touches {BAR} after {Mc.K} seeds')
        else:
            rec.update(label='pending', stage='discovery', score=ds, ci=list(dci), note='awaiting confirmation')
        labels[x] = rec

    # dominated: condition 1 (trusted same-cluster replacement, same level allocation)
    def alloc(t):
        return sorted((toid(s['species']), s['level']) for s in t['sets'] if s['level'] > 50)
    dom_checks = []
    for x in measured:
        for r in measured:
            if r == x:
                continue
            tx, tr = by[x], by[r]
            if tr['family'] not in tc.TRUSTED or not tx['variantCluster'] or tx['variantCluster'] != tr['variantCluster']:
                continue
            if alloc(tx) != alloc(tr):
                continue
            chk = {'candidate': x, 'replacement': r, 'replacementVersion': f"{tr['source']['name']} — {tr['source'].get('version')}"}
            if not (confirmed(x) and confirmed(r)):
                chk['result'] = 'not confirmed (no confirmation games)'
                dom_checks.append(chk)
                continue
            d, ci = paired(Mc, x, r, panel, w, cluster, own_of(x), rng, a.boot)
            chk['delta'] = d
            chk['ci'] = list(ci)
            cond2 = d >= MATERIAL and ci[0] > 0
            uniq = {}
            for st in important:
                sd, sci = stratum_diff(Mc, x, r, panel, w, cluster, own_of(x), strata, st, rng, a.boot)
                uniq[st] = {'candidateMinusReplacement': sd, 'ci': list(sci)}
            unique_adv = [st for st, v in uniq.items() if v['candidateMinusReplacement'] >= UNIQUE and v['ci'][0] > 0]
            chk['strata'] = uniq
            chk['uniqueAdvantage'] = unique_adv
            # condition 3: distinguishing elements selected and used
            dist = []
            sx = {toid(s['species']): s for s in tx['sets']}
            sr = {toid(s['species']): s for s in tr['sets']}
            for sp, s in sx.items():
                if sp not in sr or toid(s.get('item', '')) != toid(sr[sp].get('item', '')) or \
                        sorted(map(toid, s['moves'])) != sorted(map(toid, sr[sp]['moves'])):
                    dist.append(s['species'])
            ex = audit.get(x, {}).get('mons', {})
            exec_rows = {sp: {'pickRate': ex.get(sp, {}).get('pickRate'),
                              'moveUseRate': ex.get(sp, {}).get('moveUseRate')} for sp in dist}
            picks_ok = all((v['pickRate'] or 0) >= PICK_FLOOR for v in exec_rows.values())
            man = manual.get(x)
            man_ok = bool(man and man.get('replacement') == r and man.get('established'))
            chk['execution'] = {'distinguishing': exec_rows, 'selectionFloorMet': picks_ok, 'logRead': man}
            if not cond2:
                chk['result'] = 'condition 2 not met'
            elif unique_adv:
                chk['result'] = f'unique advantage in {unique_adv}'
            elif not picks_ok:
                chk['result'] = 'withheld: a distinguishing mon is selected in <10% of games'
            elif not man_ok:
                chk['result'] = 'withheld: execution log read not established'
            else:
                chk['result'] = 'dominated'
            dom_checks.append(chk)
    for x in measured:
        hits = [c for c in dom_checks if c['candidate'] == x and c['result'] == 'dominated']
        if hits:
            best = max(hits, key=lambda c: c['delta'])
            labels[x].update(label='dominated', replacement={
                'id': best['replacement'], 'version': best['replacementVersion'],
                'delta': best['delta'], 'ci': best['ci'], 'panel': 'PROTOCOL.md v1 human panel',
                'conditions': 'open vs open, 1000 iterations, confirmation seeds 20261001'},
                execution=best['execution'])
    # a replacement must not itself be dominated
    for x, l in labels.items():
        if l['label'] == 'dominated' and labels[l['replacement']['id']]['label'] == 'dominated':
            l.update(label='pending', note='replacement is itself dominated')

    alloc_cfg = json.load(open(a.allocation))
    out = {
        'protocol': 'data/team-inventory-v1/PROTOCOL.md (v1)',
        'evidence': {'discovery': a.discovery, 'confirmation': a.confirm},
        'allocation': alloc_cfg,
        'labels': {x: {k: v for k, v in l.items()} for x, l in labels.items()},
    }
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    json.dump(out, open(a.out, 'w'), indent=1, default=float)
    if a.report:
        json.dump({'dominatedChecks': dom_checks, 'importantStrata': important}, open(a.report, 'w'), indent=1, default=float)
    cnt = collections.Counter(l['label'] for l in labels.values())
    print(dict(cnt))
    for c in dom_checks:
        print(f"  {c['candidate']:26s} <- {c['replacement']:26s} {c.get('delta', float('nan')):+.3f} {c['result']}")


if __name__ == '__main__':
    main()
