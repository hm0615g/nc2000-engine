// psense.lib.net/_/PDINPUT.cgi encoder & automated browser importer.
//
// Specification of `PDINPUT.cgi` (EUC-JP, Apache 1.3.39):
// 1. Cookie storage:
//    User parties are stored in HTTP cookies `PD0`, `PD1`, `PD2`, ... under
//    path `/_/`. `CBAN` (1-indexed party number in the UI) maps to `PD{CBAN-1}`.
//    Active parties begin with `1_x_` followed by 6 `_x_`-delimited Pokémon
//    records; deleted slots hold `2`.
// 2. Apache 1.3 header limit:
//    `LimitRequestFieldSize` is 8190 bytes. Because each 6-Pokémon party is
//    ~455 bytes (`PDn=1_x_...`), at most 17 parties (~7.7 KB) can reside in
//    `Cookie:` simultaneously before Apache returns `400 Bad Request`.
//    The 33-team `team-pool-v2` catalog therefore splits cleanly into 2 parts
//    (Part 1: teams 1–17, Part 2: teams 18–33).
// 3. 26-field Pokémon record (`_` delimiter):
//    [0]      National Dex number (1..251)
//    [1]      Level (50..55)
//    [2..5]   Atk, Def, Spe, Spc DVs in uppercase hex (`0`..`F`)
//    [6..10]  HP, Atk, Def, Spe, Spc stat-experience effort levels (`0`..`63`, `floor(ev/4)`)
//    [11..14] Move 1..4 IDs (`dex.moves[id].num + 1`, or `0` if empty; all Hidden Power variants use 238)
//    [15..18] Move 1..4 max PP (`base_pp + floor(base_pp / 5) * ppup`, including 64 for base-40 PP moves)
//    [19..22] Move 1..4 PP Up counts (`0`..`3`)
//    [23]     Max HP (`floor(((base_hp + hp_dv) * 2 + do_hp) * level / 100) + level + 10`)
//    [24]     Status (`0`)
//    [25]     Gen 2 item internal hex ID in decimal (`0` if no item)
// 4. Cross-origin import mechanisms on `PDINPUT.cgi`:
//    - Direct slot write (1 POST per party):
//      POST `ACTION=NEW&ACTION2=SAVED&PBAN=<1..17>&PD=<mon1_x_.._x_mon5>&PC=5&B1=%B4%B0%CE%BB`
//      with the 6th Pokémon's EUC-JP form fields sets `Set-Cookie: PD{PBAN-1}=1_x_<mon1..6>`
//      immediately without requiring `SameSite=Lax` intermediate cookies.
//    - Saved-HTML import (`ACTION=RECEPT`):
//      POST `ACTION=RECEPT&C=1_x_<mon1..6>` in a top-level window/tab chains
//      `RECEPT -> RECEPT_BRIDGE -> RECEPT_SAVE` via auto-submitted forms and
//      appends the party to the first empty slot (`PD0..PD11`).

import { toId } from "./i18n";
import type { PoolTeam } from "./types";

export const PDINPUT_URL = "http://psense.lib.net/_/PDINPUT.cgi";

/** Apache 1.3.39 LimitRequestFieldSize is 8190 bytes; 17 6-mon parties (~7.7 KB)
 * is the maximum safe batch size before Cookie header overflow (HTTP 400). */
export const PDINPUT_MAX_BATCH_SIZE = 17;
export const PDINPUT_COOKIE_LIMIT_BYTES = 8000;

