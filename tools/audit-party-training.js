// Read-only audit of catalog training, camouflage and the active Nash sets.
// node tools/audit-party-training.js OUTPUT.json
'use strict';
const fs = require('fs');
const crypto = require('crypto');
const assert = require('node:assert/strict');
const paths = {
	catalog: 'data/team-pool-v2/team-pool.json',
	nash: 'data/kodoku-r3/conclusion-equilibrium-deme.json',
	dex: 'data/gen2stadium2.json',
	partybox: 'data/partybox-v0/teams.json',
};
const input = Object.fromEntries(Object.entries(paths).map(([key, path]) => [key, JSON.parse(fs.readFileSync(path, 'utf8'))]));
const { catalog, nash, dex, partybox } = input;
const id = s => s.toLowerCase().replace(/[^a-z0-9]/g, '');
const dv = (s, k) => Math.floor(s.ivs[k] / 2);
const gender = (s, atk) => {
	const sp = dex.species[id(s.species)];
	return sp.gender || (atk < (sp.genderRatio?.F ?? 0.5) * 16 ? 'F' : 'M');
};
const shiny = (s, atk) => ['def', 'spa', 'spe'].every(k => dv(s, k) === 10) && atk % 4 >= 2;
const hpType = (s, atk) => 4 * (atk % 4) + dv(s, 'def') % 4;
const hpPower = (s, atk) => Math.floor((5 * (8 * (atk >> 3) + 4 * (dv(s, 'def') >> 3)
	+ 2 * (dv(s, 'spe') >> 3) + (dv(s, 'spa') >> 3)) + dv(s, 'spa') % 4) / 2) + 31;
const physical = s => s.moves.some(m => dex.moves[id(m)].category === 'Physical');
const hiddenPower = s => s.moves.some(m => id(m).startsWith('hiddenpower'));
const callers = s => s.moves.some(m => ['metronome', 'mimic', 'mirrormove', 'transform'].includes(id(m)));
const shell = t => t.sets.map(s => `${id(s.species)}:${s.level}:${Boolean(s.item)}`).sort().join(',');
const publicSignature = t => t.sets.map(s => `${id(s.species)}:${s.level}:${s.gender}:${Boolean(s.item)}`).sort().join(',');
const groups = new Map();
for (const t of catalog.teams) {
	if (!groups.has(shell(t))) groups.set(shell(t), []);
	groups.get(shell(t)).push(t);
}
const rows = [];
const issues = [];
for (const t of catalog.teams) {
	for (const s of t.sets) {
		const hasPhysical = physical(s), hasHP = hiddenPower(s), hasCaller = callers(s);
		const old = dv(s, 'atk');
		assert.equal(gender(s, old), s.gender, `${t.id}/${s.species}: gender/DV mismatch`);
		const options = Array.from({ length: 16 }, (_, a) => a).filter(a =>
			a % 2 === old % 2 && gender(s, a) === s.gender && shiny(s, a) === shiny(s, old)
			&& (!hasHP || (hpType(s, a) === hpType(s, old) && hpPower(s, a) === hpPower(s, old))));
		const minimum = Math.min(...options), maximum = Math.max(...options);
		const nonAttackEvDeficits = ['hp', 'def', 'spa', 'spd', 'spe'].filter(k => s.evs[k] < 252);
		const nonAttackDvDeficits = hasHP
			? ['def', 'spa', 'spd', 'spe'].filter(k => dv(s, k) < (k === 'def' ? 12 + dv(s, k) % 4 : 15))
			: ['hp', 'def', 'spa', 'spd', 'spe'].filter(k => dv(s, k) < 15);
		const siblings = groups.get(shell(t)).filter(x => x.id !== t.id);
		const physicalMaleSiblings = siblings.filter(x => x.sets.some(p =>
			id(p.species) === id(s.species) && p.gender === 'M' && physical(p))).map(x => x.id);
		const row = { team: t.id, species: s.species, gender: s.gender,
			attackDv: old, attackEv: s.evs.atk, minimumDvAtSamePublicGenderAndHp: minimum,
			maximumDvAtSamePublicGenderAndHp: maximum, hasPhysical, hasHiddenPower: hasHP,
			hasExternalMoveAccess: hasCaller, physicalMaleSiblings, nonAttackEvDeficits, nonAttackDvDeficits };
		if (nonAttackEvDeficits.length || nonAttackDvDeficits.length) issues.push({ type: 'non-attack-stat', ...row });
		if (!hasPhysical && !hasHP && !hasCaller) {
			if (old !== minimum || s.evs.atk !== 0) issues.push({ type: 'low-attack-policy', ...row });
			const expectedGender = dex.species[id(s.species)].gender ||
				(physicalMaleSiblings.length || s.moves.some(m => id(m) === 'attract') ? s.gender : 'F');
			if (s.gender !== expectedGender) issues.push({ type: 'female-preference', ...row, expectedGender });
		}
		if (physicalMaleSiblings.length && s.gender !== 'M') issues.push({ type: 'camouflage-gender', ...row });
		rows.push(row);
	}
}
const camouflage = [...groups.values()].filter(g => g.length > 1).map(g => ({
	teams: g.map(t => t.id), identicalPublicSignature: new Set(g.map(publicSignature)).size === 1,
}));
const nashSetsEqualCatalog = nash.teams.every(t => {
	const cat = catalog.teams.find(c => c.id === t.id);
	return cat && JSON.stringify(cat.sets) === JSON.stringify(t.sets);
});
const hpOnlyAttackGaps = rows.filter(r => !r.hasPhysical && r.hasHiddenPower && !r.hasExternalMoveAccess
	&& (r.attackEv !== 0 || r.attackDv !== r.minimumDvAtSamePublicGenderAndHp));
const physicalLowDv = rows.filter(r => r.hasPhysical && r.attackDv < r.maximumDvAtSamePublicGenderAndHp)
	.map(r => ({ ...r, moves: catalog.teams.find(t => t.id === r.team).sets.find(s => s.species === r.species).moves,
		matchesPartyBoxSource: (() => {
			const source = partybox.teams.find(t => t.id === r.team)?.sets.find(s => s.species === r.species);
			return Boolean(source && dv(source, 'atk') === r.attackDv && source.evs.atk === r.attackEv);
		})() }));
const report = {
	format: 'nc2000-party-training-audit-v1',
	inputs: Object.fromEntries(Object.values(paths).map(p => [p, crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex')])),
	summary: { teams: catalog.teams.length, mons: rows.length,
		maximumNonAttackEvs: rows.every(r => !r.nonAttackEvDeficits.length),
		maximumNonAttackDvsSubjectToHiddenPower: rows.every(r => !r.nonAttackDvDeficits.length),
		lowAttackEligibleWithoutHiddenPower: rows.filter(r => !r.hasPhysical && !r.hasHiddenPower && !r.hasExternalMoveAccess).length,
		currentPolicyIssues: issues.length, camouflage, activeNashTeams: nash.teams.length, nashSetsEqualCatalog,
		specialHiddenPowerAttackGaps: hpOnlyAttackGaps.length, physicalLowDvExceptions: physicalLowDv.length },
	issues, specialHiddenPowerAttackGaps: hpOnlyAttackGaps, physicalLowDvExceptions: physicalLowDv, rows,
};
assert(process.argv[2], 'OUTPUT.json required');
fs.writeFileSync(process.argv[2], JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify(report.summary));
