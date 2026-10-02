// One side's selected team, and the uniform draw over a pool file the user
// loaded (app.tsx owns every draw and which rule applies).

import { randomSeed32 } from "./engine";
import type { MetaPool } from "./types";

/** One side's selected team. Sets are captured at start, so deleting a
 * saved custom during the game cannot alter the current battle or its
 * rematches. */
export interface SelectedTeam {
  id: string;
  sets: unknown[];
}

/** Draw a uniformly random team of a loaded pool file: a fresh 32-bit
 * CSPRNG roll reduced modulo the pool size (the modulo bias over 32 bits is
 * far below anything a hand-made pool could express). */
export function randomPoolTeam(pool: MetaPool): SelectedTeam {
  const teams = pool.teams;
  const idx = randomSeed32() % teams.length;
  return { id: teams[idx].id, sets: teams[idx].sets };
}
