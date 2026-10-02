// Custom parties (M14) on the blind play door: the human imports, pins and
// deletes saved parties; a game owns snapshots of both teams, so a deletion
// elsewhere cannot alter a running battle or its rematch. The opponent is
// never chosen here — it is drawn from the catalog.

import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";

interface SetJson {
  species: string;
  item?: string;
  moves: string[];
  level?: number;
}

interface PoolTeamJson {
  id: string;
  export: string;
  sets: SetJson[];
}

interface CustomRecord {
  id: string;
  name: string;
  sets: SetJson[];
  species: string[];
  levels: number[];
  savedAt: number;
}

const pool = JSON.parse(
  readFileSync(
    new URL("../../data/meta-pool-v0/meta-pool.json", import.meta.url),
    "utf8",
  ),
) as { teams: PoolTeamJson[] };

// Custom-party import stays available on the human side: legal teams of the
// retired bundled pool are the import material (their `export` field is PS
// text). Attack-heavy, self-destruct-heavy sets keep the full games short.
const teamA = pool.teams[4];
const teamB = pool.teams[29];

function custom(team: PoolTeamJson, id: string, name: string): CustomRecord {
  return {
    id,
    name,
    sets: team.sets,
    species: team.sets.map((s) => s.species),
    levels: team.sets.map((s) => s.level ?? 55),
    savedAt: 1,
  };
}

const customA = custom(teamA, "custom-a", "Custom A");

function toId(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9]/g, "");
}

async function seedStorage(
  page: Page,
  customs: CustomRecord[],
  picks?: unknown,
) {
  await page.addInitScript(
    ({ records, initialPicks }) => {
      if (sessionStorage.getItem("nc2000-e2e-seeded") === "1") return;
      sessionStorage.setItem("nc2000-e2e-seeded", "1");
      localStorage.setItem("nc2000-locale", "en");
      localStorage.setItem("nc2000-custom-teams", JSON.stringify(records));
      if (initialPicks === undefined)
        localStorage.removeItem("nc2000-start-picks");
      else
        localStorage.setItem(
          "nc2000-start-picks",
          JSON.stringify(initialPicks),
        );
    },
    { records: customs, initialPicks: picks },
  );
}

function guardConsole(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("request", (r) => {
    if (r.url().includes("/preview-tables-v0/"))
      errors.push(`pair-table request: ${r.url()}`);
  });
  return errors;
}

async function expectExactSets(
  section: ReturnType<Page["locator"]>,
  team: { name: string; sets: SetJson[] },
) {
  await expect(section.getByRole("heading", { name: new RegExp(team.name) })).toBeVisible();
  for (const set of team.sets) {
    const head = section.locator(`[data-mon="${toId(set.species)}"]`);
    await expect(head).toHaveCount(1);
    if ((await head.getAttribute("aria-expanded")) !== "true")
      await head.click();
    const sheet = head.locator("xpath=..");
    if (set.item)
      await expect(sheet.locator(`[data-item="${toId(set.item)}"]`)).toHaveCount(1);
    const actualMoves = await sheet.locator("[data-move]").evaluateAll((els) =>
      els.map((e) => e.getAttribute("data-move") ?? "").sort(),
    );
    expect(actualMoves).toEqual(set.moves.map(toId).sort());
  }
}

async function choosePreview(page: Page) {
  for (let n = 0; n < 3; n++) {
    const candidates = page.locator(
      '.pick-head[aria-pressed="false"][aria-disabled="false"]',
    );
    await expect(candidates.first()).toBeVisible();
    await candidates.first().click();
  }
  const confirm = page.getByRole("button", { name: "Confirm picks" });
  await expect(confirm).toBeEnabled();
  await confirm.click();
  await expect(page.locator(".battle-screen")).toBeVisible({ timeout: 90_000 });
}

