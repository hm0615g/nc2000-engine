#!/usr/bin/env python3
"""Own-pool selection under protocol v2 (data/team-selection-v2/PROTOCOL.md).

    python3 tools/team-select.py panel                        # panel.json (frozen)
    python3 tools/team-select.py cells-a --out FILE           # stage A cells
    python3 tools/team-select.py cells --ids FILE --out FILE  # ids x panel cells
    python3 tools/team-select.py calibrate --cells DIR        # stage A calibration only
    python3 tools/team-select.py score --cells DIR... --ids FILE [--json OUT]
    python3 tools/team-select.py shortlist --cells DIR --out FILE [--json OUT]
    python3 tools/team-select.py disc --cells DIR --ids FILE --out FILE [--json OUT]
    python3 tools/team-select.py confirm --cells DIR --ids FILE --look N [--json OUT]
    python3 tools/team-select.py sample-pool --pool FILE --measured IDS --seed S --out PREFIX
    python3 tools/team-select.py sample-panel --ids FILE --seed S --out PREFIX
    python3 tools/team-select.py sampled --manifest PREFIX.json --cells DIR [--json OUT]
    python3 tools/team-select.py finalize --a A.json --b B.json --c C.json --out selection.json

Reads team_eval cell JSONL. Every directory passed in one call must hold a
single agent condition (`agent` + `cond`); mixing is refused. A game is
identified by (p1 team, p2 team, k), so a pair written in both orientations
is counted once.
"""
import argparse
import collections
import glob
import hashlib
import json
import math
import os
import random
from statistics import NormalDist

REPO = os.path.join(os.path.dirname(__file__), '..')
INV = os.path.join(REPO, 'data/team-inventory-v1/inventory.json')
EVAL = os.path.join(REPO, 'data/team-inventory-v1/eval')
DIR = os.path.join(REPO, 'data/team-selection-v2')
PANEL = os.path.join(DIR, 'panel.json')
CONTROLS = ['ctl-floor-s07', 'ctl-del-s07-gene', 'ctl-del-s07-boom', 'ctl-conf-s07-order']
CEILING = 'sample-07'
Z95 = NormalDist().inv_cdf(0.975)
Z_LOOK = NormalDist().inv_cdf(1 - 0.025 / 2)
SHORTLIST_TOP = 40
AUDIT_N = 6
AUDIT_SEED = 20261112
DISC_MIN = 0.52
DISC_CAP = 36
FLOOR = 0.50
SIZE_MIN, SIZE_MAX = 15, 30
ALWAYS_B = ['sample-16', 'rental-cban-13', 'sample-16-orig', 'mjj-2020-kb-2020', 'mjj-2020-kb-2022']
SPECIALIST_FILES = ['data/meta-nash-v2/solution-blind3000-do2.json', 'data/meta-nash-v2/solution.json']
SPECIALIST_V1 = ['sample-07', 'sample-08', 'sample-10']


def read_ids(path):
    return [l.strip() for l in open(path) if l.strip() and not l.startswith('#')]


def inventory():
    return {t['id']: t for t in json.load(open(INV))['teams']}


def toid(s):
    return ''.join(c for c in str(s).lower() if c.isalnum())


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
    return sorted(out)


# ------------------------------------------------------------------ panel

