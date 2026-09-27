import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import type { EvaluationRun } from "../src/evaluate-core";

test.use({ actionTimeout: 10000 });

const route = `${process.env.NC2000_E2E_BASE ?? "/"}?evaluate`;
const player = [{ species: "Mewtwo", level: 100, moves: ["Psychic"] }];
const opponent = [{ species: "Magikarp", level: 1, moves: ["Splash"] }];

async function distribution(page: Page, sets = opponent) {
  await page.getByLabel("相手の設定ファイル").setInputFiles({
    name: "mix.json",
    mimeType: "application/json",
    buffer: Buffer.from(
      JSON.stringify({ teams: [{ id: "fish", weight: 1, sets }] }),
    ),
  });
  await expect(page.locator(".eval-opponent strong")).toHaveText(["fish"]);
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
  await page.getByText("テキストから読み込む", { exact: true }).click();
}

test("warnings allow illegal parties, invalid data blocks execution", async ({
  page,
}) => {
  await boot(page);
  await page.getByLabel("自分のパーティ", { exact: true }).fill(
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
    page.getByText(
      "このパーティは、選ぶ3匹のレベル合計が155を超えていても対戦できます。",
      {
        exact: true,
      },
    ),
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
  await page
    .getByLabel("相手の設定ファイル")
    .setInputFiles({
      name: "invalid.json",
      mimeType: "application/json",
      buffer: Buffer.from(
        JSON.stringify({ teams: [{ id: "bad", weight: -1, sets: opponent }] }),
      ),
    });
  await expect(page.getByRole("alert")).toContainText("出やすさ");
  await expect(page.locator(".eval-opponent strong")).toHaveCount(3);
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
  await page.getByRole("button", { name: "結果をファイルに保存" }).click();
  const file = await download;
  expect(JSON.parse(readFileSync((await file.path())!, "utf8"))).toEqual(
    updated,
  );
  await distribution(page, [{ ...opponent[0], level: 2 }]);
  await expect(
    page.getByRole("button", { name: "追加計測", exact: true }),
  ).toBeDisabled();
  await distribution(page);
  await page.getByLabel("試合数", { exact: true }).fill("100");
  await page.getByRole("button", { name: "追加計測", exact: true }).click();
  await page.getByRole("button", { name: "停止", exact: true }).click();
  await page.reload();
  await expect(
    page.getByRole("button", { name: "再開", exact: true }),
  ).toBeEnabled();
  expect((await saved(page))[0].pairs.length).toBeGreaterThanOrEqual(2);
  await page.getByText("テキストから読み込む", { exact: true }).click();
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
    page.getByRole("button", { name: "結果をファイルに保存" }),
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

test("Japanese party form works without secure-context APIs, preserving moves across members", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(Crypto.prototype, "subtle", { get: () => undefined });
    Object.defineProperty(Crypto.prototype, "randomUUID", { value: undefined });
  });
  await page.goto(route);
  const mon = page.getByRole("combobox", {
    name: "自分のパーティ1匹目のポケモン",
    exact: true,
  });
  await expect(mon).toBeEnabled();
  await expect(page.getByLabel("自分のパーティ", { exact: true })).toBeHidden();
  await mon.fill("みゅう");
  await page.getByRole("option", { name: "ミュウツー", exact: true }).click();
  await page
    .getByLabel("自分のパーティ1匹目のレベル", { exact: true })
    .fill("100");
  await page
    .getByRole("combobox", { name: "自分のパーティ1匹目の技1", exact: true })
    .fill("サイコキネシス");
  await page
    .getByRole("heading", { name: "自分のパーティ", exact: true })
    .click();
  await page
    .getByRole("button", { name: "＋ ポケモンを追加", exact: true })
    .first()
    .click();
  await page
    .getByRole("combobox", {
      name: "自分のパーティ2匹目のポケモン",
      exact: true,
    })
    .fill("カビゴン");
  await page
    .getByRole("combobox", { name: "自分のパーティ2匹目の技1", exact: true })
    .fill("のしかかり");
  await page
    .getByRole("button", { name: "1. ミュウツー", exact: true })
    .click();
  await expect(
    page.getByRole("combobox", {
      name: "自分のパーティ1匹目の技1",
      exact: true,
    }),
  ).toHaveValue("サイコキネシス");
  await page.getByRole("button", { name: "2. カビゴン", exact: true }).click();
  await page
    .getByRole("button", { name: "このポケモンを外す", exact: true })
    .first()
    .click();
  await distribution(page);
  await page.getByLabel("試合数", { exact: true }).fill("2");
  await page.getByRole("button", { name: "計測を開始", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("計測が完了", {
    timeout: 120000,
  });
  const result = (await saved(page))[0];
  expect(result.config.player.sets).toHaveLength(1);
  expect(result.config.player.sets[0]).toMatchObject({
    species: "Mewtwo",
    level: 100,
    moves: ["Psychic"],
  });
  expect(result.pairs[0].games.map((g) => g.outcome)).toEqual(["win", "win"]);
  expect(result.config.beliefHash).toHaveLength(64);
  await page.reload();
  await expect(
    page.getByRole("button", { name: "追加計測", exact: true }),
  ).toBeEnabled();
});

test("editing an imported party preserves custom stats and exposes no technical input by default", async ({
  page,
}) => {
  await boot(page);
  const original = {
    species: "Raikou",
    level: 55,
    moves: ["Thunderbolt", "Hidden Power Ice"],
    ivs: { hp: 30, atk: 22, def: 26, spa: 30, spd: 30, spe: 30 },
    evs: { hp: 200, atk: 0, def: 128, spa: 255, spd: 255, spe: 255 },
    happiness: 17,
  };
  await page
    .getByLabel("自分のパーティ", { exact: true })
    .fill(JSON.stringify([original]));
  await page
    .getByRole("combobox", { name: "自分のパーティ1匹目の持ち物", exact: true })
    .fill("たべのこし");
  const raw = JSON.parse(
    await page.getByLabel("自分のパーティ", { exact: true }).inputValue(),
  );
  expect(raw[0]).toEqual({ ...original, item: "Leftovers" });
  await page.getByText("テキストから読み込む", { exact: true }).first().click();
  const visibleText = await page.locator("main").innerText();
  expect(visibleText).not.toMatch(/WASM|belief|反復|BLIND|正規化/);
  await expect(page.getByLabel("考える回数", { exact: true })).toHaveValue(
    "3000",
  );
});

test("opponent panel shows three parties and probabilities, with JSON as its only edit control", async ({
  page,
}) => {
  await page.goto(route);
  const panel = page.getByRole("region", { name: "対戦相手の設定" });
  await expect(panel.locator(".eval-opponent strong")).toHaveText([
    "基本の相手1",
    "基本の相手2",
    "基本の相手3",
  ]);
  await expect(panel.locator(".eval-opponent span")).toHaveText([
    "57.6%",
    "22.2%",
    "20.1%",
  ]);
  await expect(panel.locator(".eval-opponent-roster li")).toHaveCount(18);
  await expect(panel.locator("input")).toHaveCount(1);
  await expect(panel.locator("input")).toHaveAttribute("type", "file");
  await expect(panel.locator("textarea, select")).toHaveCount(0);
  await expect(
    page.getByText("自動で補った項目", { exact: false }),
  ).toHaveCount(0);
  await expect(
    page.getByLabel("考える回数", { exact: true }).locator("option"),
  ).toHaveText(["3,000回", "10,000回", "27,000回"]);
  await panel.getByText("技・持ち物を見る", { exact: true }).first().click();
  await expect(
    panel.locator(".eval-opponent-sets").first().locator("strong"),
  ).toHaveCount(6);
});
