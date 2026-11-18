// Build the Node WASM target first. Run from any directory:
// node web/tests/nash-refresh.mjs GAMES.jsonl NEW_OUTPUT_DIR [workers]
// Each game specifies the two team IDs, their search seeds and the battle seed.
// Optional picks force a preview action in a paired counterfactual.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Worker, isMainThread, parentPort, workerData } from 'node:worker_threads';
import { build } from 'vite';
import wasm from '../../crates/wasm/pkg-node/nc2000_wasm.js';

const root = fileURLToPath(new URL('../../', import.meta.url));
const read = name => fs.readFileSync(path.join(root, name), 'utf8');
const data = name => JSON.parse(read('data/' + name + '.json'));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const compiled = path.join(root, 'web/src/compiled-preview.ts');
const live = data('live-search');
const { c } = data('search-profiles').blind;

const teamFiles = [
  'kodoku-beq-v1/novel_evolved_teams',
  'team-pool-v2/team-pool',
  'kodoku-r3/conclusion-equilibrium-deme',
];

async function start() {
  const [, , taskFile, output, n = '8'] = process.argv;
  const source = fs.readFileSync(taskFile, 'utf8').trim();
  const records = source.startsWith('[')
    ? JSON.parse(source)
    : source.split('\n').map(line => JSON.parse(line));
  const tasks = records.map(({ block, cell, rowSeat, ids, seeds, battleSeed, picks }) =>
    ({ block, cell, rowSeat, ids, seeds, battleSeed, ...(picks ? { picks } : {}) }));
  assert(tasks.length && Number.isSafeInteger(+n) && +n > 0);
  fs.mkdirSync(output); // refuse to mix builds or overwrite an earlier run
  const bundle = await build({ configFile: false, logLevel: 'silent',
    define: { __COMPILED_PREVIEW_ENABLED__: 'true' },
    build: { write: false, lib: { entry: compiled, formats: ['es'], fileName: 'picker' } },
  });
  const picker = 'data:text/javascript;base64,' + Buffer.from(bundle[0].output[0].code).toString('base64');
  const inputs = ['compiled-preview-v1', 'belief-pool-v3/belief-pool', ...teamFiles, 'live-search', 'search-profiles'];
  const stamp = {
    tasks: hash(fs.readFileSync(taskFile)),
    wasm: hash(fs.readFileSync(path.join(root, 'crates/wasm/pkg-node/nc2000_wasm_bg.wasm'))),
    picker: hash(picker),
    data: Object.fromEntries(inputs.map(k => [k, hash(read('data/' + k + '.json'))])),
  };
  fs.writeFileSync(path.join(output, 'manifest.json'), JSON.stringify(stamp));
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(+n, tasks.length) }, () => new Promise((resolve, reject) => {
    const thread = new Worker(new URL(import.meta.url), { workerData: { picker } });
    thread.on('error', reject);
    thread.on('exit', code => code ? reject(Error('worker exit ' + code)) : resolve());
    thread.on('message', result => {
      if (result) fs.writeFileSync(path.join(output, result.index + '.json'), JSON.stringify(result));
      if (next === tasks.length) thread.postMessage(null);
      else {
        const index = next++;
        thread.postMessage({ ...tasks[index], index });
      }
    });
  })));
  console.log(JSON.stringify({ games: tasks.length, output }));
}

async function worker() {
  const pool = read('data/belief-pool-v3/belief-pool.json');
  const { compiledPreviewCompatibility, compiledPreviewPick } = await import(workerData.picker);
  const allowed = await compiledPreviewCompatibility(pool, wasm.embeddedDexJson());
  assert(allowed.size > 0);
  const dex = new wasm.Dex();
  const validator = new wasm.Validator(dex);
  const teams = new Map();
  for (const name of teamFiles) {
    for (const t of data(name).teams) teams.set(t.id, JSON.stringify(t.sets));
  }
  parentPort.on('message', task => {
    if (!task) { parentPort.close(); return; }
    const { ids, seeds, battleSeed, picks = [] } = task;
    const sets = ids.map(id => {
      const team = teams.get(id);
      assert(team && JSON.parse(validator.validateTeam(team)).ok, 'invalid ' + id);
      return team;
    });
    const battle = new wasm.Battle(dex, ...sets, wasm.deriveBattleSeed(battleSeed));
    const channels = [new wasm.PlayerChannel(0), new wasm.PlayerChannel(1)];
    const agents = sets.map((team, side) => {
      const agent = new wasm.ProtocolSearcher(dex, side, pool, seeds[side], c);
      agent.setOwnTeam(team);
      return agent;
    });
    const preview = [];
    const began = performance.now();
    let first = true;
    while (!battle.outcome() && battle.turn() < 500) {
      const inputs = JSON.parse(battle.needsChoice()).map((needs, side) => {
        if (!needs) return null;
        const frame = JSON.parse(channels[side].frame(battle));
        const agent = agents[side];
        agent.pushLines(JSON.stringify(frame.lines));
        assert(agent.onRequest(JSON.stringify(frame.request)));
        let input;
        let source = 'live';
        if (first) {
          input = compiledPreviewPick(agent.compiledPreviewBuild(), agent.compiledPreviewKey(),
            seeds[side], c, agent.previewPublicInfo(), allowed);
          const legal = pick => frame.legal_actions.some(a => a.replaceAll(' ', '') === pick.replaceAll(' ', ''));
          if (input && legal(input)) source = 'table';
          else input = null;
          if (picks[side]) {
            assert(legal(picks[side]), 'illegal forced selection');
            source = 'forced';
            input = picks[side];
          }
        }
        if (!input) {
          agent.stepAdaptiveBattle(first ? live.unknownPreview : live.battle.adaptive);
          input = JSON.parse(agent.commitDecision('legacy')).input;
        }
        if (first) preview[side] = { source, input };
        return input;
      });
      for (const [side, input] of inputs.entries()) if (input) battle.applyChoice(side, input);
      first = false;
    }
    const result = { ...task, preview, outcome: battle.outcome() ?? 'cap', turns: battle.turn(),
      ms: Math.round(performance.now() - began) };
    for (const object of [...agents, ...channels, battle]) object.free();
    parentPort.postMessage(result);
  });
  parentPort.postMessage(null);
}

if (isMainThread) await start(); else await worker();
