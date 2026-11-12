"use strict";

const assert = require("node:assert/strict");
const { wasm, readData } = require("./common");
const set = (species, moves, item = "") => ({
  species, name: species, moves, item, level: 50, happiness: 255,
  evs: { hp: 255, atk: 255, def: 255, spa: 255, spd: 255, spe: 255 },
  ivs: { hp: 30, atk: 30, def: 30, spa: 30, spd: 30, spe: 30 },
});
const poolJson = readData("belief-pool-v3/belief-pool.json");
const ownJson = JSON.stringify([set("Jolteon", ["Return", "Thunder Shock", "Toxic", "Thunder Wave"])]);
const foeJson = JSON.stringify([set("Wobbuffet", ["Counter", "Mirror Coat", "Safeguard", "Destiny Bond"], "Leftovers")]);
const prepareBattle = (dex) => {
  const b = new wasm.Battle(dex, ownJson, foeJson, "1,2,3,1");
  const step = (x, y) => { b.applyChoice(0, x); b.applyChoice(1, y); };
  step("team 1", "team 1");
  step("move return", "move counter");
  step("move thundershock", "move mirrorcoat");
  step("move toxic", "move mirrorcoat");
  for (let i = 0; i < 4; i++) step("move thunderwave", "move mirrorcoat");
  return b;
};
const dex = new wasm.Dex();
try {
  for (const seed of [82001, 82002]) {
    let best;
    const base = prepareBattle(dex);
    const search = new wasm.BlindSearcher(base, 0, poolJson, seed, 0.4);
    try {
      search.observe(base);
      search.step(3000);
      best = search.best();
      assert.ok(["move toxic", "move thunderwave"].includes(best), best);
    } finally {
      search.free();
      base.free();
    }
    for (const reply of ["counter", "mirrorcoat", "safeguard", "destinybond"]) {
      const battle = prepareBattle(dex);
      try {
        battle.applyChoice(0, best);
        battle.applyChoice(1, `move ${reply}`);
        assert.equal(battle.outcome(), "p1");
      } finally {
        battle.free();
      }
    }
  }
} finally {
  dex.free();
}
console.log("residual-wait: OK");
