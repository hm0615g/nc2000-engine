// The Nash door's mixture, as the browser plays it: a fixed probability
// distribution over the solved support, and the draw that samples it.
//
// `data/meta-nash-v3/pool-artifact.json` is the shipped end of the rebuild's
// Nash step (docs/TEAM-POOL-REBUILD-PLAN.md). Its claim is about the MIXTURE,
// not about any one team, so the draw is the product — a nash game samples
// it once per battle, independently, and a rematch draws again. Playing the
// top-weighted team every time would be a different strategy with a
// different (worse) guarantee.
//
// The mixture governs which team the BOT brings. What the bot assumes about
// the team it is FACING is the shipped opponent prior (belief-pool-v3),
// which app.tsx hands every blind door alike; the two files are independent
// by design. The artifact carries its own sets and those are what gets
// played.
//
// The loader is strict: a team that does not validate refuses the whole
// artifact, and a refusal fails the page rather than degrading. `?nash`
// without its mixture is not a weaker nash mode, it is blind mode wearing
// its name.

import type { SelectedTeam } from "./pool-pick";
import {
  drawWeighted,
  parseWeightedTeams,
  type WeightedTeam,
} from "./weighted-teams";

/** One team of the mixture: species / levels for display (the start
 * screen shows the composition, never the sets), weight renormalized. */
export type NashTeam = WeightedTeam;

export interface NashMix {
  teams: NashTeam[];
  /** The solution file the weights came from, for the credit line. */
  source: string;
}

export type NashParse =
  | { ok: true; mix: NashMix }
  | { ok: false; errors: string[] };

export function parseNashArtifact(text: string): NashParse {
  const parsed = parseWeightedTeams(text, "weight", "nash artifact");
  if (!parsed.ok) return parsed;
  const source =
    typeof parsed.raw.source_solution === "string" ? parsed.raw.source_solution : "";
  return { ok: true, mix: { teams: parsed.teams, source } };
}

/** Sample the mixture: one battle, one draw. */
export function drawNashTeam(mix: NashMix): SelectedTeam {
  return drawWeighted(mix.teams);
}