def build_panel():
    inv = inventory()
    ids = [t['id'] for t in inv.values()
           if t['eligibility']['status'] == 'eligible' and t['measuredAs'] == t['id'] and t['origin'] == 'human']
    ids.sort()
    cluster = {i: inv[i]['variantCluster'] or f'solo:{i}' for i in ids}
    size = collections.Counter(cluster.values())
    n_clusters = len(size)
    members = [{'id': i, 'family': inv[i]['family'], 'cluster': cluster[i],
                'weight': 1 / (n_clusters * size[cluster[i]]), 'strata': strata_of(inv[i]['sets'])} for i in ids]
    sw = collections.Counter()
    for m in members:
        for s in m['strata']:
            sw[s] += m['weight']
    candidates = sorted(t['id'] for t in inv.values()
                        if t['eligibility']['status'] == 'eligible' and t['measuredAs'] == t['id'])
    out = {
        'format': 'nc2000-selection-panel-v2',
        'generator': 'tools/team-select.py panel',
        'rule': 'every eligible measured human-source record of data/team-inventory-v1; each variant cluster '
                'carries 1/(clusters), split equally among its members',
        'clusters': n_clusters,
        'strataWeight': {k: round(v, 6) for k, v in sorted(sw.items())},
        'importantStrata': sorted(k for k, v in sw.items() if v >= 0.10),
        'members': members,
        'candidates': candidates,
        'challenge': read_ids(os.path.join(EVAL, 'heldout.txt')) + read_ids(os.path.join(EVAL, 'offprior.txt')),
    }
    os.makedirs(DIR, exist_ok=True)
    with open(PANEL, 'w') as f:
        f.write(json.dumps(out, indent=1) + '\n')
    print(f'{PANEL}: {len(members)} members in {n_clusters} clusters, {len(candidates)} candidates')


def load_panel():
    p = json.load(open(PANEL))
    return p, {m['id']: m['weight'] for m in p['members']}


def pairs_cells(rows, cols):
    """Unordered pairs {r, c}, r != c, each written once as `min max`."""
    seen = set()
    out = []
    for r in rows:
        for c in cols:
            if r == c:
                continue
            key = tuple(sorted((r, c)))
            if key not in seen:
                seen.add(key)
                out.append(f'{key[0]} {key[1]}\n')
    return out


# ------------------------------------------------------------------ games

class Games:
    """Per unordered pair: {(p1, p2, k): score of p1} plus result tags."""

    def __init__(self, dirs):
        self.g = {}
        self.res = {}
        conds = set()
        for d in dirs:
            files = glob.glob(os.path.join(d, 'cell-*.jsonl'))
            if not files:
                raise SystemExit(f'{d}: no cells')
            for f in files:
                row, col = os.path.basename(f)[5:-6].split('__')
                for line in open(f):
                    r = json.loads(line)
                    conds.add(json.dumps([r['agent'], r.get('cond'), r.get('forced')], sort_keys=True))
                    p1, p2 = (row, col) if r['row_p1'] else (col, row)
                    s1 = r['score'] if r['row_p1'] else 1.0 - r['score']
                    self.g[(p1, p2, r['k'])] = s1
                    self.res[(p1, p2, r['k'])] = r['result']
        if len(conds) != 1:
            raise SystemExit(f'refusing to mix {len(conds)} conditions across {dirs}: {sorted(conds)}')
        self.cond = json.loads(conds.pop())
        self.by_pair = collections.defaultdict(list)
        for (p1, p2, k), s in self.g.items():
            self.by_pair[(p1, p2)].append((k, s))

    def cell(self, t, p):
        """t's game scores vs p as {(k, t_is_p1): score}."""
        out = {}
        for k, s in self.by_pair.get((t, p), []):
            out[(k, True)] = s
        for k, s in self.by_pair.get((p, t), []):
            out[(k, False)] = 1.0 - s
        return out

    def results(self, t):
        c = collections.Counter()
        for (p1, p2, k), r in self.res.items():
            if t in (p1, p2):
                c[r] += 1
        return c


def pooled_var(cells):
    num = den = 0.0
    for xs in cells:
        if len(xs) >= 2:
            m = sum(xs) / len(xs)
            num += sum((x - m) ** 2 for x in xs)
            den += len(xs) - 1
    return (num / den if den else float('nan')), den


def score(G, t, weights, members=None):
    """Panel score, its SE (stratified, pooled within-opponent game
    variance), games, and per-opponent means."""
    members = members or weights
    tot = sum(weights[p] for p in members)
    mean = 0.0
    cells = {}
    for p in members:
        w = weights[p] / tot
        if p == t:
            mean += w * 0.5
            continue
        c = G.cell(t, p)
        if not c:
            raise SystemExit(f'missing cell {t} vs {p}')
        cells[p] = c
        mean += w * sum(c.values()) / len(c)
    var_g, df = pooled_var([list(c.values()) for c in cells.values()])
    var = sum((weights[p] / tot) ** 2 * var_g / len(c) for p, c in cells.items())
    return {'score': mean, 'se': math.sqrt(var), 'games': sum(len(c) for c in cells.values()), 'df': df,
            'per': {p: sum(c.values()) / len(c) for p, c in cells.items()}}