async function playToOutcome(page: Page) {
  let decisions = 0;
  const deadline = Date.now() + 10 * 60 * 1000;
  while (decisions < 300 && Date.now() < deadline) {
    if (await page.locator(".end-banner").isVisible()) return;
    const moves = page.locator(".move-btn");
    const moveCount = await moves.count();
    if (moveCount > 0) {
      const scores = await moves.evaluateAll((buttons) =>
        buttons.map((b) => {
          const text = b.querySelector(".move-bp")?.textContent ?? "0";
          return Number(text.match(/\d+/)?.[0] ?? 0);
        }),
      );
      let best = 0;
      for (let i = 1; i < scores.length; i++)
        if (scores[i] > scores[best]) best = i;
      await moves.nth(best).click();
      decisions++;
      await page.waitForTimeout(100);
      continue;
    }
    const switches = page.locator(".switch-btn");
    if ((await switches.count()) > 0) {
      await switches.first().click();
      decisions++;
      await page.waitForTimeout(100);
      continue;
    }
    await page.waitForTimeout(100);
  }
  throw new Error(
    `battle did not reach an outcome (${decisions} human decisions)`,
  );
}

test.describe.configure({ mode: "serial" });

test("an imported party persists, plays its exact sets, and survives deletion mid-game", async ({
  page,
}) => {
  const errors = guardConsole(page);
  await seedStorage(page, [customA]);
  await page.goto("/");

  // Import from the human picker; a successful import pins the new party
  // while keeping the result visible.
  await page.locator('[data-party="human"]').click();
  await page.getByRole("button", { name: "+ Import a custom team" }).click();
  await page.locator(".import-text").fill(teamB.export);
  await page.locator(".import-name").fill("Custom B");
  await page.locator(".import-btn").click();
  await expect(page.locator(".import-ok-note")).toContainText("Custom B");
  const stored = await page.evaluate(() => ({
    customs: JSON.parse(localStorage.getItem("nc2000-custom-teams") ?? "[]"),
    picks: JSON.parse(localStorage.getItem("nc2000-start-picks") ?? "{}"),
  }));
  const savedB = (stored.customs as CustomRecord[]).find(
    (t) => t.name === "Custom B",
  );
  expect(savedB).toBeTruthy();
  expect(stored.picks).toEqual({ human: { kind: "custom", id: savedB!.id } });
  await expect(page.locator(`[data-custom="${savedB!.id}"]`)).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await page.locator("dialog.modal .modal-head button").click();

  // The pin survives a reload by stable custom id.
  await page.reload();
  await expect(page.locator('[data-party="human"] .party-value')).toHaveText(
    "Custom B",
  );

  await page.getByRole("button", { name: "Start battle" }).click();
  await expect(page.locator(".preview-screen")).toBeVisible();

  // Simulate deletion from another tab after start. The game owns its
  // snapshot, so this battle and the rematch keep Custom B exactly.
  await page.evaluate((id) => {
    const records = JSON.parse(
      localStorage.getItem("nc2000-custom-teams") ?? "[]",
    ) as CustomRecord[];
    localStorage.setItem(
      "nc2000-custom-teams",
      JSON.stringify(records.filter((t) => t.id !== id)),
    );
  }, savedB!.id);

  await choosePreview(page);
  await page.locator(".sheets-btn").click();
  const mine = page.locator(".team-sheets > section").first();
  await expectExactSets(mine, { name: "Custom B", sets: savedB!.sets });
  await page.locator("dialog.modal .modal-head button").click();
  await playToOutcome(page);

  await page.getByRole("button", { name: "Rematch" }).click();
  await expect(page.locator(".preview-screen")).toBeVisible();
  const previewMine = page.locator(".preview-cols > section").nth(1);
  const species = await previewMine
    .locator("[data-mon]")
    .evaluateAll((els) => els.map((e) => e.getAttribute("data-mon")));
  expect(species.sort()).toEqual(savedB!.sets.map((s) => toId(s.species)).sort());
  await page.locator(".preview-actions .quit-btn").click();

  // Back on the start screen the deleted party is gone, so the pin falls
  // back to Random; the in-UI delete path does the same for Custom A.
  await expect(page.locator('[data-party="human"] .party-value')).toHaveText(
    "Random",
  );
  await page.locator('[data-party="human"]').click();
  await page.locator('[data-custom="custom-a"]').click();
  await expect(page.locator('[data-party="human"] .party-value')).toHaveText(
    "Custom A",
  );
  await page.locator('[data-party="human"]').click();
  const deleteA = page
    .locator('.custom-card:has([data-custom="custom-a"])')
    .locator(".delete-btn");
  await deleteA.click();
  await deleteA.click();
  await page.locator("dialog.modal .modal-head button").click();
  await expect(page.locator('[data-party="human"] .party-value')).toHaveText(
    "Random",
  );
  expect(errors).toEqual([]);
});
