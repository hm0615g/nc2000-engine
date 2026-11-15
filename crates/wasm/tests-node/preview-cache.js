'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {createHash} = require('node:crypto');
const {wasm, REPO, readData} = require('./common');
const {createPreviewCache} = require('../../../tools/preview-cache');

const poolJson = readData('belief-pool-v3/belief-pool.json');
const catalog = JSON.parse(readData('team-pool-v2/team-pool.json')).teams;
const own = catalog[0].sets, foe = catalog[1].sets;
const dex = new wasm.Dex();
const options = {
	wasmSha256: createHash('sha256').update(fs.readFileSync(path.join(REPO, 'crates/wasm/pkg-node/nc2000_wasm_bg.wasm'))).digest('hex'),
	dexJson: wasm.embeddedDexJson(), iterations: 128,
};
const cache = createPreviewCache(options);
const objects = [];
function blind({ours = own, theirs = foe, seed = 7, c = .4, pool = poolJson, prior, pickPrior, buckets, pinBeforeObserve = false} = {}) {
	const b = new wasm.Battle(dex, JSON.stringify(ours), JSON.stringify(theirs), '1,2,3,4');
	const s = new wasm.BlindSearcher(b, 0, pool, seed, c, buckets);
	objects.push(s, b);
	assert.equal(s.previewCacheInfo(), undefined);
	if (prior) assert.equal(JSON.parse(s.setBeliefPrior(prior)).applied, true);
	if (pickPrior) assert.equal(JSON.parse(s.setPickPrior(pickPrior)).applied, true);
	if (pinBeforeObserve) s.pinOpponent(JSON.stringify(theirs));
	s.observe(b);
	return {s, b};
}
const details = p => `${p.species}, L${p.level}${p.gender === 'M' || p.gender === 'F' ? ', ' + p.gender : ''}`;
function protocol(seed = 7, {pickPrior} = {}) {
	const s = new wasm.ProtocolSearcher(dex, 0, poolJson, seed, .4);
	objects.push(s); s.setOwnTeam(JSON.stringify(own));
	if (pickPrior) assert.equal(JSON.parse(s.setPickPrior(pickPrior)).applied, true);
	const lines = [['p1', own], ['p2', foe]].flatMap(([tag, team]) => team.map(p => `|poke|${tag}|${details(p)}|${p.item ? 'item' : ''}`));
	s.pushLines(JSON.stringify([...lines, '|teampreview']));
	assert.equal(s.onRequest(JSON.stringify({teamPreview: true, rqid: 1, side: {id: 'p1', pokemon: own.map(p => ({ident: `p1: ${p.name || p.species}`, details: details(p), condition: '100/100', item: p.item, active: false}))}})), true);
	return s;
}
try {
	const {s, b} = blind(); const key = cache.key(s);
	assert.match(key, /^[a-f0-9]{64}$/);
	assert.throws(() => cache.createRecord(s), /exactly 128/);
	s.step(64); assert.throws(() => cache.createRecord(s), /exactly 128/);
	s.step(64); assert.equal(cache.key(s), key, 'root visit sorting must not change the key');
	const record = cache.createRecord(s);
	const fresh = blind().s;
	assert.deepEqual(cache.readRecord(fresh, record), {choice: s.best(), source: 'cache', iterations: 0, sourceIterations: 128});
	assert.equal(fresh.iterations(), 0);
	assert.equal(cache.readRecord(blind({seed: 999}).s, record).choice, s.best(), 'reuse across battle seeds');
	for (const text of ['{', 'null', '{}', JSON.stringify({...JSON.parse(record), choice: 'move tackle'}), JSON.stringify({...JSON.parse(record), checksum: '0'})]) assert.equal(cache.readRecord(fresh, text), null);
	for (const override of [{iterations: 129}, {wasmSha256: '0'.repeat(64)}, {dexJson: '{}'}, {formatId: 'other'}]) assert.equal(createPreviewCache({...options, ...override}).readRecord(fresh, record), null);
	assert.equal(cache.readRecord(blind({c: .6}).s, record), null);
	assert.equal(cache.readRecord(blind({buckets: 8}).s, record), null);
	assert.equal(cache.readRecord(blind({pool: JSON.stringify({...JSON.parse(poolJson), cacheTest: true})}).s, record), null);
	assert.equal(cache.readRecord(blind({pool: JSON.stringify(JSON.parse(poolJson), null, 2)}).s, record).choice, s.best());
	assert.equal(cache.readRecord(blind({prior: readData('belief-prior-v0.sample.json')}).s, record), null);
	const hidden = structuredClone(foe); hidden[0].moves = ['Tackle']; hidden[0].item = hidden[0].item === 'Leftovers' ? 'Miracle Berry' : 'Leftovers';
	assert.equal(cache.key(blind({theirs: hidden}).s), key, 'hidden moves and item identity must not leak');
	const noItem = structuredClone(foe); noItem[0].item = '';
	assert.notEqual(cache.key(blind({theirs: noItem}).s), key, 'public item presence must distinguish keys');
	const gender = structuredClone(foe); const slot = gender.findIndex(p => p.gender === 'M' || p.gender === 'F');
	gender[slot].gender = gender[slot].gender === 'M' ? 'F' : 'M';
	assert.notEqual(cache.key(blind({theirs: gender}).s), key, 'public gender must distinguish keys');
	const ownChanged = structuredClone(own); ownChanged[0].moves = ['Tackle'];
	assert.notEqual(cache.key(blind({ours: ownChanged}).s), key);
	const pinned = blind({pinBeforeObserve: true}).s;
	assert.equal(cache.key(pinned), null);
	const alreadyObserved = blind().s;
	assert.throws(() => alreadyObserved.pinOpponent(JSON.stringify(foe)), /before observe/);

	// Lookup after observation must preserve the later decision's RNG and belief.
	b.applyChoice(0, cache.readRecord(fresh, record).choice);
	b.applyChoice(1, JSON.parse(b.legalChoices(1))[0].input);
	for (const searcher of [s, fresh]) { searcher.observe(b); assert.equal(cache.key(searcher), null); searcher.step(128); }
	assert.equal(fresh.rootPolicy(), s.rootPolicy()); assert.equal(fresh.best(), s.best());

	const ps = protocol(); const protocolKey = cache.key(ps);
	assert(protocolKey); assert.notEqual(protocolKey, key, 'different drivers do not share entries');
	ps.step(128); const protocolRecord = cache.createRecord(ps); const ps2 = protocol(33);
	assert.equal(cache.readRecord(ps2, protocolRecord).choice, ps.best());
	ps2.step(1); assert.equal(cache.readRecord(ps2, protocolRecord), null, 'never replace a partial search');
	ps.step(1); assert.throws(() => cache.createRecord(ps), /exactly 128/);
	ps2.onRequest(JSON.stringify({wait: true})); assert.equal(cache.key(ps2), null);
	const pinnedPs = protocol();
	assert.ok(JSON.parse(pinnedPs.beliefInfo()).count > 1);
	pinnedPs.pinOpponent(JSON.stringify(foe));
	assert.equal(JSON.parse(pinnedPs.beliefInfo()).count, 1);
	assert.equal(cache.key(pinnedPs), null);

	const toId = name => String(name).toLowerCase().replace(/[^a-z0-9]/g, '');
	let legalPick = null;
	for (let i = 0; i < foe.length && !legalPick; i++)
		for (let j = i + 1; j < foe.length && !legalPick; j++)
			for (let k = j + 1; k < foe.length && !legalPick; k++)
				if ((foe[i].level ?? 50) + (foe[j].level ?? 50) + (foe[k].level ?? 50) <= 155)
					legalPick = [i, j, k].map(n => toId(foe[n].species));
	const foeIds = JSON.parse(poolJson).teams
		.filter(t => JSON.stringify(t.sets.map(x => toId(x.species)).sort()) === JSON.stringify(foe.map(x => toId(x.species)).sort()))
		.map(t => t.id);
	const pickPrior = JSON.stringify({
		schema: 'nc2000-pick-prior-v1',
		smoothing: 0.001,
		backoff_strength: 1.0,
		source: {},
		rows: foeIds.map(team => ({
			team,
			enemy_preview: own.map(p => ({species: toId(p.species), level: p.level ?? 50, gender: p.gender === 'M' || p.gender === 'F' ? p.gender : '', item: Boolean(p.item)})),
			side: 1,
			choices: [{species: legalPick, count: 500}],
		})),
	});
	assert.equal(JSON.parse(wasm.probePickPrior(pickPrior)).applied, true);
	const basePreviewInfo = JSON.parse(blind().s.previewCacheInfo());
	assert.equal(Object.hasOwn(basePreviewInfo, 'pickPrior'), false);
	const ppBlind = blind({pickPrior}).s;
	assert.deepEqual(JSON.parse(ppBlind.pickPriorInfo()), {installed: true, active: true});
	assert.equal(Object.hasOwn(JSON.parse(ppBlind.previewCacheInfo()), 'pickPrior'), true);
	assert.equal(cache.readRecord(ppBlind, record), null, 'PickPrior must invalidate cached preview');
	const ppPs = protocol(7, {pickPrior});
	assert.deepEqual(JSON.parse(ppPs.pickPriorInfo()), {installed: true, active: true});
	assert.equal(Object.hasOwn(JSON.parse(ppPs.previewCacheInfo()), 'pickPrior'), true);
	assert.equal(cache.readRecord(ppPs, protocolRecord), null, 'PickPrior must invalidate protocol cached preview');
	assert.equal(JSON.parse(ps.setPickPrior(pickPrior)).applied, false, 'late ProtocolSearcher.setPickPrior must be rejected');
	assert.equal(JSON.parse(pinnedPs.setPickPrior(pickPrior)).applied, false);
	console.log('preview-cache: passed (real WASM, both drivers, invalidation, public-only identity, RNG continuation)');
} finally {
	for (const object of objects) object.free();
	dex.free();
}
