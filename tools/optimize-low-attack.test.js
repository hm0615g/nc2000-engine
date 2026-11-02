'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const dex = require('../data/gen2stadium2.json');
const { optimizeLowAttack } = require('./optimize-low-attack');

function set(species, moves, atk = 31) {
	return { species, moves,
		ivs: { hp: 31, atk, def: 31, spa: 31, spd: 31, spe: 31 },
		evs: { hp: 255, atk: 255, def: 255, spa: 255, spd: 255, spe: 255 } };
}
const optimize = (mon, options) => optimizeLowAttack([mon], dex, options).sets[0];

test('preserves male gender threshold and maximum HP parity', () => {
	for (const species of ['Venusaur', 'Jolteon', 'Vaporeon', 'Umbreon']) {
		const mon = optimize(set(species, ['Rest']));
		assert.equal(mon.ivs.atk, 6, species); // male threshold DV 2, first odd DV 3
		assert.equal(mon.ivs.hp, 31);
		assert.equal(mon.evs.atk, 0);
	}
	assert.equal(optimize(set('Gengar', ['Perish Song'])).ivs.atk, 18); // male, 1:1 ratio
});

test('female, fixed-gender and genderless sets can reach odd DV 1', () => {
	for (const mon of [set('Gengar', ['Perish Song'], 2), set('Jynx', ['Psychic']), set('Starmie', ['Surf'])]) {
		const result = optimize(mon);
		assert.equal(result.ivs.atk, 2);
		assert.equal(result.ivs.hp, mon.ivs.hp);
	}
});

test('preserves an even Attack DV parity rather than silently increasing HP', () => {
	const mon = set('Starmie', ['Surf'], 8);
	mon.ivs.hp = 14;
	const result = optimize(mon);
	assert.equal(result.ivs.atk, 0);
	assert.equal(result.ivs.hp, 14);
});

test('preserves the shiny DV predicate together with male gender and HP', () => {
	const mon = set('Gengar', ['Psychic']);
	Object.assign(mon.ivs, { hp: 16, def: 20, spe: 20, spa: 20, spd: 20 });
	mon.gender = 'M';
	mon.shiny = true;
	const result = optimize(mon);
	assert.equal(result.ivs.atk, 22); // DV 11: male, odd and shiny
	assert.equal(result.shiny, true);
	assert.equal(result.gender, 'M');
	assert.equal(result.ivs.hp, 16);
});

test('excludes Physical moves, every Hidden Power spelling and external move access', () => {
	for (const move of ['Explosion', 'Counter', 'Hidden Power', 'Hidden Power Ice',
		'Metronome', 'Mimic', 'Mirror Move', 'Transform']) {
		const mon = set('Gengar', [move]);
		assert.deepEqual(optimize(mon), mon, move);
	}
});

test('uses Gen 2 categories and allows Sleep Talk on qualifying own moves', () => {
	for (const mon of [set('Jynx', ['Thief']), set('Umbreon', ['Pursuit']),
		set('Suicune', ['Surf', 'Ice Beam', 'Rest', 'Sleep Talk'])]) {
		assert.equal(optimize(mon).evs.atk, 0);
	}
});

test('does not mutate inputs, is idempotent, and fails closed on unknown data', () => {
	const original = [set('Starmie', ['Surf'])];
	const frozen = structuredClone(original);
	const once = optimizeLowAttack(original, dex);
	assert.deepEqual(original, frozen);
	assert.equal(once.changedSets, 1);
	const twice = optimizeLowAttack(once.sets, dex);
	assert.deepEqual(twice.sets, once.sets);
	assert.equal(twice.changedSets, 0);
	assert.throws(() => optimize(set('Starmie', ['Nonexistent Move'])), /unknown move/);
	const incomplete = set('Starmie', ['Surf']);
	delete incomplete.evs;
	assert.throws(() => optimize(incomplete), /complete validated/);
});

test('opt-in female preference removes the male threshold while preserving HP and other fields', () => {
	for (const species of ['Venusaur', 'Jolteon', 'Vaporeon', 'Umbreon']) {
		const mon = set(species, ['Rest']); mon.gender = 'M';
		const result = optimize(mon, { preferFemale: true });
		assert.equal(result.gender, 'F');
		assert.equal(result.ivs.atk, 2);
		assert.equal(result.ivs.hp, mon.ivs.hp);
		assert.equal(result.evs.atk, 0);
		const again = optimizeLowAttack([result], dex, { preferFemale: true });
		assert.deepEqual(again.sets, [result]);
		assert.equal(again.changedGenders, 0);
	}
});

test('female preference preserves own Attract targets and impossible fixed/shiny genders', () => {
	const user = set('Umbreon', ['Pursuit', 'Attract']); user.gender = 'M';
	assert.equal(optimize(user, { preferFemale: true }).gender, 'M');
	for (const [species, gender] of [['Tauros', 'M'], ['Jynx', 'F'], ['Starmie', 'N']]) {
		const mon = set(species, ['Rest']); mon.gender = gender;
		assert.equal(optimize(mon, { preferFemale: true }).gender, gender);
	}
	const shiny = set('Venusaur', ['Rest']); shiny.gender = 'M'; shiny.shiny = true;
	Object.assign(shiny.ivs, { hp: 16, def: 20, spa: 20, spd: 20, spe: 20 });
	const result = optimize(shiny, { preferFemale: true });
	assert.equal(result.gender, 'M');
	assert.equal(result.ivs.atk, 6); // no legal shiny female exists at a 1/8 female ratio
	assert.equal(result.shiny, true);
});

test('female preference is supported only by the frozen prior source mix', () => {
	const { attractGenderCensus } = require('./attract-gender-census');
	const prior = require('../data/belief-pool-v3/belief-pool.json');
	const result = attractGenderCensus(prior);
	assert.deepEqual(result.weightedSources, { M: 1.25, F: 8, N: 0 });
	assert.equal(result.preferFemale, true);
	assert.equal(result.sources.length, 10);
	const catalog = require('../data/team-pool-v2/team-pool.json');
	assert.equal(attractGenderCensus(catalog, 'drawWeight').sources.length, 0);
	const equal = { teams: [result.sources[0], result.sources[1]].map(s => ({ id: s.team, weight: 1,
		sets: [{ species: s.species, gender: s.gender, moves: ['Attract'] }] })) };
	assert.equal(attractGenderCensus(equal).preferFemale, false);
	equal.teams[0].weight = NaN;
	assert.throws(() => attractGenderCensus(equal), /invalid weight/);
});

test('training-v3 catalog retains male camouflage and constrained minimum Attack', () => {
	const catalog = require('../data/team-pool-v2/team-pool.json');
	assert.equal(catalog.teams.length, 33);
	const apex = catalog.teams.filter(t => t.nash);
	assert.equal(apex.length, 4);
	assert.equal(new Set(apex.map(t => JSON.stringify(t.sets.map(s => [s.species,s.level,s.gender,Boolean(s.item)])))).size, 1);
	for (const t of apex) {
		const p=t.sets.find(s=>s.species==='Cloyster');
		const boom=p.moves.includes('Explosion');
		assert.equal(p.gender,'M');assert.equal(Math.floor(p.ivs.atk/2),boom?15:9);assert.equal(p.evs.atk,boom?255:0);
	}
});