def diff(G, x, y, weights):
    """Paired x - y over shared (opponent, k, side) games."""
    tot = sum(weights.values())
    mean = 0.0
    ds = []
    ns = {}
    for p in weights:
        w = weights[p] / tot
        cx = {'self': 0.5} if p == x else G.cell(x, p)
        cy = {'self': 0.5} if p == y else G.cell(y, p)
        if 'self' in cx or 'self' in cy:
            other = cy if 'self' in cx else cx
            sign = -1 if 'self' in cx else 1
            d = [sign * (v - 0.5) for v in other.values()]
        else:
            keys = sorted(set(cx) & set(cy))
            if not keys:
                raise SystemExit(f'no shared games for {x}/{y} vs {p}')
            d = [cx[k] - cy[k] for k in keys]
        ds.append(d)
        ns[p] = len(d)
        mean += w * sum(d) / len(d)
    var_d, _ = pooled_var(ds)
    var = sum((weights[p] / tot) ** 2 * var_d / ns[p] for p in weights)
    return mean, math.sqrt(var)


# ------------------------------------------------------------------ pools

def draw_weights(S, inv):
    key = lambda t: inv[t]['variantCluster'] or f'solo:{t}'
    size = collections.Counter(key(t) for t in S)
    nc = len(size)
    return {t: 1 / (nc * size[key(t)]) for t in S}


def pool_value(S, est, inv):
    q = draw_weights(S, inv)
    v = sum(q[t] * est[t]['score'] for t in S)
    se = math.sqrt(sum((q[t] * est[t]['se']) ** 2 for t in S))
    return v, se


def deletion(t, S, est, inv):
    """Delta_remove(t, S) = V(S without t) - V(S), linear in the scores."""
    q = draw_weights(S, inv)
    rest = [u for u in S if u != t]
    q2 = draw_weights(rest, inv)
    coef = {u: q2.get(u, 0.0) - q[u] for u in S}
    d = sum(c * est[u]['score'] for u, c in coef.items())
    se = math.sqrt(sum((c * est[u]['se']) ** 2 for u, c in coef.items()))
    return d, se


def z_simultaneous(m):
    return NormalDist().inv_cdf(1 - 0.025 / max(m, 1))


def harm_trim(S, est, inv, log):
    """Remove clearly harmful members one at a time: the deletion effect's
    simultaneous (Bonferroni over |S|) lower bound is above 0."""
    S = list(S)
    while len(S) > 1:
        z = z_simultaneous(len(S))
        worst = None
        for t in S:
            d, se = deletion(t, S, est, inv)
            if d - z * se > 0 and (worst is None or (d / se) > worst[1]):
                worst = (t, d / se, d, se)
        if worst is None:
            break
        t, zt, d, se = worst
        V, _ = pool_value(S, est, inv)
        log.append({'removed': t, 'delta': d, 'se': se, 'z': zt, 'zRequired': z, 'size': len(S), 'V': V})
        S.remove(t)
    return S


# ------------------------------------------------------------------ main

def calibration(G, weights):
    rows = {x: score(G, x, weights) for x in [CEILING] + CONTROLS}
    half = lambda r: Z95 * r['se']
    ceil, conf = rows[CEILING], rows['ctl-conf-s07-order']
    noise = max(half(ceil), half(conf), abs(conf['score'] - ceil['score']))
    brk = min(ceil['score'] - rows['ctl-floor-s07']['score'], ceil['score'] - rows['ctl-del-s07-boom']['score'])
    stuck = ceil['score'] - rows['ctl-del-s07-boom']['score'] <= half(ceil)
    verdict = ('INVALID_STUCK_FLOOR' if stuck else 'VALID' if brk >= 2 * noise
               else 'WEAK' if brk > noise else 'INVALID_NOISY')
    return {'noise': noise, 'break': brk, 'verdict': verdict,
            'resolution': 2 * Z95 * ceil['se'],
            'geneDeletion': ceil['score'] - rows['ctl-del-s07-gene']['score'],
            'arms': {k: {x: v for x, v in r.items() if x != 'per'} for k, r in rows.items()}}


