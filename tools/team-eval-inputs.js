// Inputs for the team-pool rebuild measurements (docs/TEAM-POOL-REBUILD-PLAN.md
// step 2): the team file every harness run reads, the measured-candidate id
// list, the held-out reference opponents, and the metric-calibration
// controls.
//
//   node tools/team-eval-inputs.js
//
// Output: data/team-inventory-v1/eval/teams.json   {teams:[{id, role, sets}]}
//         data/team-inventory-v1/eval/measured.txt  one id per line
//         data/team-inventory-v1/eval/heldout.txt
//         data/team-inventory-v1/eval/controls.txt
'use strict';
const fs = require('fs');
const path = require('path');

const REPO = path.join(__dirname, '..');
const DIR = path.join(REPO, 'data', 'team-inventory-v1', 'eval');
const inv = JSON.parse(fs.readFileSync(path.join(REPO, 'data/team-inventory-v1/inventory.json'), 'utf8'));
const learnsets = JSON.parse(fs.readFileSync(path.join(REPO, 'data/learnsets-gen2.json'), 'utf8'));
const wasm = require(path.join(REPO, 'crates/wasm/pkg-node/nc2000_wasm.js'));
const dex = new wasm.Dex();
const validator = new wasm.Validator(dex);
const byId = new Map(inv.teams.map(t => [t.id, t]));
const toid = s => String(s).toLowerCase().replace(/[^a-z0-9]/g, '');

const measured = [...new Set(inv.teams.map(t => t.measuredAs).filter(Boolean))];
// Legal human designs the intake does not admit (page variants no serious
// entry designates): opponents only, never candidates.
const heldout = inv.teams
	.filter(t => t.family === 'majinjima' && t.source.unlisted && t.sets.length === 6)
	.filter(t => JSON.parse(validator.validateTeam(JSON.stringify(t.sets))).ok)
	.map(t => t.id);

// ------------------------------------------------------------ controls
// Calibration arms on the ceiling team (the shipped Nash anchor): a floor
// with every move replaced by the least useful legal ones, two single-
// element deletions, and a confound the metric must ignore.
const JUNK = ['growl', 'tailwhip', 'leer', 'harden', 'defensecurl', 'withdraw', 'sandattack', 'smokescreen',
	'sweetscent', 'foresight', 'flash', 'charm', 'screech', 'scaryface', 'lockon', 'mindreader', 'teleport',
	'kinesis', 'splash', 'focusenergy', 'sharpen', 'bide', 'rage', 'mimic', 'tackle', 'scratch', 'pound', 'peck'];
const moveName = id => dex.moveName ? dex.moveName(id) : id;
function junkMoves(species) {
	const legal = new Set(learnsets.species[toid(species)].moves);
	return JUNK.filter(m => legal.has(m)).slice(0, 4);
}
function control(id, base, edit, note) {
	const sets = JSON.parse(JSON.stringify(byId.get(base).sets));
	edit(sets);
	const c = JSON.parse(validator.canonicalizeTeam(JSON.stringify(sets)));
	if (!c.ok) throw new Error(`${id}: ${JSON.stringify(c.errors)}`);
	const v = JSON.parse(validator.validateTeam(JSON.stringify(c.team)));
	if (!v.ok) throw new Error(`${id}: ${JSON.stringify(v.findings)}`);
	return { id, role: 'control', base, note, sets: c.team };
}
const CEILING = 'sample-07';
const controls = [
	control('ctl-floor-s07', CEILING, sets => {
		for (const s of sets) {
			const j = junkMoves(s.species);
			if (j.length < 1) throw new Error(`no junk moves for ${s.species}`);
			s.moves = j;
		}
	}, 'floor: species, levels and items kept; every move replaced by the least useful legal moves'),
	control('ctl-del-s07-gene', CEILING, sets => {
		sets.find(s => s.species === 'Tauros').item = '';
	}, 'deletion: Tauros loses Berserk Gene'),
	control('ctl-del-s07-boom', CEILING, sets => {
		const swap = (sp, from, to) => {
			const s = sets.find(x => x.species === sp);
			const i = s.moves.findIndex(m => toid(m) === from);
			if (i < 0) throw new Error(`${sp} lacks ${from}`);
			s.moves[i] = to;
		};
		swap('Electrode', 'explosion', 'Screech');
		swap('Cloyster', 'explosion', 'Withdraw');
		swap('Snorlax', 'selfdestruct', 'Defense Curl');
	}, 'deletion: the three self-KO moves replaced by junk (the explosion win condition removed)'),
	control('ctl-conf-s07-order', CEILING, sets => {
		sets.reverse();
	}, 'confound: display order reversed (strength must not move)'),
];

const teams = [
	...inv.teams.filter(t => t.eligibility.status === 'eligible').map(t => ({ id: t.id, role: 'candidate', sets: t.sets })),
	...heldout.map(id => ({ id, role: 'heldout', sets: byId.get(id).sets })),
	...controls,
];
fs.mkdirSync(DIR, { recursive: true });
fs.writeFileSync(path.join(DIR, 'teams.json'), JSON.stringify({ teams }, null, 1) + '\n');
fs.writeFileSync(path.join(DIR, 'measured.txt'), measured.join('\n') + '\n');
fs.writeFileSync(path.join(DIR, 'heldout.txt'), heldout.join('\n') + '\n');
fs.writeFileSync(path.join(DIR, 'controls.txt'), controls.map(c => c.id).join('\n') + '\n');
console.log(`${teams.length} teams (${measured.length} measured candidates, ${heldout.length} held out, ${controls.length} controls)`);
for (const c of controls) console.log(c.id, c.sets.map(s => `${s.species}[${s.moves.join('/')}]@${s.item || '-'}`).join(' '));
