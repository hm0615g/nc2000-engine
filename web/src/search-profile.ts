import live from "../../data/live-search.json";
import profilesJson from "../../data/search-profiles.json?raw";
import type { InfoMode } from "./info-mode";

const profiles: Record<InfoMode, { c: number; iterations: number }> =
  JSON.parse(profilesJson);
const testBudget =
  import.meta.env.MODE === "test"
    ? Number(import.meta.env.VITE_NC2000_TEST_BUDGET)
    : Number.NaN;

/** Production cap for adaptive battle search. Team Preview has its own budget. */
export const PRODUCT_ITERATIONS = live.battle.adaptive;
export const PREVIEW_ITERATIONS = live.unknownPreview;
export function liveBudget(preview = false): number {
  return Number.isSafeInteger(testBudget) && testBudget > 0
    ? testBudget : preview ? PREVIEW_ITERATIONS : PRODUCT_ITERATIONS;
}

/** Live play is "blind"; "open" is the retired profile, only for replaying
 * records made under it. */
export function searchProfile(mode: InfoMode) {
  return {
    c: profiles[mode].c,
    iterations:
      Number.isSafeInteger(testBudget) && testBudget > 0
        ? testBudget
        : profiles[mode].iterations,
  };
}
