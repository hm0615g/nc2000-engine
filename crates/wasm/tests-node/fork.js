// Forked battles through the wasm bridge, driven the way web/src/bot-worker.ts
// drives them: a mirror `Battle.fromFork`, the bot as a `ProtocolSearcher`
// installed at the fork's position and fed its `PlayerChannel` frames, the
// other side a fixed policy standing in for the human.
//
// `ForkArena` runs the native `fork_counterfactual` trials; with
// NC2000_NATIVE_PARITY=1 its rows are compared with the native binary's.
//
//   node crates/wasm/tests-node/fork.js
//   NC2000_NATIVE_PARITY=1 node crates/wasm/tests-node/fork.js
"use strict";

const path = require("path");
const { execFileSync } = require("child_process");
const { wasm, REPO, readData, check, checkEq, finish } = require("./common");

const dex = new wasm.Dex();
const poolJson = readData("meta-pool-v0/meta-pool.json");
const forkJson = readData("forks/4296-t11.json");
const fork = JSON.parse(forkJson);
const ITERS = 200;

const info = JSON.parse(wasm.forkInfo(dex, forkJson));
checkEq(info.arms.map((a) => a.input), ["switch 2", "move earthquake"], "arm inputs");
checkEq(info.botSide, 1, "bot side");
checkEq(info.turn, 11, "fork turn");
checkEq(info.opponentView, true, "the opponent's own view is present");

let rejected = false;
try {
  const bad = JSON.parse(forkJson);
  bad.arms.push({ input: "move thunderbolt", label: "" });
  wasm.forkInfo(dex, JSON.stringify(bad));
} catch (e) {
  rejected = String(e).includes("not legal");
}
check(rejected, "an illegal arm is rejected with its reason");

function play(armIndex, seed, mode) {
  const bot = info.botSide;
  const human = 1 - bot;
  const battle = wasm.Battle.fromFork(dex, forkJson, seed);
  const channel = new wasm.PlayerChannel(bot);
  const searcher = new wasm.ProtocolSearcher(dex, bot, poolJson, seed, 0.4);
  if (mode === "open") searcher.pinOpponent(JSON.stringify(fork.opponent_team));
  searcher.setPosition(JSON.stringify(fork.position));
  let positioned = true;
  let steps = 0;
  const log = [];
  while (battle.outcome() === undefined && steps < 3000) {
    const needs = JSON.parse(battle.needsChoice());
    const picks = [];
    if (needs[human]) picks.push([human, JSON.parse(battle.legalChoices(human))[0].input]);
    if (needs[bot]) {
      const legal = JSON.parse(battle.legalChoices(bot));
      let input;
      if (steps === 0) {
        input = info.arms[armIndex].input;
        const frame = JSON.parse(channel.frame(battle));
        check(frame.lines.length === 0, "the fork point carries no history");
        positioned = false;
      } else if (legal.length === 1) {
        input = legal[0].input;
      } else {
        const frame = JSON.parse(channel.frame(battle));
        if (!(positioned && frame.lines.length === 0)) {
          searcher.pushLines(JSON.stringify(frame.lines));
          check(searcher.onRequest(JSON.stringify(frame.request)), "bot owes a decision");
        }
        positioned = false;
        searcher.step(ITERS);
        input = battle.resolveChoice(bot, searcher.best());
      }
      check(legal.some((c) => c.input === input), `bot input ${input} is legal`);
      picks.push([bot, input]);
    }
    for (const [side, input] of picks) battle.applyChoice(side, input);
    log.push(...JSON.parse(battle.takeNewLog()));
    steps += 1;
  }
  const metrics = JSON.parse(searcher.metrics());
  return { outcome: battle.outcome(), steps, log, metrics, seed: battle.seed() };
}

for (const mode of ["blind", "open"]) {
  for (const arm of [0, 1]) {
    for (const seed of [1, 2]) {
      const a = play(arm, seed, mode);
      check(a.outcome !== undefined, `${mode} arm ${arm} seed ${seed} finishes`);
      checkEq(a.metrics, { legalityDrift: 0, projections: 0 }, `${mode} arm ${arm} seed ${seed} legality`);
      const b = play(arm, seed, mode);
      checkEq(b.log, a.log, `${mode} arm ${arm} seed ${seed} replays identically`);
    }
  }
}

const arenaConfig = {
  bot: "protocol",
  foe: "protocol",
  iters: 200,
  foe_iters: 200,
  c: 0.4,
  seed: 11,
  max_steps: 3300,
};
const arena = new wasm.ForkArena(dex, forkJson, poolJson, JSON.stringify(arenaConfig));
const rows = [];
for (let trial = 0; trial < 3; trial++)
  for (let arm = 0; arm < info.arms.length; arm++) rows.push(JSON.parse(arena.play(trial, arm)));
checkEq(JSON.parse(arena.play(2, 1)), rows[5], "an arena trial replays identically");
for (const row of rows) {
  check(["win", "loss", "tie"].includes(row.outcome), `arena outcome ${row.outcome}`);
  checkEq([row.legality_drift, row.projections], [0, 0], "arena legality");
}
checkEq(rows[0].battle_seed, rows[1].battle_seed, "arms of a trial share the battle seed");

if (process.env.NC2000_NATIVE_PARITY === "1") {
  const bin = path.join(REPO, "target/release/examples/fork_counterfactual");
  const out = execFileSync(bin, [
    "--fork", path.join(REPO, "data/forks/4296-t11.json"),
    "--trials", "3", "--seed", "11", "--iters", "200", "--foe-iters", "200",
    "--bot", "protocol", "--foe", "protocol", "--c", "0.4", "--threads", "1",
  ], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  const native = out.trim().split("\n").map((l) => {
    const row = JSON.parse(l);
    delete row.elapsed_ms;
    return row;
  });
  checkEq(rows, native, "wasm arena rows equal the native binary's");
}

finish("fork");