def rows_for(G, ids, weights, panel, inv):
    strata = {m['id']: m['strata'] for m in panel['members']}
    out = {}
    for t in ids:
        r = score(G, t, weights)
        per_stratum = {}
        for st in panel['importantStrata']:
            ps = {p: w for p, w in weights.items() if st in strata[p]}
            per_stratum[st] = score(G, t, ps)['score']
        res = G.results(t)
        out[t] = {'score': r['score'], 'se': r['se'], 'ci': [r['score'] - Z95 * r['se'], r['score'] + Z95 * r['se']],
                  'games': r['games'], 'caps': res.get('cap', 0), 'ties': res.get('tie', 0),
                  'strata': per_stratum, 'family': inv[t]['family'] if t in inv else None}
    return out


BLOCKS_POOL = 240
BLOCKS_PANEL = 32


def weighted_choice(rng, items, weights):
    r = rng.random() * sum(weights)
    for it, w in zip(items, weights):
        if r < w:
            return it
        r -= w
    return items[-1]


def write_blocks(prefix, blocks, meta):
    """blocks: [(team, opponent, k)]. Self blocks are recorded in the
    manifest (scored 0.5) and not played."""
    lines = [f'{t} {p} {k} {k + 1}\n' for t, p, k in blocks if t != p]
    open(prefix + '.cells', 'w').write(''.join(lines))
    json.dump({**meta, 'blocks': [{'team': t, 'opponent': p, 'k': k} for t, p, k in blocks]},
              open(prefix + '.json', 'w'), indent=1)
    print(f'{prefix}.cells: {len(lines)} played blocks, {len(blocks) - len(lines)} self blocks')


def sample_pool(pool_file, measured_ids, seed, prefix, panel_w):
    """PPS blocks for the members of a fixed old pool that have no 27k
    score: team by its draw weight restricted to them, opponent by panel
    weight, one fresh seed index per block."""
    pool = json.load(open(pool_file))
    measured = set(read_ids(measured_ids))
    key = 'drawWeight' if all('drawWeight' in t for t in pool['teams']) else None
    q = {t['id']: (t[key] if key else 1.0) for t in pool['teams']}
    tot = sum(q.values())
    q = {t: w / tot for t, w in q.items()}
    rest = sorted(t for t in q if t not in measured)
    rng = random.Random(seed)
    opps = sorted(panel_w)
    blocks = [(weighted_choice(rng, rest, [q[t] for t in rest]),
               weighted_choice(rng, opps, [panel_w[p] for p in opps]), j) for j in range(BLOCKS_POOL)]
    write_blocks(prefix, blocks, {'what': 'old-pool sample', 'pool': pool_file, 'q': q, 'unmeasured': rest,
                                  'unmeasuredMass': sum(q[t] for t in rest), 'seed': seed})


def sample_panel(ids, seed, prefix, panel_w):
    """One panel schedule shared by every candidate: BLOCKS_PANEL
    (opponent, k) draws by panel weight."""
    rng = random.Random(seed)
    opps = sorted(panel_w)
    sched = [(weighted_choice(rng, opps, [panel_w[p] for p in opps]), j) for j in range(BLOCKS_PANEL)]
    blocks = [(t, p, k) for t in read_ids(ids) for p, k in sched]
    write_blocks(prefix, blocks, {'what': 'shared panel schedule', 'schedule': sched, 'seed': seed})


