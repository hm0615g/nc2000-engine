'use strict';

// Training policy v3. Loadouts, public gender, shiny status and Hidden Power
// type are constraints. Maximize HP and the other non-Attack stats, then HP
// power; maximize Attack when it can power a move, otherwise minimize it.
const STATS = ['hp', 'atk', 'def', 'spa', 'spd', 'spe'];
const FIXED_PHYSICAL = new Set(['counter', 'seismictoss', 'superfang', 'bide']);
const PHYSICAL_ACCESS = new Set(['metronome', 'mimic', 'mirrormove']);
const id = s => s.toLowerCase().replace(/[^a-z0-9]/g, '');
const cache = new Map();
const shiny = (a, d, s, c) => d === 10 && s === 10 && c === 10 && a % 4 >= 2;
const hp = (a, d, s, c) => (a % 2) * 8 + (d % 2) * 4 + (s % 2) * 2 + c % 2;
const power = (a, d, s, c) => Math.floor((5 * ((a >> 3) * 8 + (d >> 3) * 4 + (s >> 3) * 2 + (c >> 3)) + c % 4) / 2) + 31;
const unown = (a, d, s, c) => Math.floor((((a & 6) << 5) | ((d & 6) << 3) | ((s & 6) << 1) | ((c & 6) >> 1)) / 10);

function usesAttack(set, dex) {
	return set.moves.some(m => {
		const key = id(m), move = dex.moves[key];
		if (!move) throw new Error(`unknown move: ${m}`);
		return PHYSICAL_ACCESS.has(key) || (move.category === 'Physical' && !FIXED_PHYSICAL.has(key));
	});
}

function normalizeTraining(sets, dex) {
	return sets.map(source => {
		const set = structuredClone(source), sp = dex.species[id(set.species)];
		if (!sp) throw new Error(`unknown species: ${set.species}`);
		for (const k of STATS) {
			if (!Number.isInteger(set.ivs?.[k]) || set.ivs[k] < 0 || set.ivs[k] > 31 ||
				!Number.isInteger(set.evs?.[k]) || set.evs[k] < 0 || set.evs[k] > 255)
				throw new Error(`${set.species}: complete validated IV/EV maps required`);
		}
		const [a0, d0, s0, c0] = ['atk', 'def', 'spe', 'spa'].map(k => Math.floor(set.ivs[k] / 2));
		const gender = a => sp.gender || (a < (sp.genderRatio?.F ?? 0.5) * 16 ? 'F' : 'M');
		if (gender(a0) !== set.gender) throw new Error(`${set.species}: inconsistent source gender`);
		const targetShiny = shiny(a0, d0, s0, c0);
		const hpType = set.moves.some(m => id(m).startsWith('hiddenpower')) ? 4 * (a0 % 4) + d0 % 4 : null;
		const form = id(set.species) === 'unown' ? unown(a0, d0, s0, c0) : null;
		const attack = usesAttack(set, dex);
		const key = JSON.stringify([id(set.species), set.gender, targetShiny, hpType, form, attack]);
		let best = cache.get(key);
		if (!best) {
			let bestScore;
			for (let a = 0; a < 16; a++) for (let d = 0; d < 16; d++) {
				if (gender(a) !== set.gender || (hpType !== null && 4 * (a % 4) + d % 4 !== hpType)) continue;
				for (let s = 0; s < 16; s++) for (let c = 0; c < 16; c++) {
					if (shiny(a, d, s, c) !== targetShiny || (form !== null && unown(a, d, s, c) !== form)) continue;
					const score = [hp(a, d, s, c), d, c, s, hpType === null ? 0 : power(a, d, s, c), attack ? a : -a];
					const first = bestScore ? score.findIndex((x, i) => x !== bestScore[i]) : -1;
					if (!bestScore || (first >= 0 && score[first] > bestScore[first])) {
						bestScore = score; best = [hp(a, d, s, c), a, d, c, c, s];
					}
				}
			}
			if (!best) throw new Error(`${set.species}: no feasible training spread`);
			cache.set(key, best);
		}
		set.ivs = Object.fromEntries(STATS.map((k, i) => [k, best[i] === 15 ? 31 : best[i] * 2]));
		set.evs = Object.fromEntries(STATS.map(k => [k, k === 'atk' && !attack ? 0 : 255]));
		const returnMove = set.moves.some(m => id(m) === 'return');
		const frustration = set.moves.some(m => id(m) === 'frustration');
		if (returnMove && frustration) throw new Error(`${set.species}: Return and Frustration require conflicting happiness`);
		if (returnMove) set.happiness = 255;
		if (frustration) set.happiness = 0;
		return set;
	});
}

function normalizeCatalog(teams, dex, preferFemale) {
	const { optimizeLowAttack } = require('./optimize-low-attack');
	const groups = new Map();
	for (const t of teams) {
		const key = t.variantCluster || t.id;
		if (!groups.has(key)) groups.set(key, []);
		groups.get(key).push(t);
	}
	const out = new Map();
	for (const group of groups.values()) {
		const preserve = new Set();
		if (group.length > 1) for (const t of group) {
			for (const s of optimizeLowAttack(t.sets, dex, { preferFemale }).sets)
				if (s.gender === 'M') preserve.add(s.species);
		}
		for (const t of group) {
			const pre = optimizeLowAttack(t.sets, dex, { preferFemale, preserveGenderSpecies: preserve });
			const sets = normalizeTraining(pre.sets, dex);
			out.set(t.id, { sets,
				changedSets: sets.filter((s, i) => JSON.stringify(s) !== JSON.stringify(t.sets[i])).length,
				changedGenders: sets.filter((s, i) => s.gender !== t.sets[i].gender).length });
		}
	}
	return out;
}

module.exports = { normalizeTraining, normalizeCatalog, usesAttack };

if (require.main === module) {
	const fs = require('fs');
	const dex = JSON.parse(fs.readFileSync(require('path').join(__dirname, '../data/gen2stadium2.json'), 'utf8'));
	const input = JSON.parse(fs.readFileSync(0, 'utf8'));
	process.stdout.write(JSON.stringify(Object.fromEntries(Object.entries(input).map(([k, sets]) => [k, normalizeTraining(sets, dex)]))));
}