/** Gen 2 internal item IDs (decimal) used by psense.lib.net/_/PDINPUT.cgi. */
export const PDINPUT_ITEM_CODES: Record<string, number> = {
  "": 0,
  masterball: 2,
  ultraball: 3,
  brightpowder: 4,
  greatball: 5,
  pokeball: 6,
  moonstone: 9,
  firestone: 23,
  thunderstone: 24,
  waterstone: 25,
  luckypunch: 31,
  leafstone: 35,
  metalpowder: 36,
  quickclaw: 74,
  psncureberry: 75,
  softsand: 77,
  sharpbeak: 78,
  przcureberry: 79,
  burntberry: 80,
  iceberry: 81,
  poisonbarb: 82,
  kingsrock: 83,
  bitterberry: 84,
  mintberry: 85,
  silverpowder: 89,
  mysticwater: 96,
  twistedspoon: 97,
  blackbelt: 99,
  blackglasses: 103,
  pinkbow: 105,
  stick: 106,
  nevermeltice: 108,
  magnet: 109,
  miracleberry: 110,
  spelltag: 114,
  miracleseed: 118,
  thickclub: 119,
  focusband: 120,
  hardstone: 126,
  charcoal: 139,
  berryjuice: 140,
  scopelens: 141,
  metalcoat: 144,
  dragonfang: 145,
  leftovers: 147,
  mysteryberry: 151,
  dragonscale: 152,
  berserkgene: 153,
  heavyball: 158,
  levelball: 160,
  lureball: 161,
  fastball: 162,
  lightball: 164,
  friendball: 165,
  moonball: 166,
  loveball: 167,
  sunstone: 170,
  polkadotbow: 171,
  upgrade: 173,
  berry: 174,
  goldberry: 175,
};

export interface PdinputDexSpecies {
  num: number;
  name: string;
  baseStats: { hp: number; atk: number; def: number; spa: number; spd: number; spe: number };
}

export interface PdinputDexMove {
  num: number;
  name: string;
  pp: number;
}

export interface PdinputDex {
  species: Record<string, PdinputDexSpecies>;
  moves: Record<string, PdinputDexMove>;
}

export interface PdinputI18nJa {
  species: Record<string, string>;
  moves: Record<string, string>;
  items: Record<string, string>;
}

export interface SetInput {
  species?: string;
  name?: string;
  level?: number;
  item?: string;
  moves?: string[];
  ivs?: { hp?: number; atk?: number; def?: number; spa?: number; spd?: number; spe?: number };
  evs?: { hp?: number; atk?: number; def?: number; spa?: number; spd?: number; spe?: number };
}

export interface EncodedMon {
  /** 26-field `_`-delimited PDINPUT record for this Pokémon. */
  record: string;
  speciesId: string;
  jpSpecies: string;
  level: number;
  jpMoves: [string, string, string, string];
  ppUps: [number, number, number, number];
  jpItem: string;
  doValues: [number, number, number, number, number];
  koHex: string;
  maxHp: number;
}

export interface EncodedTeam {
  id: string;
  tier: string;
  /** Full `PDn` cookie value (`1_x_<mon1>_x_..._x_<mon6>`). */
  cookieValue: string;
  /** `_x_`-joined records of the first 5 Pokémon (used as hidden `PD` in single-POST slot writes). */
  firstFivePd: string;
  /** The 6th Pokémon's form fields (used with `firstFivePd` in single-POST slot writes). */
  sixthMon: EncodedMon;
  mons: EncodedMon[];
}

