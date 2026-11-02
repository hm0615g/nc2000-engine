// Verify the native 81k comparator against the actual WASM live search path.
const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const root = path.resolve(__dirname, "..");
const wasm = require(path.join(root, "crates/wasm/pkg-node/nc2000_wasm.js"));
const read = p => JSON.parse(fs.readFileSync(path.join(root, p), "utf8"));
const game = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const poolText = fs.readFileSync(path.join(root, "data/belief-pool-v3/belief-pool.json"), "utf8");
// A saved witness is self-contained: replay it without re-exporting all worlds.
if (game.format === "nc2000-ico-wasm-parity-v1") {
  const dex = new wasm.Dex();
  for (const record of game.cases) {
    const r = record.replay;
    const battle = new wasm.Battle(dex, JSON.stringify(r.own), JSON.stringify(r.foe), "1,2,3,4");
    const channel = new wasm.PlayerChannel(0);
    const preview = JSON.parse(channel.frame(battle));
    const search = new wasm.BlindSearcher(battle, 0, poolText, r.seed, .4, 16);
    assert.equal(search.compiledPreviewBuild(), game.build, "WASM build is stale");
    search.observe(battle);
    battle.applyChoice(0, r.own_pick);
    battle.applyChoice(1, r.foe_pick);
    const frame = JSON.parse(channel.frame(battle));
    const history = JSON.stringify([r.own, preview, r.own_pick, frame]);
    assert.equal(crypto.createHash("sha256").update(history).digest("hex"), record.visible_history_sha256);
    search.observe(battle);
    while (search.iterations() < 81000) search.step(Math.min(1000, 81000-search.iterations()));
    assert.equal(search.best(), record.choice);
    assert.deepEqual(JSON.parse(search.rootPolicy()), record.policy);
    search.free(); channel.free(); battle.free();
  }
  dex.free();
  const collisions = [];
  for (let i = 0; i < game.cases.length; i++) for (let j = i+1; j < game.cases.length; j++) {
    const a = game.cases[i], b = game.cases[j];
    if (a.visible_history_sha256 === b.visible_history_sha256 && a.choice !== b.choice)
      collisions.push([a.world,b.world]);
  }
  assert.deepEqual(collisions, game.hidden_state_choice_collisions);
  console.log(JSON.stringify({replayed:game.cases.length, hidden_state_choice_collisions:collisions}));
  process.exit(0);
}
const m = game.metadata;
const catalog = read("data/team-pool-v2/team-pool.json").teams;
const prior = JSON.parse(poolText).teams;
const own = read("data/kodoku-r3/conclusion-equilibrium-deme.json").teams;
const reference = catalog.find(t => t.id === m.opponent);
assert.equal(m.baseline_iterations, 81000);
assert.equal(m.baseline_records.length, m.worlds.length);
const dex = new wasm.Dex();
const indexes = process.argv[3] ? process.argv[3].split(",").map(Number) :
  [...new Set([0, Math.floor(m.worlds.length / 2), m.worlds.length - 1])];
const results = [];
for (const index of indexes) {
  const w = m.worlds[index], record = m.baseline_records[index];
  const a = own.find(t => t.id === w.own).sets;
  const raw = [...catalog, ...prior].find(t => t.id === w.foe).sets;
  const b = reference.sets.map(s => raw.find(t => t.species === s.species && t.level === s.level));
  assert(b.every(Boolean));
  const battle = new wasm.Battle(dex, JSON.stringify(a), JSON.stringify(b), "1,2,3,4");
  const channel = new wasm.PlayerChannel(0);
  const previewFrame = JSON.parse(channel.frame(battle));
  const search = new wasm.BlindSearcher(battle, 0, poolText, record.browser_constructor_seed, .4, 16);
  assert.equal(search.compiledPreviewBuild(), m.build, "WASM build is stale");
  search.observe(battle); // compiled preview consumes observe's seed, no search
  battle.applyChoice(0, w.own_pick);
  battle.applyChoice(1, w.foe_pick);
  const rootFrame = JSON.parse(channel.frame(battle));
  const visibleHistory = JSON.stringify([a, previewFrame, w.own_pick, rootFrame]);
  search.observe(battle);
  while (search.iterations() < 81000) search.step(Math.min(1000, 81000-search.iterations()));
  assert.equal(search.best(), record.choice);
  const policy = JSON.parse(search.rootPolicy());
  for (const action of policy.actions) {
    const nativeIndex = record.actions.indexOf(action.input);
    assert(nativeIndex >= 0);
    assert.equal(action.visits, record.visits[nativeIndex]);
    assert(Math.abs(action.mean-record.means[nativeIndex]) < 1e-12);
  }
  results.push({world:index, choice:search.best(), iterations:search.iterations(), policy,
    visible_history_sha256:crypto.createHash("sha256").update(visibleHistory).digest("hex"),
    replay:{own:a,foe:b,own_pick:w.own_pick,foe_pick:w.foe_pick,seed:record.browser_constructor_seed}});
  search.free();channel.free();battle.free();
}
dex.free();
const collisions=[];
for (let i=0;i<results.length;i++) for (let j=i+1;j<results.length;j++) {
  if(results[i].visible_history_sha256===results[j].visible_history_sha256 && results[i].choice!==results[j].choice)
    collisions.push([results[i].world,results[j].world]);
}
console.log(JSON.stringify({format:"nc2000-ico-wasm-parity-v1", build:m.build, cases:results,
  hidden_state_choice_collisions:collisions}, null, 2));
