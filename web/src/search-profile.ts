import profilesJson from "../../data/search-profiles.json?raw";
import type { InfoMode } from "./info-mode";

const profiles: Record<InfoMode, { c: number; iterations: number }> =
  JSON.parse(profilesJson);
const testBudget =
  import.meta.env.MODE === "test"
    ? Number(import.meta.env.VITE_NC2000_TEST_BUDGET)
    : Number.NaN;

export function searchProfile(mode: InfoMode) {
  return {
    c: profiles[mode].c,
    iterations:
      Number.isSafeInteger(testBudget) && testBudget > 0
        ? testBudget
        : profiles[mode].iterations,
  };
}
