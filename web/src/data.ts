// Runtime data fetching. The meta pool and the baked preview tables are
// served read-only from the repo data/ dir (see vite.config.ts) — pair
// files are still being baked in the background, so a missing or
// half-written file is an expected condition, answered with null (the
// caller falls back to live Searcher preview).

import type { MetaPool } from "./types";

/** All data fetches go under the deploy base (`/` locally,
 * `/nc2000-engine/` on GH Pages — see vite.config.ts). BASE_URL always ends
 * with a slash. */
const dataUrl = (rel: string) => `${import.meta.env.BASE_URL}data/${rel}`;

export interface PoolData {
  pool: MetaPool;
  poolJson: string;
}

/** JP name tables (M13). Throws on failure — the caller (i18n loadJaNames)
 * treats any failure as "no tables" and falls back to English names. */
export async function fetchI18nJa(): Promise<unknown> {
  const res = await fetch(dataUrl("i18n-ja.json"));
  if (!res.ok) throw new Error(`i18n-ja fetch failed: ${res.status}`);
  return res.json();
}

/** Dex JSON (data/gen2stadium2.json — the same data the wasm engine
 * embeds). Consulted client-side for set-sheet move meta (type/category/
 * BP) and species types. Throws on failure — the caller treats any
 * failure as "no meta available". */
export async function fetchDexJson(): Promise<unknown> {
  const res = await fetch(dataUrl("gen2stadium2.json"));
  if (!res.ok) throw new Error(`dex fetch failed: ${res.status}`);
  return res.json();
}

/** A hosted `nc2000-fork-v1` document, `data/forks/<name>.json`. */
export async function fetchFork(name: string): Promise<string> {
  if (!/^[A-Za-z0-9_-]+$/.test(name)) throw new Error(`invalid fork name: ${name}`);
  const res = await fetch(dataUrl(`forks/${name}.json`));
  if (!res.ok) throw new Error(`fork ${name}: ${res.status}`);
  return res.text();
}

export async function fetchPool(): Promise<PoolData> {
  const res = await fetch(dataUrl("meta-pool-v0/meta-pool.json"));
  if (!res.ok) throw new Error(`meta pool fetch failed: ${res.status}`);
  const poolJson = await res.text();
  return { pool: JSON.parse(poolJson) as MetaPool, poolJson };
}

/** The Nash door's shipped mixture (`?nash` only, so it is fetched only on
 * that door). Returned as text because nash-mix.ts is what decides what the
 * file means — and because a nash page with no mixture is not a mode, this
 * one throws: the caller lets it reach the boot error box rather than
 * quietly starting a plainer game under the mode's name. */
export async function fetchNashArtifact(): Promise<string> {
  const res = await fetch(dataUrl("meta-nash-v2/pool-artifact.json"));
  if (!res.ok) throw new Error(`nash artifact fetch failed: ${res.status}`);
  return res.text();
}

/** The bot's ordinary own-team pool (own-pool.ts decides what it means).
 * Every playing door draws from it, so a page that cannot load it fails
 * rather than quietly drawing from some other list. */
export async function fetchOwnPool(): Promise<string> {
  const res = await fetch(dataUrl("team-pool-v1/team-pool.json"));
  if (!res.ok) throw new Error(`own-team pool fetch failed: ${res.status}`);
  return res.text();
}

/** The shipped opponent prior (data/belief-pool-v2): the candidate set, with
 * weights, that every blind searcher narrows its opponent down to — plain
 * blind, nash and the solver alike, whatever pool file the user loaded.
 * Belief-only: the own-team draw, the start-screen lists and the baked
 * tables never read it. Strict: a blind page that cannot load it fails
 * rather than quietly playing under a plainer prior. */
export async function fetchBeliefPool(): Promise<PoolData> {
  const res = await fetch(dataUrl("belief-pool-v2/belief-pool.json"));
  if (!res.ok) throw new Error(`belief pool fetch failed: ${res.status}`);
  const poolJson = await res.text();
  const pool = JSON.parse(poolJson) as MetaPool;
  // Sanity, not validation: a truncated or wrong file must not silently
  // narrow the shipped belief below the bundled pool's size.
  if (!Array.isArray(pool.teams) || pool.teams.length < 32) {
    throw new Error(`belief pool: expected >=32 teams, got ${pool.teams?.length}`);
  }
  return { pool, poolJson };
}

/** Pair table for pool indices (i, j); canonical file is lo-hi. Returns the
 * raw JSON text, or null when the pair is not baked yet (404) or the file
 * is mid-write (parse failure). */
export async function fetchPairJson(
  i: number,
  j: number,
): Promise<string | null> {
  const lo = Math.min(i, j);
  const hi = Math.max(i, j);
  const pad = (n: number) => String(n).padStart(2, "0");
  const url = dataUrl(`preview-tables-v0/pair-${pad(lo)}-${pad(hi)}.json`);
  try {
    const res = await fetch(url);
    if (!res.ok) return null;
    const text = await res.text();
    JSON.parse(text); // reject half-written files
    return text;
  } catch {
    return null;
  }
}
