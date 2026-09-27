import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import type { EvaluationRun } from "../src/evaluate-core";

const route = `${process.env.NC2000_E2E_BASE ?? "/"}?evaluate`;
const player = [{ species: "Mewtwo", level: 100, moves: ["Psychic"] }];
const opponent = [{ species: "Magikarp", level: 1, moves: ["Splash"] }];

async function distribution(page: Page, sets = opponent) {
  await page
    .getByLabel("分布JSONファイル")
    .setInputFiles({
      name: "mix.json",
      mimeType: "application/json",
      buffer: Buffer.from(
        JSON.stringify({ teams: [{ id: "fish", weight: 1, sets }] }),
      ),
    });
  await expect(page.locator(".eval-opponent strong")).toHaveText("fish");
}
async function saved(page: Page): Promise<EvaluationRun[]> {
  return page.evaluate(
    () =>
      new Promise((resolve, reject) => {
        const req = indexedDB.open("nc2000-evaluations", 1);
        req.onsuccess = () => {
          const db = req.result;
          const read = db.transaction("runs").objectStore("runs").getAll();
          read.onsuccess = () => {
            db.close();
            resolve(read.result);
          };
          read.onerror = () => reject(read.error);
        };
        req.onerror = () => reject(req.error);
      }),
  );
}
async function boot(page: Page) {
  await page.goto(route);
  await expect(
    page.getByLabel("自分のパーティ", { exact: true }),
  ).toBeEnabled();
}

test("warnings allow illegal parties, invalid data blocks execution", async ({
  page,
}) => {
  await boot(page);
  await page
    .getByLabel("自分のパーティ", { exact: true })
    .fill(
      JSON.stringify(
        Array.from({ length: 6 }, () => ({
          species: "Snorlax",
          level: 55,
          item: "Leftovers",
          moves: ["Spikes"],
        })),
      ),
    );
  await expect(
    page.getByText("このパーティは選出の合計155制限を解除します。", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "計測を開始", exact: true }),
  ).toBeEnabled();
  await page
    .getByLabel("自分のパーティ", { exact: true })
    .fill('[{"species":"MissingNo","moves":["Tackle"]}]');
  await expect(
    page.getByRole("button", { name: "計測を開始", exact: true }),
  ).toBeDisabled();
  await page
    .getByLabel("自分のパーティ", { exact: true })
    .fill(JSON.stringify(player));
  await page.getByLabel("sample-07 の重み").fill("-1");
  await expect(
    page.getByRole("button", { name: "計測を開始", exact: true }),
  ).toBeDisabled();
  await expect(page.getByRole("alert")).toContainText("重み");
});

