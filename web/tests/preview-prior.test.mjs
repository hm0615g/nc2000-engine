import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs';
import crypto from 'node:crypto';
import { compatiblePolicyKeys, priorSnapshot, policyShells, teamMaterial, materialShell } from '../src/preview-prior.mjs';
const read = p => JSON.parse(fs.readFileSync(new URL(`../../${p}`, import.meta.url)));
const prior = read('data/belief-pool-v3/belief-pool.json');
const dex = read('data/gen2stadium2.json');
const artifact = read('data/compiled-preview-v1.json');
const digest = s => crypto.createHash('sha256').update(s).digest('hex');
const compatible = p => compatiblePolicyKeys(artifact, p, dex, digest);
const count = Object.keys(artifact.policies).length;

test('current bake retains every key', async () => assert.equal((await compatible(prior)).size, count));
test('metadata, formatting, order, engine-equivalent IVs and proportional weights preserve every key', async () => {
 const p = structuredClone(prior); p.note = 'changed'; p.teams.reverse();
 for (const t of p.teams) {
  t.id += '-renamed'; t.weight *= 2; t.sets.reverse();
  for (const s of t.sets) { s.name = 'nickname'; s.ivs.hp &= 30; s.moves = s.moves.map(m => m.toLowerCase()); }
 }
 assert.equal((await compatible(p)).size, count);
});
test('splitting identical variants preserves the distribution', async () => {
 const p = structuredClone(prior), t = p.teams.find(t => t.id === 'sample-07');
 t.weight /= 2; p.teams.push(structuredClone(t));
 assert.equal((await compatible(p)).size, count);
});
test('new unrelated shell preserves every existing key', async () => {
 const p = structuredClone(prior), t = structuredClone(p.teams[0]);
 t.sets[0].level = 51; p.teams.push(t);
 assert.equal((await compatible(p)).size, count);
});
test('new variant rejects both directions, retains unrelated keys', async () => {
 const p = structuredClone(prior), t = structuredClone(p.teams.find(t => t.id === 'sample-07'));
 const shell = materialShell(teamMaterial(t.sets, dex)); t.sets[0].moves[0] = 'Thunder'; p.teams.push(t);
 const actual = await compatible(p), deps = policyShells(artifact.policies, dex);
 let affected = 0;
 for (const [key, shells] of Object.entries(deps)) {
  const expected = !shells.includes(shell); assert.equal(actual.has(key), expected); affected += !expected;
 }
 assert.ok(affected > 0 && affected < count);
});
test('weight changes within a multi-type shell reject that shell', async () => {
 const p = structuredClone(prior), deps = policyShells(artifact.policies, dex);
 const before = await priorSnapshot(p, dex, digest);
 const t = p.teams.find(t => t.id === 'sample-07'); t.weight *= 2;
 const after = await priorSnapshot(p, dex, digest), shell = materialShell(teamMaterial(t.sets, dex));
 assert.notEqual(before[shell], after[shell]);
 const actual = await compatible(p);
 for (const [key, shells] of Object.entries(deps)) assert.equal(actual.has(key), !shells.includes(shell));
});
test('tiny weight changes are not rounded away', async () => {
 const p = structuredClone(prior); p.teams.find(t => t.id === 'sample-07').weight += 1e-12;
 assert.ok((await compatible(p)).size < count);
});
test('missing evidence and invalid weights fail closed', async () => {
 await assert.rejects(compatiblePolicyKeys({...artifact, priorDependencies: undefined}, prior, dex, digest));
 const p = structuredClone(prior); p.teams[0].weight = 0;
 await assert.rejects(compatible(p));
});

test('real Tokiwa variant invalidates exactly 62 sample-09 shell keys', async () => {
 const { createRequire } = await import('node:module');
 const { normalizeTraining } = createRequire(import.meta.url)('../../tools/normalize-training.js');
 const t = read('data/kodoku-r3/majinjima-assassins.json').teams.find(t => t.id === 'mjj-gym-Tokiwa');
 const sets = normalizeTraining(t.sets, dex);
 const p = { teams: prior.teams.filter(team => team.id !== 'mjj-gym-Tokiwa') };
 const valid = await compatible(p);
 assert.equal(count - valid.size, 62);
 const shell = materialShell(teamMaterial(sets, dex));
 for (const [key, dependencies] of Object.entries(policyShells(artifact.policies, dex)))
  assert.equal(valid.has(key), !dependencies.includes(shell));
});

test('deployed expansion keeps only catalog-generated types and has 990 reusable keys', async () => {
 const pool = read('data/belief-pool-v3/belief-pool.json');
 const bakePrior = read('data/belief-pool-v3/bake-prior.json');
 const catalog = read('data/team-pool-v2/team-pool.json');
 const types = new Set(catalog.teams.map(t => JSON.stringify(teamMaterial(t.sets, dex))));
 for (const t of pool.teams) if (t.provenance.origin !== 'human')
  assert.ok(types.has(JSON.stringify(teamMaterial(t.sets, dex))), t.id);
 assert.ok(pool.teams.some(t => t.id === 'mjj-2022-nrsk'));
 assert.ok(pool.teams.some(t => t.id === 'mjj-gym-Tokiwa'));
 assert.equal((await compatible(pool)).size, 990);
 assert.equal((await compatible(bakePrior)).size, 810);
 assert.equal(pool.teams.length, 411);
});
