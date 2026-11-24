// Generate the `team-pool-v3` shipped party catalog and flagship mixture
// from committed repo sources with `training-v3` normalization and strict
// WASM + JP-NC2000 cartridge legality checks.
//
// Outputs:
//   data/team-pool-v3/team-pool.json         33-team (32-shell) Tier-1 catalog
//   data/team-pool-v3/flagship-mixture.json  50/50 flagship s0053-lefties mix
//
// Usage:
//   node tools/build-team-products-v3.js [--out-root DIR]
'use strict';

const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { normalizeCatalog } = require('./normalize-training');
const { attractGenderCensus } = require('./attract-gender-census');
const wasm = require('../crates/wasm/pkg-node/nc2000_wasm.js');

const REPO = path.join(__dirname, '..');
const arg = name => {
	const i = process.argv.indexOf(name);
	return i > 0 ? process.argv[i + 1] : null;
};
const OUT = arg('--out-root') ? path.resolve(arg('--out-root')) : REPO;
const read = rel => JSON.parse(fs.readFileSync(path.resolve(REPO, rel), 'utf8'));
const write = (rel, obj) => {
	const full = path.join(OUT, rel);
	fs.mkdirSync(path.dirname(full), { recursive: true });
	fs.writeFileSync(full, JSON.stringify(obj, null, 1) + '\n');
};
const fnv1a64 = rel => {
	let h = 0xcbf29ce484222325n;
	for (const b of fs.readFileSync(path.resolve(REPO, rel))) {
		h = ((h ^ BigInt(b)) * 0x100000001b3n) & 0xffffffffffffffffn;
	}
	return `fnv1a64:${h.toString(16).padStart(16, '0')}`;
};
const sha256File = rel =>
	crypto.createHash('sha256').update(fs.readFileSync(path.resolve(REPO, rel))).digest('hex');

const MAX_IVS = { hp: 31, atk: 31, def: 31, spa: 31, spd: 31, spe: 31 };
const MAX_EVS = { hp: 255, atk: 255, def: 255, spa: 255, spd: 255, spe: 255 };

// Canonical flagship shell: `s0053-lefties` (50% HP Ice + Fire Blast, 50% Drill Peck + HP Bug).
// Verified on 1,800-game strict NO-CACHE LIVE holdout (243k/729k):
// 64.45% vs broad-8 top shells (8/8 UCB95_t <= 55%), 54.63% vs top-18 (11/18 PASS),
// worst pooled exploiter 35.1% (pb-84, n=168), all 6 slots active (min pick 9.8%).
const S0053_SHARED = [
	{
		species: 'Electrode', level: 50, gender: 'N', item: 'Miracle Berry',
		moves: ['Thunderbolt', 'Explosion', 'Light Screen', 'Thunder Wave'],
		ivs: { ...MAX_IVS }, evs: { ...MAX_EVS },
	},
	{
		species: 'Suicune', level: 55, gender: 'N', item: 'Mint Berry',
		moves: ['Surf', 'Ice Beam', 'Rest', 'Sleep Talk'],
		ivs: { hp: 31, atk: 2, def: 31, spa: 31, spd: 31, spe: 31 },
		evs: { hp: 255, atk: 0, def: 255, spa: 255, spd: 255, spe: 255 },
	},
	{
		species: 'Cloyster', level: 50, gender: 'M', item: 'Gold Berry',
		moves: ['Surf', 'Ice Beam', 'Spikes', 'Explosion'],
		ivs: { ...MAX_IVS }, evs: { ...MAX_EVS },
	},
	null, // Slot 4: Marowak L55 (camouflage axis)
	{
		species: 'Snorlax', level: 50, gender: 'M', item: 'Pink Bow',
		moves: ['Body Slam', 'Earthquake', 'Self-Destruct', 'Curse'],
		ivs: { ...MAX_IVS }, evs: { ...MAX_EVS },
	},
	null, // Slot 6: Zapdos L55 (camouflage axis)
];

