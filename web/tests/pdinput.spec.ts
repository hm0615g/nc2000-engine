import { expect, test } from "@playwright/test";

const base = process.env.NC2000_E2E_BASE ?? "/";

test("?pdinput renders all 33 teams, splits into 2 safe batches under 8KB, and generates installer script", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto(`${base}?pdinput`);
  await expect(page.getByTestId("pdinput-page")).toBeVisible();
  await expect(page.getByTestId("pdinput-batch-1")).toBeVisible();
  await expect(page.getByTestId("pdinput-batch-2")).toBeVisible();

  // Batch 1 is selected by default: 17 teams, under 8000 bytes
  const summary = page.getByTestId("pdinput-selection-summary");
  await expect(summary).toContainText("17 / 17");
  const b1Match = (await summary.textContent())?.match(/([\d,]+)\s*\/\s*8,000 B/);
  expect(b1Match).toBeTruthy();
  const b1Bytes = Number(b1Match![1].replace(/,/g, ""));
  expect(b1Bytes).toBeGreaterThan(7000);
  expect(b1Bytes).toBeLessThanOrEqual(8000);

  // Switching to Batch 2 selects the remaining 16 teams (18..33)
  await page.getByTestId("pdinput-batch-2").click();
  await expect(summary).toContainText("16 / 17");
  const b2Match = (await summary.textContent())?.match(/([\d,]+)\s*\/\s*8,000 B/);
  expect(b2Match).toBeTruthy();
  const b2Bytes = Number(b2Match![1].replace(/,/g, ""));
  expect(b2Bytes).toBeGreaterThan(6500);
  expect(b2Bytes).toBeLessThanOrEqual(8000);

  // Script textarea holds a valid javascript: one-liner with 16 entries
  const scriptVal = await page.getByTestId("pdinput-script-text").inputValue();
  expect(scriptVal.startsWith("javascript:(()=>{")).toBe(true);
  expect(scriptVal).toContain("1_x_");

  expect(errors).toEqual([]);
});
