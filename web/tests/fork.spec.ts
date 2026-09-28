import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";

const forkJson = readFileSync(
  new URL("../../data/forks/4296-t11.json", import.meta.url),
  "utf8",
);

function guardConsole(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(String(e)));
  return errors;
}

async function clearForkStorage(page: Page) {
  await page.addInitScript(() => {
    if (sessionStorage.getItem("nc2000-e2e-seeded") === "1") return;
    sessionStorage.setItem("nc2000-e2e-seeded", "1");
    for (const key of Object.keys(localStorage))
      if (key.startsWith("nc2000-fork:")) localStorage.removeItem(key);
  });
}

async function playToOutcome(page: Page) {
  const deadline = Date.now() + 10 * 60 * 1000;
  for (let decisions = 0; decisions < 300 && Date.now() < deadline; ) {
    if (await page.locator(".end-banner").isVisible()) return;
    const buttons = page.locator(".move-btn, .switch-btn");
    if ((await buttons.count()) > 0) {
      await buttons.first().click();
      decisions++;
    }
    await page.waitForTimeout(100);
  }
  throw new Error("forked battle did not reach an outcome");
}

test.describe.configure({ mode: "serial" });

test("a hosted fork plays, forfeits, reveals, exports and persists", async ({ page }) => {
  const errors = guardConsole(page);
  page.on("dialog", (d) => void d.accept());
  await clearForkStorage(page);
  await page.goto("/?fork=4296-t11");

  await expect(page.getByRole("heading", { name: "4296 T11" })).toBeVisible();
  await expect(page.locator(".fork-arms li")).toHaveCount(2);
  await page.getByRole("button", { name: "対局を始める" }).click();
  await expect(page.locator(".fork-battle")).toBeVisible();
  await expect(page.locator(".turn-label")).toContainText("ターン 11");
  await expect(page.locator(".fork-battle")).not.toContainText("この対局の bot の初手");
  await playToOutcome(page);
  await expect(page.locator(".end-banner")).toContainText("この対局の bot の初手");
  await page.getByRole("button", { name: "結果一覧へ" }).click();
  await expect(page.getByText("1 局終了")).toBeVisible();

  await page.getByRole("button", { name: "次の対局" }).click();
  await expect(page.locator(".fork-battle")).toBeVisible();
  await page.getByRole("button", { name: "投了" }).click();
  await expect(page.getByText("2 局終了")).toBeVisible();

  await page.getByRole("button", { name: "候補ごとの結果を表示" }).click();
  const rows = page.locator(".fork-table tbody tr");
  await expect(rows).toHaveCount(2);
  for (let i = 0; i < 2; i++) await expect(rows.nth(i).locator("td").nth(1)).toHaveText("1");

  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "結果を書き出す (JSONL)" }).click();
  const text = readFileSync(await (await download).path(), "utf8");
  const exported = text.trim().split("\n").map((l) => JSON.parse(l));
  expect(exported).toHaveLength(2);
  expect(exported.map((r) => r.action).sort()).toEqual(["move earthquake", "switch 2"]);
  expect(exported[1]).toMatchObject({ forfeit: true, outcome: "win", score: 1, reply: "human", trial: 1 });
  expect(exported[0].seed).toBe(exported[1].seed);

  await page.reload();
  await expect(page.getByText("2 局終了")).toBeVisible();
  expect(errors).toEqual([]);
});

test("a pasted fork is validated before any game starts", async ({ page }) => {
  await clearForkStorage(page);
  await page.goto("/?fork");
  const bad = JSON.parse(forkJson);
  bad.arms.push({ input: "move thunderbolt", label: "" });
  await page.locator(".fork-panel textarea").fill(JSON.stringify(bad));
  await page.getByRole("button", { name: "貼り付けた定義を読み込む" }).click();
  await expect(page.locator(".fork-error")).toContainText("not legal");

  await page.locator(".fork-panel textarea").fill(forkJson);
  await page.getByRole("button", { name: "貼り付けた定義を読み込む" }).click();
  await expect(page.getByRole("button", { name: "対局を始める" })).toBeVisible();
});
