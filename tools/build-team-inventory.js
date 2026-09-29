// Build the versioned candidate inventory (docs/TEAM-POOL-REBUILD-PLAN.md
// step 1): every team the rebuild may consider, with stable ids, canonical
// sets, provenance, source family, explicit deviations, eligibility, exact
// duplicate groups and near-variant links. Quality is NOT decided here —
// base labels live in classification.json, which cites evidence.
//
//   node tools/import-majinjima.js   # first, when the source changes
//   node tools/build-team-inventory.js
//
// Inputs : data/meta-pool-v0/meta-pool.json          (bundled 32)
//          data/belief-pool-v1/belief-pool.json      (shipped prior, 87)
//          data/community-rentals-v0/teams.json      (rental DB, 28)
//          data/team-inventory-v1/sources/majinjima.json
// Output : data/team-inventory-v1/inventory.json
'use strict';
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const REPO = path.join(__dirname, '..');
const DIR = path.join(REPO, 'data', 'team-inventory-v1');
const read = rel => JSON.parse(fs.readFileSync(path.join(REPO, rel), 'utf8'));
const wasm = require(path.join(REPO, 'crates/wasm/pkg-node/nc2000_wasm.js'));
const dexJson = read('data/gen2stadium2.json');
const dex = new wasm.Dex();
const validator = new wasm.Validator(dex);

const toid = s => String(s).toLowerCase().replace(/[^a-z0-9]/g, '');

// ------------------------------------------------------------ signatures
// Identity of a set for dedupe: every field that changes play.
function setKey(s) {
	const moves = s.moves.map(toid).sort().join(',');
	const iv = s.ivs || {}, ev = s.evs || {};
	const stats = ['hp', 'atk', 'def', 'spa', 'spd', 'spe'];
	return [toid(s.species), s.level, toid(s.item || ''), moves, s.gender || '',
		stats.map(k => iv[k] ?? 31).join('/'), stats.map(k => ev[k] ?? 255).join('/'), s.happiness ?? 255].join('|');
}
const teamKey = sets => sets.map(setKey).sort().join('||');
const hash = s => crypto.createHash('sha256').update(s).digest('hex').slice(0, 16);
// What team preview shows, and therefore what the blind belief matches on
// (crates/bot/src/belief.rs, preview filter).
const previewKey = sets => sets.map(s => `${toid(s.species)}:${s.level}:${s.item ? 1 : 0}`).sort().join(',');
// The play-relevant set without training (moves/item/level), for variant diffs.
const loadoutKey = s => [toid(s.species), s.level, toid(s.item || ''), s.moves.map(toid).sort().join(',')].join('|');

function canon(sets) {
	const c = JSON.parse(validator.canonicalizeTeam(JSON.stringify(sets)));
	const team = c.ok ? c.team : sets;
	const v = JSON.parse(validator.validateTeam(JSON.stringify(team)));
	const errors = [...(c.ok ? [] : c.errors), ...(v.findings || [])].filter(f => f.severity === 'error');
	return { sets: team, valid: !!v.ok && c.ok, errors, applied: c.applied || [] };
}

// ------------------------------------------------------------ records
const records = [];
function add(r) {
	if (records.some(x => x.id === r.id)) throw new Error(`duplicate id ${r.id}`);
	records.push(r);
}

// Bundled pool: the existing product pool, source-faithful as shipped.
const metaPool = read('data/meta-pool-v0/meta-pool.json');
const BP_NOTE = /^(\w[\w ]*?) had Bright Powder in the original team\.$/;
for (const t of metaPool.teams) {
	const c = canon(t.sets);
	const family = t.tier === 'T1' ? 'hc75-top8' : 'smogon-hub-samples';
	add({
		id: t.id,
		family,
		origin: 'human',
		source: {
			kind: t.tier === 'T1' ? 'tournament-report' : 'expert-sample-thread',
			name: t.provenance.source, url: t.provenance.url,
			authors: t.provenance.authors || t.provenance.player || null,
			version: t.tier === 'T1' ? `${t.provenance.event} ${t.provenance.placement}` : `thread sample ${t.id.slice(-2)}`,
		},
		inherited: ['meta-pool-v0', 'belief-pool-v1'],
		sets: c.sets,
		valid: c.valid, errors: c.errors,
		deviations: [
			...(t.tier === 'T1' ? ['DVs not published by the report; canonical Hidden Power spreads from the hub samples, otherwise maximum (meta-pool-v0 README)'] : []),
			'training not published: stat exp maximum on every stat, happiness 255 (0 for Frustration) — fixture convention',
			...(t.provenance.notes ? [`thread translation edit: ${t.provenance.notes}`] : []),
		],
	});
	// The thread swapped Bright Powder out; reconstruct the original.
	const m = BP_NOTE.exec(t.provenance.notes || '');
	if (m) {
		const mon = t.sets.find(s => s.species === m[1]);
		if (!mon) throw new Error(`${t.id}: ${m[1]} not in team`);
		const sets = t.sets.map(s => (s === mon ? { ...s, item: 'Bright Powder' } : s));
		const c2 = canon(sets);
		add({
			id: `${t.id}-orig`,
			family: 'smogon-hub-samples',
			origin: 'human',
			source: {
				kind: 'reconstructed-original',
				name: t.provenance.source, url: t.provenance.url, authors: t.provenance.authors,
				version: `original of thread sample ${t.id.slice(-2)} per its note`,
			},
			inherited: [],
			sets: c2.sets,
			valid: c2.valid, errors: c2.errors,
			deviations: [
				`reconstructed: ${m[1]} holds Bright Powder instead of the translation's ${mon.item} ("${t.provenance.notes}")`,
				'rest identical to the thread translation, including its unpublished-training convention',
			],
		});
	}
}

