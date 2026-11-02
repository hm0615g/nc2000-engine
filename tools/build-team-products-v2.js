// Generate the shipped team files of the blind rebuild
// (docs/TEAM-POOL-REBUILD-PLAN.md step 6) from explicit versioned inputs:
//
//   data/team-pool-v2/team-pool.json          the catalog: selected parties —
//                                             human built-in choices and the
//                                             bot's ordinary draw (drawWeight)
//   data/meta-nash-v3/pool-artifact.json      Nash support and weights
//   data/team-selection-v2/reference.json     every inventory record: v1 label,
//                                             selection label and reason,
//                                             memberships and probabilities
//
//   node tools/build-team-products-v2.js [--selection F] [--solution F] [--out-root DIR]
//
// Inputs: data/team-inventory-v1/inventory.json,
// data/team-selection-v2/selection.json, data/meta-nash-v3/solution.json,
// and the frozen prior data/belief-pool-v3/belief-pool.json, which is read
// and checked, never written: prior membership does not follow selection
// labels. The three probability fields stay separate: drawWeight (catalog),
// weight (Nash), priorWeight (reference only).
'use strict';
const fs = require('fs');
const path = require('path');
const { normalizeTraining, normalizeCatalog } = require('./normalize-training');
const { attractGenderCensus } = require('./attract-gender-census');

const REPO = path.join(__dirname, '..');
const arg = name => {
	const i = process.argv.indexOf(name);
	return i > 0 ? process.argv[i + 1] : null;
};
const OUT = arg('--out-root') ? path.resolve(arg('--out-root')) : REPO;
const read = rel => JSON.parse(fs.readFileSync(path.resolve(REPO, rel), 'utf8'));
const write = (rel, obj) => {
	fs.mkdirSync(path.dirname(path.join(OUT, rel)), { recursive: true });
	fs.writeFileSync(path.join(OUT, rel), JSON.stringify(obj, null, 1) + '\n');
};
/** The fingerprint team_eval stamps into every record's `cond.belief_*`. */
const fnv1a64 = rel => {
	let h = 0xcbf29ce484222325n;
	for (const b of fs.readFileSync(path.resolve(REPO, rel))) h = ((h ^ BigInt(b)) * 0x100000001b3n) & 0xffffffffffffffffn;
	return `fnv1a64:${h.toString(16).padStart(16, '0')}`;
};
const fail = msg => { throw new Error(msg); };

const inv = read('data/team-inventory-v1/inventory.json');
const evolved = read('data/team-selection-v2/evolved-teams.json');
const partybox = read('data/partybox-v0/teams.json');
const v1 = read('data/team-inventory-v1/classification.json');
const sel = read(arg('--selection') || 'data/team-selection-v2/selection.json');
const sol = read(arg('--solution') || 'data/meta-nash-v3/solution.json');
const deployedNash = read('data/kodoku-r3/conclusion-equilibrium-deme.json');
const PRIOR = 'data/belief-pool-v3/belief-pool.json';
const prior = read(PRIOR);
const allTeams = [...inv.teams, ...evolved.teams, ...partybox.teams];
const byId = new Map(allTeams.map(t => [t.id, t]));

const priorFp = fnv1a64(PRIOR);
const acceptedMeasurementPrior = fingerprint => fingerprint === priorFp ||
	(prior.trainingNormalization?.policy === 'training-v3' && fingerprint === prior.trainingNormalization.previousFingerprint);
if (!acceptedMeasurementPrior(sel.prior.fingerprint)) fail(`selection was measured on another prior (${sel.prior.fingerprint} vs ${priorFp})`);
if (!acceptedMeasurementPrior(sol.prior.fingerprint)) fail(`Nash solution was measured on another prior (${sol.prior.fingerprint} vs ${priorFp})`);

const LABELS = new Set(['selected', 'pending', 'not-selected']);
const measured = inv.teams.filter(t => t.eligibility.status === 'eligible' && t.measuredAs === t.id);
const pbMeasured = partybox.teams.filter(t => t.eligibility.status === 'eligible' && t.measuredAs === t.id);
const candidates = [...measured, ...evolved.teams, ...pbMeasured];
for (const t of candidates) {
	const l = sel.labels[t.id];
	if (!l || !LABELS.has(l.label)) fail(`${t.id}: no selection label`);
	if (l.label !== 'selected' && !l.reason) fail(`${t.id}: ${l.label} without a reason`);
}
for (const id of Object.keys(sel.labels)) {
	if (!byId.has(id) || byId.get(id).measuredAs !== id) fail(`selection label for non-measured id ${id}`);
}
const priorIds = new Set(prior.teams.map(t => t.id));
for (const t of candidates) if (!priorIds.has(t.id)) fail(`${t.id}: measured but missing from the frozen prior`);

