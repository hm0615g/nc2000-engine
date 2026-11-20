import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";

const live = JSON.parse(
  readFileSync(new URL("../../data/live-search.json", import.meta.url), "utf8"),
) as { defaultStrength: string; battle: { adaptive: number } };

test("play doors use the adaptive battle budget without a strength selector", async ({ page }) => {
  expect(live.defaultStrength).toBe("adaptive");
  expect(live.battle).toEqual({ adaptive: 243000 });
  await page.addInitScript(() => {
    localStorage.setItem("nc2000-locale", "en");
    const post = Worker.prototype.postMessage;
    Worker.prototype.postMessage = function (message, ...args: unknown[]) {
      if (message?.t === "battle") {
        document.documentElement.dataset.adaptiveBudget = String(message.searcher.adaptiveBudget);
      }
      return Reflect.apply(post, this, [message, ...args]);
    };
  });
  for (const url of ["/", "/?nash", "/?blind", "/?budget=strong", "/?budget=standard"]) {
    await page.goto(url);
    await expect(page.locator(".start-title")).toBeVisible();
    await expect(page.getByRole("combobox", { name: "Bot strength", exact: true })).toHaveCount(0);
    await expect(page.locator(".strength-choice")).toHaveCount(0);
    await page.getByRole("button", { name: "Start battle" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-adaptive-budget", "true");
  }
});