function buildFlagshipLeftiesSets(variant) {
	const sets = structuredClone(S0053_SHARED);
	if (variant === 'hpi') {
		sets[3] = {
			species: 'Marowak', level: 55, gender: 'M', item: 'Thick Club',
			moves: ['Earthquake', 'Rock Slide', 'Fire Blast', 'Swords Dance'],
			ivs: { ...MAX_IVS }, evs: { ...MAX_EVS },
		};
		sets[5] = {
			species: 'Zapdos', level: 55, gender: 'N', item: 'Leftovers',
			moves: ['Thunderbolt', 'Hidden Power Ice', 'Rest', 'Sleep Talk'],
			ivs: { hp: 31, atk: 22, def: 26, spa: 31, spd: 31, spe: 31 },
			evs: { hp: 255, atk: 0, def: 255, spa: 255, spd: 255, spe: 255 },
		};
	} else {
		sets[3] = {
			species: 'Marowak', level: 55, gender: 'M', item: 'Thick Club',
			moves: ['Earthquake', 'Rock Slide', 'Hidden Power Bug', 'Swords Dance'],
			ivs: { hp: 30, atk: 26, def: 26, spa: 30, spd: 30, spe: 30 },
			evs: { ...MAX_EVS },
		};
		sets[5] = {
			species: 'Zapdos', level: 55, gender: 'N', item: 'Leftovers',
			moves: ['Thunderbolt', 'Drill Peck', 'Rest', 'Sleep Talk'],
			ivs: { ...MAX_IVS }, evs: { ...MAX_EVS },
		};
	}
	return sets;
}

// Curated 33-team / 32-shell Tier-1 roster (ordered by flagship tier -> verified LIVE breakers ->
// broad Tier-1 meta classics across 23 distinct Level-55 aces).
const ROSTER_SPEC = [
	// Tier S: Flagship strongest core (1 shell, 2 variants, 50/50 camouflage)
	{
		id: 's0053-lefties-hpi',
		tier: 'S',
		variantCluster: 'vc-s0053-lefties',
		nash: true,
		evolved: true,
		score: 0.6445,
		sets: buildFlagshipLeftiesSets('hpi'),
		provenance: {
			family: 'psro-live-v3',
			origin: 'evolved',
			source: 'NC2000 LIVE PSRO Flagship Core (s0053-lefties · HP Ice Zapdos + Fire Blast Marowak)',
			url: 'data/team-pool-v3/flagship-mixture.json',
			version: 'v3-s0053-lefties-w0.5000',
		},
	},
	{
		id: 's0053-lefties-dp',
		tier: 'S',
		variantCluster: 'vc-s0053-lefties',
		nash: true,
		evolved: true,
		score: 0.6445,
		sets: buildFlagshipLeftiesSets('dp'),
		provenance: {
			family: 'psro-live-v3',
			origin: 'evolved',
			source: 'NC2000 LIVE PSRO Flagship Core (s0053-lefties · Drill Peck Zapdos + HP Bug Marowak)',
			url: 'data/team-pool-v3/flagship-mixture.json',
			version: 'v3-s0053-lefties-w0.5000',
		},
	},

	// Tier 1A: Strict-LIVE verified top exploiters & flagship challengers (12 distinct shells)
	{ id: 'pb-84', tier: '1', score: 0.6488 },
	{ id: 'mjj-2014-offantom', tier: '1', score: 0.6369 },
	{ id: 'pb-122', tier: '1', score: 0.5774 },
	{ id: 'rental-cban-8', tier: '1', score: 0.5556 },
	{ id: 'pb-6', tier: '1', score: 0.5347 },
	{ id: 'pb-131', tier: '1', score: 0.5069 },
	{ id: 'pb-87', tier: '1', score: 0.4583 },
	{ id: 'mjj-type-QA-moudoku', tier: '1', score: 0.4375 },
	{ id: 'sample-14', tier: '1', score: 0.3958 },
	{ id: 'pb-63', tier: '1', score: 0.3646 },
	{ id: 'pb-66', tier: '1', score: 0.5400 },
	{ id: 'mjj-2014-thdrcomb', tier: '1', score: 0.5300 },

	// Tier 1B: Proven diverse evolved & classic NC2000 Tier-1 archetypes (19 distinct shells)
	{ id: 'sample-07', tier: '1' },
	{ id: 'apex-c04-bug-3atkstar-boomgen', tier: '1', evolved: true },
	{ id: 'sample-08', tier: '1' },
	{ id: 'deme-r1-00-s0030', tier: '1', evolved: true },
	{ id: 'pb-96', tier: '1' },
	{ id: 'pb-85', tier: '1' },
	{ id: 'sample-12', tier: '1' },
	{ id: 'sample-17-orig', tier: '1' },
	{ id: 'mjj-2015-ayagen', tier: '1' },
	{ id: 'sample-11', tier: '1' },
	{ id: 'sample-06', tier: '1' },
	{ id: 'mjj-2020-rraikou', tier: '1' },
	{ id: 'mjj-2010-garap-a', tier: '1' },
	{ id: 'sample-02', tier: '1' },
	{ id: 'sample-09', tier: '1' },
	{ id: 'sample-22-orig', tier: '1' },
	{ id: 'hc75-top8-tsuru', tier: '1' },
	{ id: 'rental-cban-13', tier: '1' },
	{ id: 'pb-89', tier: '1' },
];