test("local wasm plays both sides, persists, resumes, separates changed configurations and exports", async ({
  page,
}) => {
  const failures: string[] = [];
  let workerUrl = "";
  page.on("worker", (w) => {
    workerUrl = w.url();
  });
  page.on("pageerror", (e) => failures.push(String(e)));
  await boot(page);
  await distribution(page);
  await page
    .getByLabel("自分のパーティ", { exact: true })
    .fill(JSON.stringify(player));
  await page.getByLabel("試合数", { exact: true }).fill("2");
  await expect(
    page.getByRole("button", { name: "計測を開始", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "計測を開始", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("計測が完了", {
    timeout: 120000,
  });
  await expect.poll(async () => (await saved(page))[0]?.pairs.length).toBe(1);
  const initial = (await saved(page))[0];
  expect(initial.pairs[0].games.map((g) => g.playerSide)).toEqual([0, 1]);
  expect(initial.pairs[0].games.map((g) => g.outcome)).toEqual(["win", "win"]);
  await page.reload();
  await expect(
    page.getByRole("button", { name: "追加計測", exact: true }),
  ).toBeEnabled();
  await page.getByLabel("試合数", { exact: true }).fill("2");
  await page.getByRole("button", { name: "追加計測", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("計測が完了", {
    timeout: 120000,
  });
  const updated = (await saved(page))[0];
  expect(updated.pairs).toHaveLength(2);
  expect(updated.pairs[0]).toEqual(initial.pairs[0]);
  expect(updated.pairs[1].index).toBe(1);
  const replayed = await page.evaluate(
    async ({ url, run, base }) => {
      const beliefJson = await (
        await fetch(`${base}data/belief-pool-v1/belief-pool.json`)
      ).text();
      return new Promise((resolve, reject) => {
        const w = new Worker(url, { type: "module" });
        const pairs: unknown[] = [];
        w.onerror = (event) => {
          w.terminate();
          reject(event.message);
        };
        w.onmessage = (event) => {
          if (event.data.type === "pair") {
            pairs.push(event.data.pair);
            w.postMessage({ type: "ack" });
          }
          if (event.data.type === "done") {
            w.terminate();
            resolve(pairs);
          }
          if (event.data.type === "error") {
            w.terminate();
            reject(event.data.message);
          }
        };
        w.postMessage({
          type: "start",
          run: { ...run, pairs: [] },
          beliefJson,
        });
      });
    },
    { url: workerUrl, run: updated, base: process.env.NC2000_E2E_BASE ?? "/" },
  );
  expect(replayed).toEqual(updated.pairs);

  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "結果JSONを出力" }).click();
  const file = await download;
  expect(JSON.parse(readFileSync((await file.path())!, "utf8"))).toEqual(
    updated,
  );
  await page.getByLabel("fish の重み").fill("0");
  await expect(
    page.getByRole("button", { name: "追加計測", exact: true }),
  ).toBeDisabled();
  await page.getByLabel("fish の重み").fill("1");
  await page.getByLabel("試合数", { exact: true }).fill("100");
  await page.getByRole("button", { name: "追加計測", exact: true }).click();
  await page.getByRole("button", { name: "停止", exact: true }).click();
  await page.reload();
  await expect(
    page.getByRole("button", { name: "再開", exact: true }),
  ).toBeEnabled();
  expect((await saved(page))[0].pairs.length).toBeGreaterThanOrEqual(2);
  await page
    .getByLabel("自分のパーティ", { exact: true })
    .fill(JSON.stringify([{ ...player[0], level: 99 }]));
  await expect(
    page.getByRole("button", { name: "再開", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "別の計測を開始", exact: true }),
  ).toBeEnabled();
  expect(failures).toEqual([]);
});

test("relaxed high-level preview survives blind search and reaches outcomes", async ({
  page,
}) => {
  await boot(page);
  await distribution(page);
  await page
    .getByLabel("自分のパーティ", { exact: true })
    .fill(JSON.stringify(Array.from({ length: 3 }, () => player[0])));
  await page.getByLabel("試合数", { exact: true }).fill("2");
  await expect(
    page.getByRole("button", { name: "計測を開始", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "計測を開始", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("計測が完了", {
    timeout: 120000,
  });
  const result = (await saved(page))[0];
  expect(result.config.player.relaxed).toBe(true);
  expect(result.pairs[0].games.map((g) => g.outcome)).toEqual(["win", "win"]);
});

test("ordinary six-Pokemon party completes against the default Nash mixture", async ({
  page,
}) => {
  await boot(page);
  const input = readFileSync(
    new URL("../../data/party-vs-nash-20260927/party.json", import.meta.url),
    "utf8",
  );
  await page.getByLabel("自分のパーティ", { exact: true }).fill(input);
  await page.getByLabel("試合数", { exact: true }).fill("2");
  await expect(
    page.getByRole("button", { name: "計測を開始", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "計測を開始", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("計測が完了", {
    timeout: 180000,
  });
  const result = (await saved(page))[0];
  expect(result.config.player.relaxed).toBe(false);
  expect(result.config.player.warnings).toEqual([]);
  expect(["sample-07", "sample-08", "sample-10"]).toContain(
    result.pairs[0].opponent,
  );
  expect(result.pairs[0].games).toHaveLength(2);
});

test("storage failure remains visible while calculation and export still work", async ({
  page,
}) => {
  await page.addInitScript(() =>
    Object.defineProperty(window, "indexedDB", {
      get() {
        throw new Error("storage unavailable");
      },
    }),
  );
  await boot(page);
  await distribution(page);
  await page
    .getByLabel("自分のパーティ", { exact: true })
    .fill(JSON.stringify(player));
  await page.getByLabel("試合数", { exact: true }).fill("2");
  await page.getByRole("button", { name: "計測を開始", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("計測が完了", {
    timeout: 120000,
  });
  await expect(page.getByRole("alert")).toContainText("自動保存できません");
  await expect(
    page.getByRole("button", { name: "結果JSONを出力" }),
  ).toBeEnabled();
});

test("worker failure stops without inventing a game outcome", async ({
  page,
}) => {
  await page.route("**/assets/evaluate-worker-*.js", (route) => route.abort());
  await boot(page);
  await distribution(page);
  await page
    .getByLabel("自分のパーティ", { exact: true })
    .fill(JSON.stringify(player));
  await page.getByLabel("試合数", { exact: true }).fill("2");
  await page.getByRole("button", { name: "計測を開始", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("エラーで停止");
  await expect(page.getByRole("alert")).toBeVisible();
  expect((await saved(page))[0].pairs).toEqual([]);
});
