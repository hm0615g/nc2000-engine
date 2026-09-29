// Generate every shipped team artifact from the one classified inventory
// (docs/TEAM-POOL-REBUILD-PLAN.md step 5):
//
//   data/team-pool-v1/team-pool.json       ordinary own-team pool: strong
//                                          teams (Nash members included)
//   data/belief-pool-v2/belief-pool.json   opponent prior: strong + weak +
//                                          pending, weighted
//   data/meta-nash-v2/pool-artifact.json   Nash support and weights
//   data/team-inventory-v1/reference.json  every record: eligibility, label,
//                                          replacement links, memberships
//
//   node tools/build-team-products.js
//
// Inputs: data/team-inventory-v1/inventory.json, classification.json,
// data/meta-nash-v2/solution.json. Labels are never decided here.
'use strict';
const fs = require('fs');
const path = require('path');

const REPO = path.join(__dirname, '..');
const read = rel => JSON.parse(fs.readFileSync(path.join(REPO, rel), 'utf8'));
const write = (rel, obj) => {
	fs.mkdirSync(path.dirname(path.join(REPO, rel)), { recursive: true });
	fs.writeFileSync(path.join(REPO, rel), JSON.stringify(obj, null, 1) + '\n');
};

const inv = read('data/team-inventory-v1/inventory.json');
const cls = read('data/team-inventory-v1/classification.json');
const sol = read('data/meta-nash-v2/solution.json');
const byId = new Map(inv.teams.map(t => [t.id, t]));

// Machine-generated teams are kept in the prior for identification, but a
// human opponent is far less likely to bring one than a published team
// (step 3 calibration, data/belief-pool-v2/README.md).
const MACHINE_PRIOR_FACTOR = cls.allocation.machineFactor;

const LABELS = new Set(['strong', 'weak', 'dominated', 'pending']);
const label = id => {
	const t = byId.get(id);
	const own = cls.labels[t.measuredAs];
	if (!own) throw new Error(`${id}: no label for its measured representative ${t.measuredAs}`);
	if (!LABELS.has(own.label)) throw new Error(`${id}: unknown label ${own.label}`);
	return own;
};

const eligible = inv.teams.filter(t => t.eligibility.status === 'eligible');
const measured = eligible.filter(t => t.measuredAs === t.id);
for (const t of measured) label(t.id);
for (const [id, l] of Object.entries(cls.labels)) {
	if (!byId.has(id) || byId.get(id).measuredAs !== id) throw new Error(`label for non-measured id ${id}`);
	if (l.label === 'dominated') {
		const r = l.replacement;
		if (!r || !byId.has(r.id) || !r.version || r.delta == null || !r.ci || !l.execution) {
			throw new Error(`${id}: a dominated verdict must name its replacement, version, delta, interval and execution evidence`);
		}
		if (label(r.id).label === 'dominated') throw new Error(`${id}: replacement ${r.id} is itself dominated`);
	}
}

const provenance = t => ({ family: t.family, origin: t.origin, source: t.source.name, url: t.source.url, version: t.source.version || null });
const clusterWeights = (teams, keyOf) => {
	const size = new Map();
	for (const t of teams) size.set(keyOf(t), (size.get(keyOf(t)) || 0) + 1);
	return t => 1 / size.get(keyOf(t));
};
const clusterKey = t => t.variantCluster || t.id;
const round = x => Math.round(x * 1e6) / 1e6;

// ------------------------------------------------------------ own pool
const strong = measured.filter(t => label(t.id).label === 'strong');
const byScore = (a, b) => (label(b.id).score ?? 0) - (label(a.id).score ?? 0) || a.id.localeCompare(b.id);
strong.sort(byScore);
const drawW = clusterWeights(strong, clusterKey);
const drawTotal = strong.reduce((s, t) => s + drawW(t), 0);
write('data/team-pool-v1/team-pool.json', {
	format: 'nc2000-team-pool-v1',
	generator: 'tools/build-team-products.js',
	protocol: cls.protocol,
	drawRule: 'each variant cluster among the strong teams is drawn with equal probability, split equally among its members (drawWeight sums to 1)',
	teams: strong.map((t, i) => ({
		id: t.id, rank: i + 1, label: 'strong',
		species: t.species, levels: t.levels,
		drawWeight: round(drawW(t) / drawTotal),
		provenance: provenance(t),
		sets: t.sets,
	})),
});

