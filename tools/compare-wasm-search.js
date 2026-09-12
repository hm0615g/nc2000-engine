#!/usr/bin/env node
"use strict";
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const [input, output, aC, bC, aIters, bIters, limitText, filter] = process.argv.slice(2);
const configs = [{ c: Number(aC), iterations: Number(aIters) },
  { c: Number(bC), iterations: Number(bIters) }];
const limit = Number(limitText);
if (!input || !output || !Number.isSafeInteger(limit) || limit <= 0 ||
    configs.some(c => !Number.isFinite(c.c) || c.c <= 0 || !Number.isSafeInteger(c.iterations) || c.iterations <= 0) ||
    (filter !== undefined && filter !== "preview")) {
  throw new Error("usage: compare-wasm-search.js GAMES OUTPUT A_C B_C A_ITERS B_ITERS LIMIT [preview]");
}
assert.equal(fs.existsSync(output), false, "output already exists");
const repo = path.resolve(__dirname, "..");
const pkg = path.join(repo, "crates/wasm/pkg-node");
const wasm = require(path.join(pkg, "nc2000_wasm.js"));
const hash = p => crypto.createHash("sha256").update(fs.readFileSync(p)).digest("hex");
const lines = fs.readFileSync(input, "utf8").trim().split("\n");
const manifest = JSON.parse(lines.shift());
const poolJson = fs.readFileSync(manifest.config.pool, "utf8");
const pool = JSON.parse(poolJson);
assert.equal("sha256:" + hash(manifest.config.pool), manifest.hashes.pool);
assert.equal("sha256:" + hash(path.join(repo, "data/gen2stadium2.json")), manifest.hashes.dex);
const dex = new wasm.Dex();
function parseGame(line) {
  const game = JSON.parse(line);
  const seeds = line.match(/"agent_seeds"\s*:\s*\[\s*(\d+)\s*,\s*(\d+)\s*\]/);
  assert.ok(seeds, "missing exact agent seeds");
  game.seeds32 = seeds.slice(1).map(seed => Number(BigInt(seed) & 0xffffffffn));
  return game;
}
function agentsFor(game) {
  return configs.map(config => [0, 1].map(side => {
    const agent = new wasm.ProtocolSearcher(dex, side, poolJson, game.seeds32[side ^ game.swap], config.c, 16);
    agent.setOwnTeam(JSON.stringify(pool.teams[game.team_ids[side]].sets));
    return agent;
  }));
}
function choose(agent, entry, iterations) {
  agent.pushLines(JSON.stringify(entry.frame.lines));
  assert.equal(agent.onRequest(JSON.stringify(entry.frame.request)), true);
  agent.step(iterations);
  const action = agent.best();
  assert.ok(entry.frame.legal_actions.includes(action), "illegal wasm action");
}
const warmupIterations = 3000;
const warmupGame = parseGame(lines[0]);
const warmup = agentsFor(warmupGame);
try {
  for (const entry of warmupGame.frames.slice(0, 4)) {
    for (const arm of warmup) choose(arm[entry.side], entry, warmupIterations);
  }
} finally {
  warmup.flat().forEach(agent => agent.free());
}
const rows = [];
try {
  for (const line of lines) {
    const game = parseGame(line), agents = agentsFor(game);
    try {
      for (const entry of game.frames) {
        if (filter === "preview" && !entry.frame.request.teamPreview) break;
        const ns = [0, 0], iterations = [0, 0];
        for (const arm of [rows.length % 2, 1 - rows.length % 2]) {
          const agent = agents[arm][entry.side], start = process.hrtime.bigint();
          choose(agent, entry, configs[arm].iterations);
          ns[arm] = Number(process.hrtime.bigint() - start);
          iterations[arm] = JSON.parse(agent.rootPolicy()).iterations;
        }
        rows.push({ game: game.game, side: entry.side, turn: entry.turn,
          seed: game.seeds32[entry.side ^ game.swap],
          kind: entry.frame.request.teamPreview ? "preview" : Math.max(...iterations) === 0 ? "forced" : "battle",
          ns, iterations });
        if (rows.length >= limit) break;
      }
    } finally {
      agents.flat().forEach(agent => agent.free());
    }
    if (rows.length >= limit) break;
  }
} finally {
  dex.free();
}
assert.equal(rows.length, limit, "insufficient recorded decisions");
const groups = {};
for (const row of rows) {
  const group = groups[row.kind] ??= { decisions: 0, a_ns: 0, b_ns: 0 };
  group.decisions++;
  group.a_ns += row.ns[0];
  group.b_ns += row.ns[1];
}
for (const group of Object.values(groups)) group.a_over_b = group.a_ns / group.b_ns;
const result = { schema: "nc2000-wasm-search-timing-v1", configs, warmupIterations,
  hashes: { input: hash(input), script: hash(__filename), wasm: hash(path.join(pkg, "nc2000_wasm_bg.wasm")),
    binding: hash(path.join(pkg, "nc2000_wasm.js")), dex: manifest.hashes.dex, pool: manifest.hashes.pool },
  groups, rows };
fs.writeFileSync(output, JSON.stringify(result, null, 2) + "\n", { flag: "wx" });
console.log(JSON.stringify(groups));
