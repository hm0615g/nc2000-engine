// The bot's ordinary own-team pool (data/team-pool-v1/team-pool.json): the
// teams classified strong for bot use, each with its draw probability. It
// answers one question — which team the bot brings when nobody pinned one —
// on the open door's "Random" and on every plain blind game.
//
// It is deliberately NOT the list of teams on the start screen (that stays
// the bundled pool, so what a human can pick does not change underneath
// them) and NOT the bot's picture of its opponent (the shipped prior,
// belief-pool-v2). Draws carry a null pool index: the baked pair tables are
// keyed to the bundled list, never to this file.

import type { SelectedTeam } from "./pool-pick";
import {
  drawWeighted,
  parseWeightedTeams,
  type WeightedTeam,
} from "./weighted-teams";

export interface OwnPool {
  teams: WeightedTeam[];
}

export type OwnPoolParse =
  | { ok: true; pool: OwnPool }
  | { ok: false; errors: string[] };

export function parseOwnPool(text: string): OwnPoolParse {
  const parsed = parseWeightedTeams(text, "drawWeight", "own-team pool");
  if (!parsed.ok) return parsed;
  return { ok: true, pool: { teams: parsed.teams } };
}

export function drawOwnTeam(pool: OwnPool): SelectedTeam {
  return drawWeighted(pool.teams);
}