export function encodeMonForPdinput(
  rawSet: unknown,
  dex: PdinputDex,
  ja: PdinputI18nJa,
): EncodedMon {
  const s = (rawSet ?? {}) as SetInput;
  const spId = toId(s.species ?? s.name ?? "");
  const sp = dex.species[spId];
  if (!sp) throw new Error(`Unknown species: ${s.species ?? s.name ?? "?"}`);
  const jpSpecies = ja.species[spId];
  if (!jpSpecies) throw new Error(`Missing JP species name for ${spId}`);

  const level = typeof s.level === "number" ? s.level : 50;
  const ivs = s.ivs ?? {};
  const atkDv = Math.max(0, Math.min(15, Math.floor((ivs.atk ?? 31) / 2)));
  const defDv = Math.max(0, Math.min(15, Math.floor((ivs.def ?? 31) / 2)));
  const speDv = Math.max(0, Math.min(15, Math.floor((ivs.spe ?? 31) / 2)));
  const spcDv = Math.max(0, Math.min(15, Math.floor((ivs.spa ?? 31) / 2)));
  const hpDv =
    ((atkDv & 1) << 3) |
    ((defDv & 1) << 2) |
    ((speDv & 1) << 1) |
    (spcDv & 1);

  const evs = s.evs ?? {};
  const doHp = Math.max(0, Math.min(63, Math.floor((evs.hp ?? 255) / 4)));
  const doAtk = Math.max(0, Math.min(63, Math.floor((evs.atk ?? 255) / 4)));
  const doDef = Math.max(0, Math.min(63, Math.floor((evs.def ?? 255) / 4)));
  const doSpe = Math.max(0, Math.min(63, Math.floor((evs.spe ?? 255) / 4)));
  const doSpc = Math.max(0, Math.min(63, Math.floor((evs.spa ?? 255) / 4)));

  const rawMoves = Array.isArray(s.moves) ? s.moves : [];
  const moveCodes: number[] = [];
  const movePps: number[] = [];
  const ppUps: [number, number, number, number] = [0, 0, 0, 0];
  const jpMoves: [string, string, string, string] = ["", "", "", ""];

  for (let i = 0; i < 4; i++) {
    const rawMove = rawMoves[i];
    if (rawMove) {
      let mId = toId(rawMove);
      if (mId.startsWith("hiddenpower")) mId = "hiddenpower";
      const mv = dex.moves[mId];
      if (!mv) throw new Error(`Unknown move: ${rawMove}`);
      const jpMv = ja.moves[mId];
      if (!jpMv) throw new Error(`Missing JP move name for ${mId}`);
      const ppUp = 3;
      // PDINPUT.cgi uses `dex_num + 1` and `base_pp + floor(base_pp / 5) * ppUp`
      // (including 64 PP for base-40 PP moves).
      moveCodes.push(mv.num + 1);
      movePps.push(mv.pp + Math.floor(mv.pp / 5) * ppUp);
      ppUps[i] = ppUp;
      jpMoves[i] = jpMv;
    } else {
      moveCodes.push(0);
      movePps.push(0);
      ppUps[i] = 0;
      jpMoves[i] = "";
    }
  }

  const baseHp = sp.baseStats.hp;
  const maxHp =
    Math.floor(((baseHp + hpDv) * 2 + doHp) * level / 100) + level + 10;

  const itemId = toId(s.item ?? "");
  const itemCode = PDINPUT_ITEM_CODES[itemId];
  if (itemCode === undefined) {
    throw new Error(`Unsupported item for PDINPUT.cgi: ${s.item ?? ""}`);
  }
  const jpItem = itemId ? (ja.items[itemId] ?? "") : "";

  const hex = (n: number) => n.toString(16).toUpperCase();
  const koHex = `${hex(atkDv)}${hex(defDv)}${hex(speDv)}${hex(spcDv)}`;

  const fields = [
    sp.num,
    level,
    hex(atkDv),
    hex(defDv),
    hex(speDv),
    hex(spcDv),
    doHp,
    doAtk,
    doDef,
    doSpe,
    doSpc,
    ...moveCodes,
    ...movePps,
    ...ppUps,
    maxHp,
    0,
    itemCode,
  ];

  return {
    record: fields.join("_"),
    speciesId: spId,
    jpSpecies,
    level,
    jpMoves,
    ppUps,
    jpItem,
    doValues: [doHp, doAtk, doDef, doSpe, doSpc],
    koHex,
    maxHp,
  };
}