// Shipped prior members beyond the bundled pool.
const beliefPool = read('data/belief-pool-v1/belief-pool.json');
const rentals = read('data/community-rentals-v0/teams.json');
const rentalByCban = new Map(rentals.teams.map(t => [t.cban, t]));
const metaIds = new Set(metaPool.teams.map(t => t.id));
for (const t of beliefPool.teams) {
	if (metaIds.has(t.id)) continue;
	const c = canon(t.sets);
	let family, origin, source, deviations;
	if (t.id.startsWith('rental-cban-')) {
		const cban = +t.id.slice('rental-cban-'.length);
		const r = rentalByCban.get(cban);
		family = 'community-rentals'; origin = 'human';
		source = { kind: 'rental-db', name: 'psense rental-party DB (一撃無し2000)', url: `http://psense.lib.net/_/PDINPUT2.cgi?CBAN=${cban}`, version: r.archetype };
		deviations = ['DVs/stat exp not published by the table; canonicalizeTeam fills them (Hidden Power types decoded from the hidden PD field)'];
	} else {
		origin = 'machine';
		family = t.id.startsWith('ev-') ? 'meta-nash-evolved' : t.id.startsWith('nb-') ? 'meta-nash-neighborhood' : 'meta-nash-lineage';
		source = { kind: 'generated', name: 'META-NASH v1', url: 'docs/META-NASH-V1.md', version: t.provenance };
		deviations = [];
	}
	add({ id: t.id, family, origin, source, inherited: ['belief-pool-v1'], sets: c.sets, valid: c.valid, errors: c.errors, deviations });
}
// Rental entries the shipped prior left out, so every exclusion is visible.
for (const r of rentals.teams) {
	const id = `rental-cban-${r.cban}`;
	if (records.some(x => x.id === id)) continue;
	const c = canon(r.sets);
	add({
		id, family: 'community-rentals', origin: 'human',
		source: { kind: 'rental-db', name: 'psense rental-party DB (一撃無し2000)', url: `http://psense.lib.net/_/PDINPUT2.cgi?CBAN=${r.cban}`, version: r.archetype },
		inherited: [], sets: c.sets, valid: c.valid && r.sets.length === 6, errors: c.errors,
		deviations: ['DVs/stat exp not published by the table; canonicalizeTeam fills them'],
	});
}

// 魔人島 serious entries (plus page teams no entry designates).
const mjj = read('data/team-inventory-v1/sources/majinjima.json');
for (const e of mjj.entries) {
	const c = canon(e.sets);
	add({
		id: e.id, family: 'majinjima', origin: 'human',
		source: {
			kind: 'author-team-page', name: '魔人島 俺のパーティ軍団', url: e.url,
			version: e.heading, indexName: e.name, indexSummary: e.indexSummary,
			retrieved: mjj.retrieved, fingerprint: mjj.pageFingerprints[e.url.slice(e.url.indexOf('/party/') + 7).split('#')[0]],
			encodedSource: e.encoded, primaryOfEntry: e.primary,
		},
		inherited: [], sets: c.sets, valid: c.valid, errors: c.errors,
		ohkoRule: e.ohkoRule,
		deviations: e.notes,
	});
}
for (const u of mjj.unlistedPageTeams) {
	const c = canon(u.sets);
	add({
		id: u.id, family: 'majinjima', origin: 'human',
		source: { kind: 'author-team-page', name: '魔人島 俺のパーティ軍団', url: u.url, version: u.heading, retrieved: mjj.retrieved, encodedSource: u.encoded, unlisted: true },
		inherited: [], sets: c.sets, valid: c.valid, errors: c.errors, deviations: u.notes,
	});
}

