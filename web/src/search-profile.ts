import profilesJson from "../../data/search-profiles.json?raw";
import type { InfoMode } from "./info-mode";

const profiles: Record<InfoMode, { c: number; iterations: number }> =
  JSON.parse(profilesJson);
const testBudget =
  import.meta.env.MODE === "test"
    ? Number(import.meta.env.VITE_NC2000_TEST_BUDGET)
    : Number.NaN;

/** The live bot's per-decision budget, never the test build's override:
 * what a measurement has to match to speak for the product. */
export const PRODUCT_ITERATIONS = profiles.blind.iterations;

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
