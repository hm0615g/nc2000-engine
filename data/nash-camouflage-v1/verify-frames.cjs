// Run from the project root with a pools JSON file and the names of single-shell profiles.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const wasm = require('../../crates/wasm/pkg-node/nc2000_wasm.js');
const [file, ...names] = process.argv.slice(2);
const extra = JSON.parse(fs.readFileSync(file));
const incumbent = require('../kodoku-r3/conclusion-equilibrium-deme.json').teams;
const catalog = new Map([...require('../kodoku-beq-v1/novel_evolved_teams.json').teams,
  ...require('../team-pool-v2/team-pool.json').teams, ...incumbent, ...extra.teams].map(t => [t.id,t.sets]));
const dex = new wasm.Dex();
const validator = new wasm.Validator(dex);
for (const [name, members] of Object.entries(extra.mixtures)) {
  assert(Math.abs(members.reduce((s,t) => s+t.weight,0)-1)<1e-9,name);
  for (const t of members) {
    assert(t.weight>0);
    assert(JSON.parse(validator.validateTeam(JSON.stringify(catalog.get(t.id)))).ok,t.id);
  }
}
for (const name of names) {
  for (const side of [0,1]) {
    const visible = new Set();
    for (const {id} of name==='incumbent' ? incumbent : extra.mixtures[name]) {
      const teams = [catalog.get('sample-07'),catalog.get(id)].map(t=>JSON.stringify(t));
      if (side===1) teams.reverse();
      const b = new wasm.Battle(dex,...teams,wasm.deriveBattleSeed(901));
      const c = new wasm.PlayerChannel(side);
      visible.add(c.frame(b));
      c.free(); b.free();
    }
    assert.equal(visible.size,1,name);
  }
}
validator.free(); dex.free();
console.log(JSON.stringify({valid_pools:Object.keys(extra.mixtures).length,verified_public_shells:names.length}));