// ------------------------------------------------------------ eligibility
// Separate from quality. A record is a candidate only if it is a complete,
// format-legal team built for this rule set and inside the intake scope.
const RULE_INTENT = [
	[/【一撃あり用】/, 'built for the OHKO-allowed rule'],
	[/【リトルカップ専用】/, 'built for Little Cup'],
	[/【指振りルール専用】/, 'built for the Metronome-only rule'],
	[/リンク集/, 'not a team (links page)'],
];
for (const r of records) {
	const reasons = [];
	if (r.sets.length !== 6) reasons.push(`incomplete team (${r.sets.length} mons)`);
	if (!r.valid) reasons.push(`format-illegal under gen2nintendocup2000noohkostadium2strict: ${r.errors.map(e => `${e.mon || ''}:${e.code}${e.move ? ':' + e.move : ''}`).join(', ')}`);
	if (r.ohkoRule) reasons.push('source entry is marked (一撃) — OHKO-allowed rule');
	const v = String(r.source.version || '');
	for (const [re, why] of RULE_INTENT) if (re.test(v)) reasons.push(why);
	if (r.source.unlisted) reasons.push('page variant no serious index entry designates — outside the source-defined intake subset (reference only)');
	r.eligibility = reasons.length ? { status: 'ineligible', reasons } : { status: 'eligible', reasons: [] };
}

// ------------------------------------------------------------ duplicates + variants
for (const r of records) {
	r.species = r.sets.map(s => s.species);
	r.levels = r.sets.map(s => s.level);
	r.setSignature = hash(teamKey(r.sets));
	r.previewSignature = previewKey(r.sets);
}
const bySig = new Map();
for (const r of records) {
	if (!bySig.has(r.setSignature)) bySig.set(r.setSignature, []);
	bySig.get(r.setSignature).push(r.id);
}
for (const r of records) {
	const group = bySig.get(r.setSignature);
	r.exactDuplicates = group.filter(id => id !== r.id);
	r.canonicalOf = group[0] === r.id ? null : group[0];
}
const byPreview = new Map();
for (const r of records) {
	if (r.eligibility.status !== 'eligible') continue;
	if (!byPreview.has(r.previewSignature)) byPreview.set(r.previewSignature, []);
	byPreview.get(r.previewSignature).push(r.id);
}
for (const r of records) {
	const g = byPreview.get(r.previewSignature) || [];
	r.previewCollisions = g.filter(id => id !== r.id && !r.exactDuplicates.includes(id));
}

// Near variants: same species multiset differing in at most three loadouts,
// or five shared species with at least three identical loadouts.
function relation(a, b) {
	const spA = new Map(a.sets.map(s => [toid(s.species), s]));
	const spB = new Map(b.sets.map(s => [toid(s.species), s]));
	const shared = [...spA.keys()].filter(k => spB.has(k));
	const sameLoadout = shared.filter(k => loadoutKey(spA.get(k)) === loadoutKey(spB.get(k)));
	const diffs = [];
	for (const k of shared) {
		const x = spA.get(k), y = spB.get(k);
		const d = [];
		if (x.level !== y.level) d.push(`level ${x.level}->${y.level}`);
		if (toid(x.item) !== toid(y.item)) d.push(`item ${x.item}->${y.item}`);
		const mx = x.moves.map(toid), my = y.moves.map(toid);
		const out = x.moves.filter(m => !my.includes(toid(m))), inn = y.moves.filter(m => !mx.includes(toid(m)));
		if (out.length) d.push(`moves -${out.join('/')} +${inn.join('/')}`);
		if (!d.length && setKey(x) !== setKey(y)) d.push('training/DVs/gender');
		if (d.length) diffs.push(`${x.species}: ${d.join(', ')}`);
	}
	for (const k of spA.keys()) if (!spB.has(k)) diffs.push(`-${spA.get(k).species}`);
	for (const k of spB.keys()) if (!spA.has(k)) diffs.push(`+${spB.get(k).species}`);
	return { sharedSpecies: shared.length, identicalLoadouts: sameLoadout.length, diffs };
}
for (const r of records) r.nearVariants = [];
for (let i = 0; i < records.length; i++) {
	for (let j = i + 1; j < records.length; j++) {
		const a = records[i], b = records[j];
		if (a.setSignature === b.setSignature) continue;
		const rel = relation(a, b);
		const near = (rel.sharedSpecies === 6 && rel.identicalLoadouts >= 3) || (rel.sharedSpecies >= 5 && rel.identicalLoadouts >= 3);
		if (!near) continue;
		a.nearVariants.push({ id: b.id, sharedSpecies: rel.sharedSpecies, identicalLoadouts: rel.identicalLoadouts, diffs: rel.diffs });
		b.nearVariants.push({ id: a.id, sharedSpecies: rel.sharedSpecies, identicalLoadouts: rel.identicalLoadouts, diffs: relation(b, a).diffs });
	}
}
// Loadout duplicates: all six loadouts identical, only DVs / training /
// gender differ. One member is measured on the others' behalf — the one
// whose training the source actually publishes.
const AUTHORITY = r => (r.family === 'majinjima' ? (r.source.encodedSource ? 0 : 1)
	: r.family === 'hc75-top8' || r.family === 'smogon-hub-samples' ? 2
		: r.family === 'community-rentals' ? 3 : 4);