export function encodeTeamForPdinput(
  team: Pick<PoolTeam, "id" | "sets"> & { tier?: string },
  dex: PdinputDex,
  ja: PdinputI18nJa,
): EncodedTeam {
  if (!Array.isArray(team.sets) || team.sets.length !== 6) {
    throw new Error(`Team ${team.id} must have 6 Pokémon (got ${team.sets?.length ?? 0})`);
  }
  const mons = team.sets.map((s) => encodeMonForPdinput(s, dex, ja));
  const cookieValue = "1_x_" + mons.map((m) => m.record).join("_x_");
  const firstFivePd = mons.slice(0, 5).map((m) => m.record).join("_x_");
  return {
    id: team.id,
    tier: team.tier ?? "",
    cookieValue,
    firstFivePd,
    sixthMon: mons[5],
    mons,
  };
}

/** Form fields for a single-POST write into slot `pban` (1-indexed, sets `PD{pban-1}`).
 * Submitting this form with `accept-charset="EUC-JP"` to `PDINPUT.cgi` writes all
 * 6 Pokémon into `PD{pban-1}` in a single HTTP request. */
export function buildSinglePostSlotFields(
  team: EncodedTeam,
  pban: number,
): Record<string, string> {
  const m6 = team.sixthMon;
  return {
    sim: "",
    ACTION: "NEW",
    ACTION2: "SAVED",
    PD: team.firstFivePd,
    PC: "5",
    PBAN: String(pban),
    LV: String(m6.level),
    SYU: m6.jpSpecies,
    WAZA1: m6.jpMoves[0],
    PPUP1: String(m6.ppUps[0]),
    WAZA2: m6.jpMoves[1],
    PPUP2: String(m6.ppUps[1]),
    WAZA3: m6.jpMoves[2],
    PPUP3: String(m6.ppUps[2]),
    WAZA4: m6.jpMoves[3],
    PPUP4: String(m6.ppUps[3]),
    ITEM: m6.jpItem,
    DO1: String(m6.doValues[0]),
    DO2: String(m6.doValues[1]),
    DO3: String(m6.doValues[2]),
    DO4: String(m6.doValues[3]),
    DO5: String(m6.doValues[4]),
    KO: m6.koHex,
    B1: "完了",
  };
}

/** Compute the exact HTTP `Cookie:` header byte length for a batch of encoded teams. */
export function cookieHeaderBytes(teams: EncodedTeam[]): number {
  if (teams.length === 0) return 0;
  return teams
    .map((t, i) => `PD${i}=${t.cookieValue}`)
    .join("; ").length;
}

/** Split a list of encoded teams into safe batches that fit within Apache's
 * 8190-byte Cookie header limit (at most 17 teams per batch). */
export function splitIntoPdinputBatches(
  teams: EncodedTeam[],
  maxPerBatch = PDINPUT_MAX_BATCH_SIZE,
  maxBytes = PDINPUT_COOKIE_LIMIT_BYTES,
): EncodedTeam[][] {
  const batches: EncodedTeam[][] = [];
  let current: EncodedTeam[] = [];
  for (const t of teams) {
    const next = [...current, t];
    if (
      current.length > 0 &&
      (next.length > maxPerBatch || cookieHeaderBytes(next) > maxBytes)
    ) {
      batches.push(current);
      current = [t];
    } else {
      current = next;
    }
  }
  if (current.length > 0) batches.push(current);
  return batches;
}

/** Generate a bookmarklet / console one-liner that writes the given batch into
 * `document.cookie` on `http://psense.lib.net/_/` and clears any stale higher slots. */
export function buildCookieInstallerScript(
  teams: EncodedTeam[],
  clearUpToSlot = 35,
): string {
  const entries = teams.map((t) => t.cookieValue);
  return `javascript:(()=>{if(location.hostname!=='psense.lib.net'){alert('psense.lib.net/_/PDINPUT.cgi で実行してください');return;}const p=${JSON.stringify(entries)};const exp='expires=Tue, 19 Jan 2038 03:14:07 GMT; path=/_/';for(let i=0;i<${clearUpToSlot};i++){document.cookie='PD'+i+'=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/_/';}p.forEach((v,i)=>{document.cookie='PD'+i+'='+v+'; '+exp;});location.href='http://psense.lib.net/_/PDINPUT.cgi';})()`;
}