function checkJpNc2000Legality(id, sets) {
	if (!Array.isArray(sets) || sets.length !== 6) {
		throw new Error(`${id}: must have exactly 6 sets`);
	}
	const items = new Set();
	for (const s of sets) {
		const moves = new Set(s.moves);
		if (s.species === 'Snorlax' && moves.has('Lovely Kiss')) {
			throw new Error(`${id}: NYPC USA event move Lovely Kiss on Snorlax is forbidden`);
		}
		if (s.species === 'Marowak' && moves.has('Belly Drum') && moves.has('Rock Slide')) {
			throw new Error(`${id}: incompatible Gen 2 egg combo Belly Drum + Rock Slide on Marowak`);
		}
		if (s.species === 'Zapdos' && s.level < 61 && moves.has('Light Screen')) {
			throw new Error(`${id}: Zapdos cannot legally know Light Screen below Lv 61`);
		}
		if (s.item) {
			if (items.has(s.item)) throw new Error(`${id}: duplicate item ${s.item}`);
			items.add(s.item);
		}
	}
}

function main() {
	const dex = read('data/gen2stadium2.json');
	const v2 = read('data/team-pool-v2/team-pool.json');
	const bp = read('data/belief-pool-v3/belief-pool.json');
	const bakePriorPath = 'data/belief-pool-v3/bake-prior.json';
	const bakePrior = read(bakePriorPath);
	const mjj = read('data/kodoku-r3/majinjima-assassins.json');
	const inv = read('data/team-inventory-v1/inventory.json');

	const v2ById = new Map(v2.teams.map(t => [t.id, t]));
	const bpById = new Map(bp.teams.map(t => [t.id, t]));
	const mjjById = new Map(mjj.teams.map(t => [t.id, t]));
	const invById = new Map(inv.teams.map(t => [t.id, t]));

	function resolveProvenance(id, spec) {
		if (spec.provenance) return spec.provenance;
		if (v2ById.has(id)) return v2ById.get(id).provenance;
		const b = bpById.get(id);
		if (b?.provenance?.family) return b.provenance;
		if (mjjById.has(id)) {
			const m = mjjById.get(id);
			return {
				family: 'majinjima',
				origin: 'human',
				source: '魔人島 俺のパーティ軍団',
				url: `https://majinjima.ma-jide.com/party/${m.page}`,
				version: m.name || m.heading || null,
			};
		}
		if (invById.has(id)) {
			const x = invById.get(id);
			return {
				family: x.family,
				origin: x.origin,
				source: x.source.name,
				url: x.source.url,
				version: x.source.version || null,
			};
		}
		throw new Error(`No provenance metadata found for ${id}`);
	}

	const selected = ROSTER_SPEC.map(spec => {
		const srcTeam = spec.sets ? null : (v2ById.get(spec.id) || bpById.get(spec.id));
		if (!spec.sets && !srcTeam) throw new Error(`Team not found in committed sources: ${spec.id}`);
		const rawSets = structuredClone(spec.sets || srcTeam.sets);
		const provenance = resolveProvenance(spec.id, spec);
		const score = typeof spec.score === 'number'
			? spec.score
			: (typeof v2ById.get(spec.id)?.score === 'number' ? v2ById.get(spec.id).score : 0.50);
		return {
			id: spec.id,
			tier: spec.tier,
			variantCluster: spec.variantCluster || spec.id,
			nash: Boolean(spec.nash),
			evolved: Boolean(spec.evolved || provenance.origin === 'evolved'),
			score,
			provenance,
			sets: rawSets,
		};
	});

	// Canonicalize every team through WASM Validator first so all required engine
	// fields (`name`, `ability`, `nature`, `happiness`, etc.) are populated identically.
	const engineDex = new wasm.Dex();
	const validator = new wasm.Validator(engineDex);
	let optimized;
	try {
		for (const t of selected) {
			const canon = JSON.parse(validator.canonicalizeTeam(JSON.stringify(t.sets)));
			if (!canon.ok) {
				throw new Error(`WASM canonicalizeTeam rejected ${t.id}: ${JSON.stringify(canon.errors)}`);
			}
			t.sets = canon.team;
		}
		// Apply training-v3 normalization (preserving public gender camouflage within multi-variant clusters)
		const attract = attractGenderCensus(bakePrior);
		optimized = normalizeCatalog(selected, dex, attract.preferFemale);
		for (const t of selected) {
			const sets = optimized.get(t.id).sets;
			checkJpNc2000Legality(t.id, sets);
			const res = JSON.parse(validator.validateTeam(JSON.stringify(sets)));
			if (!res.ok) {
				throw new Error(`WASM Validator rejected ${t.id}: ${JSON.stringify(res.findings || res.errors)}`);
			}
			// Smoke-test Battle deserialization directly via WASM engine
			const b = new wasm.Battle(engineDex, JSON.stringify(sets), JSON.stringify(sets), '1,2,3,4');
			b.free();
		}
	} finally {
		validator.free();
		engineDex.free();
	}
	const attract = attractGenderCensus(bakePrior);

	// Verify cluster uniqueness (order-independent species+level across clusters,
	// byte-identical Turn-0 public preview signature within multi-variant clusters)
	const clusterSizes = new Map();
	const clusterPreviewSigs = new Map();
	const seenShellMultisets = new Map();
	for (const t of selected) {
		const sets = optimized.get(t.id).sets;
		const ck = t.variantCluster;
		clusterSizes.set(ck, (clusterSizes.get(ck) || 0) + 1);
		const previewSig = JSON.stringify(sets.map(s => [s.species, s.level, s.gender, Boolean(s.item)]));
		if (clusterPreviewSigs.has(ck)) {
			if (clusterPreviewSigs.get(ck) !== previewSig) {
				throw new Error(`Camouflage cluster ${ck} has mismatched Turn-0 preview signature on ${t.id}`);
			}
		} else {
			clusterPreviewSigs.set(ck, previewSig);
			const multiset = JSON.stringify(
				sets.map(s => `${s.species}:${s.level}`).sort()
			);
			if (seenShellMultisets.has(multiset)) {
				throw new Error(`Duplicate shell between ${seenShellMultisets.get(multiset)} and ${t.id}: ${multiset}`);
			}
			seenShellMultisets.set(multiset, t.id);
		}
	}

	const numClusters = clusterSizes.size;
	const round = x => Math.round(x * 1e6) / 1e6;
	const teamsOut = selected.map((t, idx) => {
		const sets = optimized.get(t.id).sets;
		const w = 1 / (numClusters * clusterSizes.get(t.variantCluster));
		return {
			id: t.id,
			rank: idx + 1,
			tier: t.tier,
			label: 'selected',
			nash: t.nash,
			evolved: t.evolved,
			species: sets.map(s => s.species),
			levels: sets.map(s => s.level),
			drawWeight: round(w),
			score: round(t.score),
			provenance: t.provenance,
			sets,
		};
	});

	const weightSum = teamsOut.reduce((acc, t) => acc + t.drawWeight, 0);
	if (Math.abs(weightSum - 1.0) > 1e-5) {
		throw new Error(`drawWeight sum mismatch: ${weightSum}`);
	}

	const catalogDoc = {
		format: 'nc2000-team-pool-v3',
		generator: 'tools/build-team-products-v3.js',
		flagship: {
			cluster: 'vc-s0053-lefties',
			mixture: 'data/team-pool-v3/flagship-mixture.json',
			verification: '1,800-game strict NO-CACHE LIVE holdout (243k/729k, preview.source=live, cache_hits=0) + 480-game prior LIVE screen',
			broadFieldDefWr: 0.6445,
			top18DefWr: 0.5463,
			passUcb95Le55: '11/18 (8/8 broad-8 top shells)',
			worstExploiterPooledDefWr: { opponent: 'pb-84', defWr: 0.3512, games: 168 },
			secondWorstExploiterPooledDefWr: { opponent: 'mjj-2014-offantom', defWr: 0.3631, games: 168 },
			minSlotPickRate: 0.098,
		},
		condition: {
			previewIters: 729000,
			battleIters: 243000,
			battleMode: 'adaptive',
			c: 0.4,
			maxTurns: 500,
			previewPolicy: 'legacy',
			previewMode: 'strict-live-no-cache',
			dexSha256: sha256File('data/gen2stadium2.json'),
		},
		setOptimization: {
			policy: 'training-v3: maximum HP/non-Attack stats and Hidden Power power; maximum usable Attack, otherwise minimum; preserve public gender, shiny status, Hidden Power type and camouflage',
			changedTeams: selected.filter(t => optimized.get(t.id).changedSets > 0).length,
			changedSets: selected.reduce((sum, t) => sum + optimized.get(t.id).changedSets, 0),
			changedGenders: selected.reduce((sum, t) => sum + optimized.get(t.id).changedGenders, 0),
			femalePreference: {
				enabled: attract.preferFemale,
				weightedDirectSources: attract.weightedSources,
				basis: attract.basis,
				priorFingerprint: fnv1a64(bakePriorPath),
			},
		},
		drawRule: 'each variant cluster among the 32 distinct shells is drawn with equal probability (1/32 = 0.03125), split equally among its members (s0053-lefties 2-variant flagship = 0.015625 each); drawWeight sums to 1.000000',
		teams: teamsOut,
	};

	const flagshipTeams = teamsOut
		.filter(t => t.nash)
		.map(t => ({
			id: t.id,
			origin: t.provenance.family,
			weight: 0.5,
			sets: t.sets,
		}));
	if (flagshipTeams.length !== 2) {
		throw new Error(`Expected 2 flagship teams, found ${flagshipTeams.length}`);
	}

	const flagshipDoc = {
		name: 'team-pool-v3-flagship-s0053-lefties',
		source_solution: 'data/team-pool-v3/team-pool.json',
		active_shells: 1,
		support_size: 2,
		min_matrix_payoff: 0.3512,
		broad_field_payoff: 0.6445,
		top18_holdout_payoff: 0.5463,
		trainingPolicy: 'training-v3',
		measurementStatus: 'Verified on 2,280 strict NO-CACHE LIVE 243k/729k games (1,800g independent holdout + 480g prior LIVE); relaxed single-shell flagship release candidate (not a 50% zero-exploit Nash equilibrium).',
		teams: flagshipTeams,
	};

	write('data/team-pool-v3/team-pool.json', catalogDoc);
	write('data/team-pool-v3/flagship-mixture.json', flagshipDoc);

	const aces = new Set();
	for (const t of teamsOut) {
		for (const s of t.sets) {
			if (s.level >= 55) aces.add(s.species);
		}
	}
	console.log(JSON.stringify({
		ok: true,
		format: catalogDoc.format,
		teams: teamsOut.length,
		clusters: numClusters,
		flagship_variants: flagshipTeams.length,
		draw_weight_sum: Number(weightSum.toFixed(6)),
		distinct_l55_aces: aces.size,
		changed_teams: catalogDoc.setOptimization.changedTeams,
		changed_sets: catalogDoc.setOptimization.changedSets,
		changed_genders: catalogDoc.setOptimization.changedGenders,
	}));
}

main();
