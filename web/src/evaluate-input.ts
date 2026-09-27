import { Battle, getDex, getValidator } from "./engine";
import { parsePsExport } from "./ps-import";
import { findingAnchor, findingText, type Finding } from "./findings";
import {
  normalizeWeights,
  type EvaluationTeam,
  type EvaluationOpponent,
} from "./evaluate-core";

export interface OpponentDraft {
  id: string;
  weight: string;
  text: string;
}
const explain = (f: Finding) =>
  [findingAnchor(f), findingText(f)].filter(Boolean).join(": ");

export function readTeam(text: string): EvaluationTeam {
  if (!text.trim()) throw new Error("パーティを入力してください。");
  let raw: unknown;
  if (/^\s*[\[{]/.test(text)) raw = JSON.parse(text);
  else {
    const parsed = parsePsExport(text);
    if (parsed.findings.length)
      throw new Error(parsed.findings.map(explain).join("\n"));
    raw = parsed.sets;
  }
  if (raw && typeof raw === "object" && "sets" in raw) raw = raw.sets;
  if (!Array.isArray(raw) || raw.length < 1 || raw.length > 6)
    throw new Error("パーティJSONは1〜6匹の配列にしてください。");
  for (const [i, set] of raw.entries()) {
    if (!set || typeof set !== "object" || typeof set.species !== "string")
      throw new Error(`${i + 1}匹目: 種族を指定してください。`);
    if (
      set.level !== undefined &&
      (!Number.isInteger(set.level) || set.level < 1 || set.level > 100)
    )
      throw new Error(`${i + 1}匹目: レベルは1〜100の整数にしてください。`);
    if (
      set.moves !== undefined &&
      (!Array.isArray(set.moves) ||
        set.moves.some((m: unknown) => typeof m !== "string") ||
        set.moves.length > 4)
    )
      throw new Error(`${i + 1}匹目: 技は4個以下の文字列配列にしてください。`);
  }
  const result = JSON.parse(
    getValidator().canonicalizeTeam(JSON.stringify(raw)),
  ) as { team: unknown[]; applied: Finding[]; errors: Finding[] };
  const fatal = result.errors.filter((f) =>
    [
      "json-invalid",
      "species-unknown",
      "move-unknown",
      "item-unknown",
    ].includes(f.code),
  );
  if (fatal.length) throw new Error(fatal.map(explain).join("\n"));
  if (!Array.isArray(result.team))
    throw new Error("パーティを構成できません。");
  const relaxed = result.errors.some(
    (f) => f.code === "level-sum" || f.code === "level-sum-highest",
  );
  const battle = new Battle(
    getDex(),
    JSON.stringify(result.team),
    JSON.stringify(result.team),
    "1,2,3,4",
  );
  battle.free();
  return {
    sets: result.team,
    warnings: result.errors.map(explain),
    fixes: result.applied.map(explain),
    relaxed,
  };
}

export function readOpponents(drafts: OpponentDraft[]): EvaluationOpponent[] {
  const seen = new Set<string>();
  const entries = drafts.map((d) => {
    const id = d.id.trim();
    if (!id || seen.has(id))
      throw new Error("相手の名前は空欄にせず、重複しないようにしてください。");
    seen.add(id);
    if (!d.weight.trim()) throw new Error(`${id}: 重みを入力してください。`);
    return { id, weight: Number(d.weight), ...readTeam(d.text) };
  });
  return normalizeWeights(entries);
}

export function importDistribution(text: string): OpponentDraft[] {
  const raw = JSON.parse(text);
  if (!raw || !Array.isArray(raw.teams))
    throw new Error("分布JSONには teams 配列が必要です。");
  const drafts = raw.teams.map(
    (e: { id?: unknown; weight?: unknown; sets?: unknown }) => {
      if (
        !e ||
        typeof e.id !== "string" ||
        typeof e.weight !== "number" ||
        !Array.isArray(e.sets)
      )
        throw new Error("各相手には id・weight・sets を指定してください。");
      return {
        id: e.id,
        weight: String(e.weight),
        text: JSON.stringify(e.sets, null, 2),
      };
    },
  );
  readOpponents(drafts);
  return drafts;
}

export async function sha256(text: string): Promise<string> {
  const hash = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text),
  );
  return Array.from(new Uint8Array(hash), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
}