def sampled(manifest, G):
    """Mean block score per team over a sampled design (self blocks 0.5)."""
    m = json.load(open(manifest))
    per = collections.defaultdict(list)
    for b in m['blocks']:
        t, p, k = b['team'], b['opponent'], b['k']
        if t == p:
            per[t].append(0.5)
            continue
        c = G.cell(t, p)
        g = [c[(k, side)] for side in (True, False) if (k, side) in c]
        if len(g) != 2:
            raise SystemExit(f'block {t} vs {p} k={k}: {len(g)} of 2 games')
        per[t].append(sum(g) / 2)
    out = {}
    for t, xs in per.items():
        n = len(xs)
        mean = sum(xs) / n
        sd = math.sqrt(sum((x - mean) ** 2 for x in xs) / (n - 1)) if n > 1 else float('nan')
        out[t] = {'score': mean, 'se': sd / math.sqrt(n), 'blocks': n}
    allx = [x for xs in per.values() for x in xs]
    mean = sum(allx) / len(allx)
    sd = math.sqrt(sum((x - mean) ** 2 for x in allx) / (len(allx) - 1))
    return out, {'score': mean, 'se': sd / math.sqrt(len(allx)), 'blocks': len(allx)}


def fnv1a64(path):
    h = 0xcbf29ce484222325
    for b in open(path, 'rb').read():
        h = ((h ^ b) * 0x100000001b3) & 0xffffffffffffffff
    return f'fnv1a64:{h:016x}'


def finalize(a_json, b_json, c_json, out):
    """Base labels with a reason for each, from the stage reports: stage A
    `shortlist --json`, stage B `disc --json`, the last stage C
    `confirm --json`."""
    A, B, C = (json.load(open(f)) for f in (a_json, b_json, c_json))
    panel, _ = load_panel()
    cond = C['condition']
    prior_path = os.path.join(REPO, 'data/belief-pool-v3/belief-pool.json')
    fp = fnv1a64(prior_path)
    if cond[1]['belief_row'] != fp or cond[1]['belief_col'] != fp:
        raise SystemExit('stage C was not measured on the frozen prior')
    rankA = {t: i + 1 for i, t in enumerate(sorted(panel['candidates'], key=lambda t: -A['rows'][t]['score']))}
    disc = B['disc']
    conf = C['confirm']
    S = conf['S']
    trimmed_c = {x['removed']: x for x in conf['trim']}
    trimmed_b = {x['removed']: x for x in disc['trim']}
    labels = {}
    for t in panel['candidates']:
        if t in S:
            r = C['rows'][t]
            labels[t] = {'label': 'selected', 'stage': 'C', 'score': r['score'], 'se': r['se'], 'games': r['games']}
            continue
        if t in conf['decision']:
            r = C['rows'][t]
            d = conf['decision'][t]
            base = {'stage': 'C', 'score': r['score'], 'se': r['se'], 'games': r['games']}
            if d == 'straddle':
                labels[t] = {**base, 'label': 'pending', 'reason': 'stage C: floor undecided after the last look'}
            elif d == 'fail':
                labels[t] = {**base, 'label': 'not-selected', 'reason': 'stage C: upper bound below the 0.50 floor'}
            elif t in trimmed_c:
                x = trimmed_c[t]
                labels[t] = {**base, 'label': 'not-selected',
                             'reason': f"stage C: harmful, deletion effect {x['delta']:+.4f} (z {x['z']:.2f} >= {x['zRequired']:.2f})"}
            else:
                labels[t] = {**base, 'label': 'not-selected', 'reason': 'stage C: beyond the 30-party size cap'}
            continue
        if t in B['rows']:
            r = B['rows'][t]
            base = {'stage': 'B', 'score': r['score'], 'se': r['se'], 'games': r['games']}
            if t in trimmed_b:
                reason = f"stage B: harmful on discovery data, deletion effect {trimmed_b[t]['delta']:+.4f}"
            elif r['score'] < DISC_MIN:
                reason = f"stage B: score {r['score']:.3f} below the {DISC_MIN} entrant bar"
            else:
                reason = f'stage B: beyond the {DISC_CAP}-entrant cap'
            labels[t] = {**base, 'label': 'not-selected', 'reason': reason}
            continue
        r = A['rows'][t]
        labels[t] = {'label': 'not-selected', 'stage': 'A', 'score': r['score'], 'se': r['se'], 'games': r['games'],
                     'reason': f"stage A: not shortlisted (rank {rankA[t]} of {len(rankA)} at {A['condition'][0].split('|')[0]})"}
    doc = {
        'format': 'nc2000-team-selection-v2',
        'generator': 'tools/team-select.py finalize',
        'protocol': 'data/team-selection-v2/PROTOCOL.md',
        'panel': 'data/team-selection-v2/panel.json',
        'condition': {'agent': cond[0], 'cond': cond[1]},
        'prior': {'path': 'data/belief-pool-v3/belief-pool.json', 'fingerprint': fp},
        'drawRule': 'each variant cluster among the selected parties carries equal mass, split equally among its members',
        'size': len(S), 'sizeTarget': [SIZE_MIN, SIZE_MAX],
        'V': C.get('V'),
        'deletion': C.get('deletion'),
        'S': S,
        'labels': labels,
    }
    with open(out, 'w') as f:
        f.write(json.dumps(doc, indent=1, ensure_ascii=False) + '\n')
    cnt = collections.Counter(l['label'] for l in labels.values())
    print(f'{out}: {dict(cnt)}')


