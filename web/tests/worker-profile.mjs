import assert from "node:assert/strict";
import { createServer as createHttpServer } from "node:http";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
import { createServer } from "vite";

const root = fileURLToPath(new URL("..", import.meta.url));
const server = createHttpServer();
let browser, vite;
try {
  await new Promise((resolve) => server.listen(0, "0.0.0.0", resolve));
  vite = await createServer({
    root,
    server: { middlewareMode: true, hmr: { server, clientPort: server.address().port } },
  });
  server.on("request", vite.middlewares);
  browser = await chromium.launch({
    ...(process.env.NC2000_CHROMIUM_EXE
      ? { executablePath: process.env.NC2000_CHROMIUM_EXE }
      : {}),
  });
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  const results = await page.evaluate(async (wasmUrl) => {
    const { BotWorker } = await import("/src/bot.ts");
    const { searchProfile } = await import("/src/search-profile.ts");
    const wasm = await import(wasmUrl);
    await wasm.default();
    const dex = new wasm.Dex();
    const poolJson = await (await fetch("/data/belief-pool-v3/belief-pool.json")).text();
    const catalog = await (await fetch("/data/team-pool-v2/team-pool.json")).json();
    const teams = catalog.teams.slice(0, 2).map((team) => JSON.stringify(team.sets));
    const seed = 7;
    const expected = (iterations, c) => {
      const battle = new wasm.Battle(dex, teams[0], teams[1], "1,2,3,4");
      battle.setLogEnabled(true);
      const search = new wasm.BlindSearcher(battle, 1, poolJson, seed, c);
      search.observe(battle);
      search.step(iterations);
      const policy = JSON.parse(search.rootPolicy());
      search.free();
      battle.free();
      return policy;
    };
    const results = [];
    // Live battles are blind-only: the worker must search with the blind
    // profile's c, and a c = 1.0 search must not reproduce its policy.
    {
      for (const scenario of ["fixed", "early-flush", "ponder-cap"]) {
        const worker = new BotWorker();
        try {
          await worker.newBattle(teams[0], teams[1], "1,2,3,4", { poolJson, side: 1, seed });
          const budget = 512;
          const promise = worker.search(1, budget, seed, scenario !== "fixed");
          if (scenario === "early-flush") worker.flush();
          const result = await Promise.race([
            promise,
            new Promise((_, reject) => setTimeout(() => reject(new Error("worker search timed out")), 60000)),
          ]);
          const iterations = scenario === "ponder-cap" ? budget * 10 : budget;
          const profile = searchProfile("blind");
          results.push({ scenario, c: profile.c, iterations: result.policy.iterations,
            expectedIterations: iterations,
            policyMatches: JSON.stringify(result.policy) === JSON.stringify(expected(iterations, profile.c)),
            wrongCoefficientMatches: JSON.stringify(result.policy) === JSON.stringify(expected(iterations, 1)),
          });
        } finally {
          worker.terminate();
        }
      }
    }
    dex.free();
    return results;
  }, "/@fs" + fileURLToPath(new URL("../../crates/wasm/pkg-web/nc2000_wasm.js", import.meta.url)));
  for (const result of results) {
    assert.equal(result.iterations, result.expectedIterations, JSON.stringify(result));
    assert.equal(result.policyMatches, true, JSON.stringify(result));
    assert.equal(result.wrongCoefficientMatches, false, JSON.stringify(result));
  }
  console.log(JSON.stringify(results, null, 2));
} finally {
  await browser?.close();
  await vite?.close();
  await new Promise((resolve) => server.close(resolve));
}
