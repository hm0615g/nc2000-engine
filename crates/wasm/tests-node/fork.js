// Forked battles through the wasm bridge, driven the way web/src/bot-worker.ts
// drives them: a mirror `Battle.fromFork`, the bot as a `ProtocolSearcher`
// installed at the fork's position and fed its `PlayerChannel` frames, the
// other side a fixed policy standing in for the human.
//
//   node crates/wasm/tests-node/fork.js
"use strict";

const { wasm, readData, check, checkEq, finish } = require("./common");

const dex = new wasm.Dex();
const poolJson = readData("meta-pool-v0/meta-pool.json");
const forkJson = readData("forks/4296-t11.json");
const fork = JSON.parse(forkJson);
const ITERS = 200;

const info = JSON.parse(wasm.forkInfo(dex, forkJson));
checkEq(info.arms.map((a) => a.input), ["switch 2", "move earthquake"], "arm inputs");
checkEq(info.botSide, 1, "bot side");
checkEq(info.turn, 11, "fork turn");

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

finish("fork");