// ------------------------------------------------------------ prior
const inPrior = measured.filter(t => label(t.id).label !== 'dominated');
inPrior.sort((a, b) => {
	const order = { strong: 0, pending: 1, weak: 2 };
	return order[label(a.id).label] - order[label(b.id).label] || byScore(a, b);
});
const priorW = clusterWeights(inPrior, clusterKey);
write('data/belief-pool-v2/belief-pool.json', {
	format: 'nc2000-belief-pool',
	version: 2,
	generator: 'tools/build-team-products.js',
	protocol: cls.protocol,
	note: 'Opponent prior: every eligible, non-dominated measured team (strong, weak and pending) from data/team-inventory-v1. '
		+ 'weight = 1/(members of its variant cluster in the prior), x' + MACHINE_PRIOR_FACTOR + ' for machine-generated teams; '
		+ 'the belief samples preview-consistent candidates in proportion to weight and falls back to the weighted-mode loadout per species '
		+ '(crates/bot/src/belief.rs). See data/belief-pool-v2/README.md.',
	teams: inPrior.map(t => ({
		id: t.id,
		label: label(t.id).label,
		weight: round(priorW(t) * (t.origin === 'machine' ? MACHINE_PRIOR_FACTOR : 1)),
		provenance: provenance(t),
		sets: t.sets,
	})),
});

// ------------------------------------------------------------ nash
const nashTeams = Object.entries(sol.weights)
	.filter(([, w]) => w > 0)
	.sort((a, b) => b[1] - a[1]);
for (const [id] of nashTeams) {
	if (!byId.has(id) || label(id).label !== 'strong') throw new Error(`Nash support ${id} is not a strong team`);
}
write('data/meta-nash-v2/pool-artifact.json', {
	format: 'meta-nash-pool-v2',
	source_solution: sol.source,
	engine_commit: sol.engine_commit,
	conditions: sol.conditions,
	teams: nashTeams.map(([id, w]) => ({ id, origin: byId.get(id).family, weight: w, sets: byId.get(id).sets })),
});

// ------------------------------------------------------------ reference
const nashW = new Map(nashTeams);
write('data/team-inventory-v1/reference.json', {
	format: 'nc2000-team-reference-v1',
	generator: 'tools/build-team-products.js',
	protocol: cls.protocol,
	counts: {
		records: inv.teams.length,
		eligible: eligible.length,
		measured: measured.length,
		...Object.fromEntries(['strong', 'weak', 'pending', 'dominated'].map(k => [k, measured.filter(t => label(t.id).label === k).length])),
		nashSupport: nashTeams.length,
	},
	teams: inv.teams.map(t => {
		const eligibleT = t.eligibility.status === 'eligible';
		const l = eligibleT ? label(t.id) : null;
		return {
			id: t.id, family: t.family, origin: t.origin, source: t.source,
			eligibility: t.eligibility,
			label: l ? l.label : null,
			measuredAs: t.measuredAs,
			evidence: l ? { score: l.score, ci: l.ci, stage: l.stage, note: l.note || null } : null,
			replacement: l && l.label === 'dominated' ? l.replacement : null,
			execution: l && l.label === 'dominated' ? l.execution : null,
			ordinaryDraw: !!l && l.label === 'strong' && t.measuredAs === t.id,
			opponentPrior: !!l && l.label !== 'dominated' && t.measuredAs === t.id,
			nashWeight: nashW.get(t.id) || 0,
			variantCluster: t.variantCluster,
			species: t.species, levels: t.levels,
		};
	}),
});

console.log(`own pool ${strong.length} | prior ${inPrior.length} | nash ${nashTeams.length} | reference ${inv.teams.length}`);