def specialists():
    s = set(SPECIALIST_V1)
    for f in SPECIALIST_FILES:
        s |= {k for k, v in json.load(open(os.path.join(REPO, f)))['weights'].items() if v > 0}
    return sorted(s)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('cmd')
    ap.add_argument('--cells', action='append', default=[])
    ap.add_argument('--ids')
    ap.add_argument('--out')
    ap.add_argument('--json')
    ap.add_argument('--look', type=int)
    ap.add_argument('--pool')
    ap.add_argument('--measured')
    ap.add_argument('--seed', type=int)
    ap.add_argument('--manifest')
    ap.add_argument('--a')
    ap.add_argument('--b')
    ap.add_argument('--c')
    a = ap.parse_args()
    inv = inventory()
    if a.cmd == 'panel':
        return build_panel()
    panel, weights = load_panel()
    members = [m['id'] for m in panel['members']]
    cands = panel['candidates']
    if a.cmd == 'cells-a':
        machine = [c for c in cands if c not in weights]
        cells = pairs_cells(members, members) + pairs_cells(machine + CONTROLS, members)
        open(a.out, 'w').write(''.join(cells))
        print(f'{a.out}: {len(cells)} cells')
        return
    if a.cmd == 'cells':
        cells = pairs_cells(read_ids(a.ids), members)
        open(a.out, 'w').write(''.join(cells))
        print(f'{a.out}: {len(cells)} cells')
        return
    if a.cmd == 'finalize':
        return finalize(a.a, a.b, a.c, a.out)
    if a.cmd == 'sample-pool':
        return sample_pool(a.pool, a.measured, a.seed, a.out, weights)
    if a.cmd == 'sample-panel':
        return sample_panel(a.ids, a.seed, a.out, weights)
    if a.cmd == 'sampled':
        G = Games(a.cells)
        per, pooled = sampled(a.manifest, G)
        print('condition:', G.cond)
        print(f"pooled {pooled['score']:.3f} +- {Z95 * pooled['se']:.3f} ({pooled['blocks']} blocks)")
        for t in sorted(per, key=lambda t: -per[t]['score']):
            r = per[t]
            print(f"{t:34s} {r['score']:.3f} +- {Z95 * r['se']:.3f} ({r['blocks']} blocks)")
        if a.json:
            json.dump({'condition': G.cond, 'manifest': a.manifest, 'pooled': pooled, 'teams': per},
                      open(a.json, 'w'), indent=1)
        return
    G = Games(a.cells)
    print('condition:', G.cond)
    if a.cmd == 'calibrate':
        c = calibration(G, weights)
        print(json.dumps({k: v for k, v in c.items() if k != 'arms'}, indent=1))
        for k, r in c['arms'].items():
            print(f"  {k:22s} {r['score']:.3f} +- {Z95 * r['se']:.3f} ({r['games']} games)")
        if a.json:
            json.dump({'condition': G.cond, 'calibration': c}, open(a.json, 'w'), indent=1)
        return
    ids = read_ids(a.ids) if a.ids else cands
    rows = rows_for(G, ids, weights, panel, inv)
    report = {'condition': G.cond, 'panel': os.path.relpath(PANEL, REPO), 'rows': rows}
    if a.cmd == 'score':
        pass
    elif a.cmd == 'shortlist':
        report['calibration'] = calibration(G, weights)
        ranked = sorted(cands, key=lambda t: -rows[t]['score'])
        top = ranked[:SHORTLIST_TOP]
        spec = [t for t in specialists() if t in rows]
        pick = sorted(set(top) | set(spec) | set(ALWAYS_B))
        rest = sorted(t for t in cands if t not in pick)
        audit = sorted(random.Random(AUDIT_SEED).sample(rest, min(AUDIT_N, len(rest))))
        reasons = {t: [r for r, s in (('top', top), ('specialist', spec), ('always', ALWAYS_B), ('audit', audit)) if t in s]
                   for t in sorted(set(pick) | set(audit))}
        report['shortlist'] = reasons
        report['belowShortlist'] = [t for t in ranked if t not in reasons]
        open(a.out, 'w').write(''.join(f'{t}\n' for t in sorted(reasons)))
        print(f'shortlist {len(reasons)} -> {a.out}')
    elif a.cmd == 'disc':
        ranked = sorted(ids, key=lambda t: -rows[t]['score'])
        pool = [t for t in ranked if rows[t]['score'] >= DISC_MIN][:DISC_CAP]
        log = []
        S = harm_trim(pool, rows, inv, log)
        report['disc'] = {'entrants': pool, 'trim': log, 'S': S}
        report['V'] = pool_value(S, rows, inv)
        open(a.out, 'w').write(''.join(f'{t}\n' for t in S))
        print(f'S_disc {len(S)} (entrants {len(pool)}, trimmed {len(log)}) -> {a.out}')
    elif a.cmd == 'confirm':
        z = Z_LOOK
        dec = {}
        for t in ids:
            r = rows[t]
            lo, hi = r['score'] - z * r['se'], r['score'] + z * r['se']
            dec[t] = 'pass' if lo > FLOOR else 'fail' if hi < FLOOR else 'straddle'
            r['lookBounds'] = [lo, hi]
        passed = [t for t in ids if dec[t] == 'pass']
        log = []
        S = harm_trim(passed, rows, inv, log)
        cut = []
        if len(S) > SIZE_MAX:
            ranked = sorted(S, key=lambda t: -rows[t]['score'])
            S, over = ranked[:SIZE_MAX], ranked[SIZE_MAX:]
            edge = rows[S[-1]]['score']
            cut = [{'id': t, 'score': rows[t]['score'], 'uncertainAtCut': rows[t]['ci'][1] >= edge} for t in over]
        report['confirm'] = {'look': a.look, 'z': z, 'decision': dec, 'trim': log, 'S': S, 'sizeCut': cut,
                             'size': len(S), 'sizeTarget': [SIZE_MIN, SIZE_MAX],
                             'shortfall': len(S) < SIZE_MIN}
        if S:
            report['V'] = pool_value(S, rows, inv)
            report['deletion'] = {t: deletion(t, S, rows, inv) for t in S}
        if a.out:
            open(a.out, 'w').write(''.join(f'{t}\n' for t in S))
        print(f"look {a.look}: pass {len(passed)} fail {sum(v == 'fail' for v in dec.values())} "
              f"straddle {sum(v == 'straddle' for v in dec.values())}; after harm test {len(S)}")
    else:
        raise SystemExit(f'unknown command {a.cmd}')
    for t in sorted(ids, key=lambda t: -rows[t]['score']):
        r = rows[t]
        print(f"{t:34s} {str(r['family'])[:14]:14s} {r['score']:.3f} [{r['ci'][0]:.3f},{r['ci'][1]:.3f}] "
              f"n={r['games']:4d} caps={r['caps']}")
    if a.json:
        json.dump(report, open(a.json, 'w'), indent=1)


if __name__ == '__main__':
    main()
