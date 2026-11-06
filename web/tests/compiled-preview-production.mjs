// Exercise the actual production worker bundle, locally or on deployed Pages.
// NC2000_PREVIEW_CHECK_URL=https://.../ overrides the local preview server.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { preview } from 'vite';
import { chromium } from '@playwright/test';
import { createRequire } from 'node:module';
import { compatiblePolicyKeys } from '../src/preview-prior.mjs';
const wasm = createRequire(import.meta.url)('../../crates/wasm/pkg-node/nc2000_wasm.js');
const frameDex = new wasm.Dex();

let server, browser;
try {
  const remote = process.env.NC2000_PREVIEW_CHECK_URL;
  if (!remote) server = await preview({ preview: { host: '127.0.0.1', port: 0 } });
  const address = server?.httpServer.address();
  const base = remote || new URL(server.config.base, `http://127.0.0.1:${address.port}`).href;
  const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
  const expected = {
    prior: hash('../data/belief-pool-v3/belief-pool.json'),
    catalog: hash('../data/team-pool-v2/team-pool.json'),
  };
  const artifact = JSON.parse(fs.readFileSync('../data/compiled-preview-v1.json', 'utf8'));
  assert.equal(artifact.status, undefined, 'production previews must be active');
  const compatible = await compatiblePolicyKeys(artifact,
    JSON.parse(fs.readFileSync('../data/belief-pool-v3/belief-pool.json')),
    JSON.parse(fs.readFileSync('../data/gen2stadium2.json')),
    text => crypto.createHash('sha256').update(text).digest('hex'));
  assert.ok(compatible.size > 0, "expected partially reusable bake");
  const poolJson = fs.readFileSync("../data/belief-pool-v3/belief-pool.json", "utf8");
  browser = await chromium.launch();
  const page = await browser.newPage();
  await page.exposeFunction('__playerFrame', (own, foe, side) => {
    const teams = side === 0 ? [own, foe] : [foe, own];
    const battle = new wasm.Battle(frameDex, JSON.stringify(teams[0]), JSON.stringify(teams[1]), '1,2,3,4');
    const channel = new wasm.PlayerChannel(side);
    try {
      const searcher = new wasm.BlindSearcher(battle, side, poolJson, 7, .4);
      try { return {...JSON.parse(channel.frame(battle)), expectedTable: compatible.has(searcher.compiledPreviewKey(battle))}; }
      finally { searcher.free(); }
    }
    finally { channel.free(); battle.free(); }
  });
  const errors = [];
  page.on('pageerror', e => errors.push(String(e)));
  await page.goto(base, { waitUntil: 'networkidle' });
  const result = await page.evaluate(async ({ base, expected }) => {
    const main = [...document.querySelectorAll('script[type="module"][src]')][0];
    if (!main) throw Error('missing production entry');
    const source = await (await fetch(main.src)).text();
    const match = source.match(/[\w./-]*\/assets\/bot-worker-[\w-]+\.js/);
    if (!match) throw Error('missing production worker asset');
    const workerUrl = new URL(match[0], base).href;
    const digest = async text => [...new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text)))].map(n => n.toString(16).padStart(2, '0')).join('');
    const nonce = Date.now();
    const prior = await (await fetch(new URL(`data/belief-pool-v3/belief-pool.json?verify=${nonce}`, base))).text();
    const catalogText = await (await fetch(new URL(`data/team-pool-v2/team-pool.json?verify=${nonce}`, base))).text();
    if (await digest(prior) !== expected.prior || await digest(catalogText) !== expected.catalog)
      throw Error('served data differs from verified products');
    const catalog = JSON.parse(catalogText).teams;
    const worker = new Worker(workerUrl, { type: 'module' });
    let id = 0, progress = 0, resolveReady, rejectReady;
    const pending = new Map();
    const ready = new Promise((resolve, reject) => { resolveReady = resolve; rejectReady = reject; });
    worker.onerror = e => { rejectReady(Error(e.message)); for (const p of pending.values()) p.reject(Error(e.message)); };
    worker.onmessage = e => {
      const m = e.data;
      if (m.t === 'ready') resolveReady();
      if (m.t === 'progress') progress++;
      if (m.t === 'error') { rejectReady(Error(m.message)); for (const p of pending.values()) p.reject(Error(m.message)); }
      if (m.t === 'result') { const p = pending.get(m.id); if (p) { pending.delete(m.id); clearTimeout(p.timeout); p.resolve(m); } }
    };
    const readyTimeout = setTimeout(() => rejectReady(Error('worker initialization timeout')), 30000);
    await ready; clearTimeout(readyTimeout);
    const timings = [], roundTrips = [], byPublic = new Map();
    const run = async (own, foe, side, poolJson = prior, seed = 7) => {
      const {expectedTable, ...frame} = await window.__playerFrame(own, foe, side);
      // The native fixture/oracle is test infrastructure, not worker latency.
      const began = performance.now();
      worker.postMessage({ t: 'battle', ownTeam: JSON.stringify(own), searcher: { poolJson, side, seed } });
      const result = new Promise((resolve, reject) => {
        const request = ++id;
        const timeout = setTimeout(() => { pending.delete(request); reject(Error(`preview timeout ${request}`)); }, 10000);
        pending.set(request, { resolve, reject, timeout });
        worker.postMessage({ t: 'search', id: request, side, budget: 1, seed: 99, ponder: true, frame });
      });
      const r = await result;
      return { ...r, expectedTable, roundTripMs: performance.now() - began };
    };
    let checked = 0, tableCases = 0, liveCases = 0, firstValidOwn;
    try {
      for (const own of catalog) for (const foe of catalog) for (const side of [0, 1]) {
        const r = await run(own.sets, foe.sets, side);
        const policy = JSON.parse(r.policy);
        if (r.expectedTable) {
          if (r.src !== 'table' || policy.iterations !== 0 || !r.best) throw Error(`valid table missed: ${own.id}/${foe.id}/side${side}`);
          tableCases++; firstValidOwn ??= own;
        } else {
          if (r.src !== 'search' || policy.iterations !== 1 || !r.best) throw Error(`invalid table used: ${own.id}/${foe.id}/side${side}`);
          liveCases++; checked++; continue;
        }
        const visible = JSON.stringify(foe.sets.map(s => [s.species, s.level, s.gender, Boolean(s.item)]));
        const key = `${own.id}:${visible}`;
        if (byPublic.has(key) && byPublic.get(key) !== r.best) throw Error('hidden opponent set or side changed the table policy');
        byPublic.set(key, r.best);
        if (!Number.isFinite(r.previewLookupMs)) throw Error('missing lookup timing');
        timings.push(r.previewLookupMs); roundTrips.push(r.roundTripMs); checked++;
      }
      if (!tableCases || !firstValidOwn) throw Error('no usable table');
      const own = firstValidOwn.sets, foe = own;
      const reference = await run(own, foe, 1);
      const hidden = structuredClone(foe); hidden[0].item = 'Polkadot Bow'; hidden[0].moves[0] = 'Thunder';
      const hiddenResult = await run(own, hidden, 1);
      if (hiddenResult.src !== 'table' || hiddenResult.best !== reference.best) throw Error('unobserved loadout affected lookup');
      const ownChange = structuredClone(own); ownChange[0].happiness = 254;
      const genderChange = structuredClone(foe); const genderMon = genderChange.find(s => s.gender === 'M' || s.gender === 'F'); genderMon.gender = genderMon.gender === 'M' ? 'F' : 'M';
      const itemChange = structuredClone(foe); itemChange[0].item = '';
      const formatted = await run(own, foe, 1, prior + ' ');
      if (formatted.src !== 'table') throw Error('format-only prior change invalidated preview');
      const changed = JSON.parse(prior);
      const variant = structuredClone({id: 'test-new-variant', sets: own, weight: 1});
      variant.id = 'test-new-variant'; variant.sets[0].happiness = 254;
      changed.teams.push(variant);
      const changedPrior = JSON.stringify(changed);
      let unrelated;
      for (const t of catalog) {
        if (t.id !== firstValidOwn.id && (await run(t.sets, t.sets, 1, changedPrior)).src === 'table') { unrelated = t.sets; break; }
      }
      if (!unrelated) throw Error('no unrelated reusable shell');
      if ((await run(unrelated, unrelated, 1, changedPrior)).src !== 'table') throw Error('unrelated shell invalidated');
      const controls = [];
      for (const [name, a, b, p] of [
        ['own-set', ownChange, foe, prior], ['prior-opponent-shell', unrelated, foe, changedPrior],
        ['prior-own-shell', own, unrelated, changedPrior],
        ['public-gender', own, genderChange, prior], ['public-item-presence', own, itemChange, prior],
      ]) {
        const r = await run(a, b, 1, p);
        if (r.src !== 'search' || JSON.parse(r.policy).iterations !== 1) throw Error(`invalid cache acceptance: ${name}`);
        controls.push(name);
      }
      const summary = values => { const s = [...values].sort((a, b) => a - b); return { median: s[Math.floor(s.length / 2)], p95: s[Math.floor(s.length * .95)], max: s.at(-1) }; };
      const lookupMs = summary(timings), battleInitAndLookupMs = summary(roundTrips);
      if (lookupMs.p95 >= 20 || battleInitAndLookupMs.p95 >= 100) throw Error(`preview latency exceeded gate: ${JSON.stringify({lookupMs, battleInitAndLookupMs})}`);
      return { base, workerUrl, catalogPairs: catalog.length ** 2, sideCases: checked, tableCases, liveCases, livePreviewTestBudget: 1,
        lookupMs, battleInitAndLookupMs, hiddenLoadoutInvariant: true, rejectionControls: controls,
        servedDataSha256: expected };
    } finally { worker.terminate(); }
  }, { base, expected });
  assert.equal(result.catalogPairs, 1089); assert.equal(result.sideCases, 2178); assert.deepEqual(errors, []);
  const output = process.env.NC2000_PREVIEW_CHECK_OUT || '../data/preview-training-v3/production-verification.json';
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, JSON.stringify(result, null, 2) + '\n');
  console.log(JSON.stringify(result, null, 2));
} finally {
  frameDex.free();
  await browser?.close();
  if (server) await new Promise(resolve => server.httpServer.close(resolve));
}