// ------------------------------------------------------------ catalog
const deployedNashW = new Map(deployedNash.teams.map(t => [t.id, t.weight]));
const clusterKey = t => t.variantCluster || t.id;
const selected = candidates.filter(t => sel.labels[t.id].label === 'selected');
if (selected.length === 0) fail('no selected parties');
selected.sort((a, b) => sel.labels[b.id].score - sel.labels[a.id].score || a.id.localeCompare(b.id));
const size = new Map();
for (const t of selected) size.set(clusterKey(t), (size.get(clusterKey(t)) || 0) + 1);
const clusters = size.size;
const drawWeight = t => 1 / (clusters * size.get(clusterKey(t)));
const round = x => Math.round(x * 1e6) / 1e6;
const provenance = t => ({ family: t.family, origin: t.origin, source: t.source.name, url: t.source.url, version: t.source.version || null });
// Normalize playable sets; historical measurements retain their original
// input fingerprints and are explicitly labelled as not remeasured.
// Within any multi-member camouflage cluster, if a sibling keeps Male on a
// species (e.g. because of a Physical move like Explosion), preserve Male on
// that species across the cluster so Team Preview gender does not leak moves.
const dex = read('data/gen2stadium2.json');
const attract = attractGenderCensus(prior);
const optimized = normalizeCatalog(selected, dex, attract.preferFemale);
write('data/team-pool-v2/team-pool.json', {
	format: 'nc2000-team-pool-v2',
	generator: 'tools/build-team-products-v2.js',
	protocol: sel.protocol,
	condition: sel.condition,
	setOptimization: {
		policy: 'training-v3: maximum HP/non-Attack stats and Hidden Power power; maximum usable Attack, otherwise minimum; preserve public gender, shiny status, Hidden Power type and camouflage',
		changedTeams: selected.filter(t => optimized.get(t.id).changedSets > 0).length,
		changedSets: selected.reduce((sum, t) => sum + optimized.get(t.id).changedSets, 0),
		changedGenders: selected.reduce((sum, t) => sum + optimized.get(t.id).changedGenders, 0),
		femalePreference: { enabled: attract.preferFemale, weightedDirectSources: attract.weightedSources,
			basis: attract.basis, priorFingerprint: priorFp },
		scoreBasis: 'Historical confirmation scores and mixture weights were measured before training-v3; normalized sets/prior have not been remeasured.',
	},
	drawRule: 'each variant cluster among the selected parties is drawn with equal probability, split equally among its members (drawWeight sums to 1); the same weights serve the human Random and the bot draw',
	teams: selected.map((t, i) => ({
		id: t.id, rank: i + 1, label: 'selected',
		nash: deployedNashW.has(t.id),
		evolved: t.origin === 'evolved',
		species: t.species, levels: t.levels,
		drawWeight: round(drawWeight(t)),
		score: round(sel.labels[t.id].score),
		provenance: provenance(t),
		sets: optimized.get(t.id).sets,
	})),
});

// ------------------------------------------------------------ nash
for (const t of deployedNash.teams) {
	if (!sel.labels[t.id] || sel.labels[t.id].label !== 'selected') fail(`Deployed Nash support ${t.id} is not a selected party`);
}
const nash = Object.entries(sol.weights).filter(([, w]) => w > 0).sort((a, b) => b[1] - a[1]);
for (const [id] of nash) {
	if (!byId.has(id)) fail(`Legacy v3 Nash support ${id} is missing from inventory`);
}
write('data/meta-nash-v3/pool-artifact.json', {
	format: 'meta-nash-pool-v3',
	source_solution: 'data/meta-nash-v3/solution.json',
	conditions: sol.conditions,
	trainingPolicy: 'training-v3',
	measurementStatus: 'Historical weights; normalized sets and prior have not been remeasured.',
	teams: nash.map(([id, w]) => ({ id, origin: byId.get(id).family, weight: w, sets: normalizeTraining(byId.get(id).sets, dex) })),
});

// ------------------------------------------------------------ reference
const nashW = new Map(nash);
const priorW = new Map(prior.teams.map(t => [t.id, t.weight]));
write('data/team-selection-v2/reference.json', {
	format: 'nc2000-team-reference-v2',
	generator: 'tools/build-team-products-v2.js',
	protocol: sel.protocol,
	counts: {
		records: allTeams.length,
		eligible: allTeams.filter(t => t.eligibility.status === 'eligible').length,
		measured: candidates.length,
		selected: selected.length,
		pending: candidates.filter(t => sel.labels[t.id].label === 'pending').length,
		notSelected: candidates.filter(t => sel.labels[t.id].label === 'not-selected').length,
		prior: prior.teams.length,
		nashSupport: deployedNash.teams.length,
		legacyNashV3Support: nash.length,
	},
	teams: allTeams.map(t => {
		const rep = t.eligibility.status === 'eligible' ? t.measuredAs : null;
		const l = rep ? sel.labels[rep] : null;
		return {
			id: t.id, family: t.family, origin: t.origin, source: t.source,
			eligibility: t.eligibility,
			measuredAs: t.measuredAs,
			v1Label: rep && v1.labels[rep] ? v1.labels[rep].label : null,
			label: l ? l.label : null,
			reason: l ? l.reason || null : null,
			evidence: l ? { score: l.score ?? null, se: l.se ?? null, stage: l.stage ?? null } : null,
			catalog: !!l && l.label === 'selected' && rep === t.id,
			drawWeight: l && l.label === 'selected' && rep === t.id ? round(drawWeight(t)) : 0,
			priorWeight: priorW.get(t.id) || 0,
			nashWeight: deployedNashW.get(t.id) || 0,
			legacyNashV3Weight: nashW.get(t.id) || 0,
			variantCluster: t.variantCluster,
			species: t.species, levels: t.levels,
		};
	}),
});

console.log(`catalog ${selected.length} (${clusters} clusters) | prior ${prior.teams.length} (frozen) | deployed nash ${deployedNash.teams.length} | reference ${allTeams.length}`);