for (const r of records) {
	r.loadoutDuplicates = r.nearVariants.filter(v => v.sharedSpecies === 6 && v.identicalLoadouts === 6).map(v => v.id);
}
for (const r of records) {
	const group = [r, ...r.loadoutDuplicates.map(id => records.find(x => x.id === id))]
		.filter(x => x.eligibility.status === 'eligible');
	group.sort((a, b) => AUTHORITY(a) - AUTHORITY(b) || records.indexOf(a) - records.indexOf(b));
	r.measuredAs = r.eligibility.status === 'eligible' ? group[0].id : null;
}

// Variant clusters (connected components over near-variant + duplicate
// links among eligible records) — the unit that keeps many near copies from
// buying weight by count.
const eligible = records.filter(r => r.eligibility.status === 'eligible');
const parent = new Map(eligible.map(r => [r.id, r.id]));
const find = x => (parent.get(x) === x ? x : (parent.set(x, find(parent.get(x))), parent.get(x)));
const union = (a, b) => { if (parent.has(a) && parent.has(b)) parent.set(find(a), find(b)); };
for (const r of eligible) {
	for (const v of r.nearVariants) union(r.id, v.id);
	for (const d of r.exactDuplicates) union(r.id, d);
}
const clusters = new Map();
for (const r of eligible) {
	const root = find(r.id);
	if (!clusters.has(root)) clusters.set(root, []);
	clusters.get(root).push(r.id);
}
let ci = 0;
const clusterId = new Map();
for (const [, ids] of [...clusters].sort((a, b) => b[1].length - a[1].length || a[0].localeCompare(b[0]))) {
	const cid = `vc${String(++ci).padStart(2, '0')}`;
	for (const id of ids) clusterId.set(id, cid);
}
for (const r of records) r.variantCluster = clusterId.get(r.id) || null;

// ------------------------------------------------------------ write
const out = {
	format: 'nc2000-team-inventory-v1',
	regulation: 'gen2nintendocup2000noohkostadium2strict',
	generator: 'tools/build-team-inventory.js',
	sources: {
		'meta-pool-v0': 'data/meta-pool-v0/meta-pool.json',
		'belief-pool-v1': 'data/belief-pool-v1/belief-pool.json',
		'community-rentals-v0': 'data/community-rentals-v0/teams.json',
		majinjima: 'data/team-inventory-v1/sources/majinjima.json',
	},
	counts: {
		records: records.length,
		eligible: eligible.length,
		eligibleDistinct: new Set(eligible.map(r => r.setSignature)).size,
		variantClusters: clusters.size,
		measured: new Set(records.map(r => r.measuredAs).filter(Boolean)).size,
	},
	teams: records.map(r => ({
		id: r.id, family: r.family, origin: r.origin, source: r.source, inherited: r.inherited,
		eligibility: r.eligibility,
		species: r.species, levels: r.levels,
		setSignature: r.setSignature, canonicalOf: r.canonicalOf, exactDuplicates: r.exactDuplicates,
		previewSignature: r.previewSignature, previewCollisions: r.previewCollisions,
		variantCluster: r.variantCluster, nearVariants: r.nearVariants,
		loadoutDuplicates: r.loadoutDuplicates, measuredAs: r.measuredAs,
		deviations: r.deviations,
		sets: r.sets,
	})),
};
fs.writeFileSync(path.join(DIR, 'inventory.json'), JSON.stringify(out, null, 1) + '\n');
console.log(JSON.stringify(out.counts));
const fams = {};
for (const r of records) {
	const k = `${r.family}:${r.eligibility.status}`;
	fams[k] = (fams[k] || 0) + 1;
}
console.log(fams);
for (const r of records) {
	if (r.eligibility.status !== 'eligible') console.log(`INELIGIBLE ${r.id}: ${r.eligibility.reasons.join('; ')}`);
}
for (const r of records) if (r.exactDuplicates.length && !r.canonicalOf) console.log(`DUP ${r.id} == ${r.exactDuplicates.join(', ')}`);
for (const [root, ids] of clusters) if (ids.length > 1) console.log(`${clusterId.get(root)}: ${ids.join(' ')}`);
for (const [k, ids] of byPreview) if (ids.length > 1) console.log(`PREVIEW-COLLISION ${ids.join(' ')}`);
