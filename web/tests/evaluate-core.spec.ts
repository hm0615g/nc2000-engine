import { test, expect } from "@playwright/test";
import {
  appendPair,
  normalizeWeights,
  pairSchedule,
  sameConfig,
  summarize,
  resultsCsv,
  type EvaluationConfig,
  type EvaluationRun,
  type PairResult,
} from "../src/evaluate-core";
import { readDoor, infoModeOf } from "../src/info-mode";

const pair = (
  index: number,
  a: "win" | "loss" | "tie" | "cap",
  b = a,
): PairResult => ({
  index,
  opponent: "one",
  battleSeed: 123,
  agentSeeds: [1, 2],
  games: [
    { playerSide: 0, outcome: a, turns: 1 },
    { playerSide: 1, outcome: b, turns: 2 },
  ],
});
const team = {
  sets: [{ species: "Snorlax", level: 55 }],
  warnings: ["team size"],
  fixes: ["default EVs"],
  relaxed: false,
};
const config: EvaluationConfig = {
  build: "abc",
  beliefHash: "def",
  player: team,
  opponents: [{ ...team, id: "one", weight: 1 }],
  iterations: 3000,
  c: 0.4,
  turnLimit: 500,
};
const run: EvaluationRun = {
  version: 1,
  id: "test",
  createdAt: "2026-09-27",
  config,
  seed: 19,
  targetPairs: 4,
  pairs: [],
};

test("evaluator URL is isolated and blind", () => {
  expect(readDoor("?evaluate&nash&solver")).toBe("evaluate");
  expect(infoModeOf(readDoor("?evaluate"))).toBe("blind");
  expect(readDoor("?evaluate=false&nash")).toBe("nash");
  expect(readDoor("")).toBe("open");
});
test("weights normalize without overflow and reject invalid distributions", () => {
  expect(
    normalizeWeights([
      { weight: Number.MAX_VALUE },
      { weight: Number.MAX_VALUE },
    ]),
  ).toEqual([{ weight: 0.5 }, { weight: 0.5 }]);
  for (const input of [
    [],
    [{ weight: 0 }],
    [{ weight: -1 }],
    [{ weight: NaN }],
    [{ weight: Infinity }],
  ])
    expect(() => normalizeWeights(input)).toThrow();
});
test("opponent draws and seeds reproduce across chunking, respecting zero weights", () => {
  const mix = [
    { id: "zero", weight: 0 },
    { id: "one", weight: 1 },
    { id: "three", weight: 3 },
  ];
  const full = Array.from({ length: 10000 }, (_, i) =>
    pairSchedule(9182, i, mix),
  );
  expect(full.some((p) => p.opponent === "zero")).toBe(false);
  const rate = full.filter((p) => p.opponent === "one").length / full.length;
  expect(rate).toBeGreaterThan(0.23);
  expect(rate).toBeLessThan(0.27);
  expect([pairSchedule(9182, 5, mix), pairSchedule(9182, 6, mix)]).toEqual(
    full.slice(5, 7),
  );
});
test("paired interval uses pairs, keeps caps distinct and scores ties as half", () => {
  expect(summarize([]).mean).toBeNull();
  expect(summarize([pair(0, "win")]).interval).toBeNull();
  const s = summarize([
    pair(0, "win"),
    pair(1, "loss"),
    pair(2, "tie"),
    pair(3, "cap"),
  ]);
  expect(s).toMatchObject({
    games: 8,
    win: 2,
    loss: 2,
    tie: 2,
    cap: 2,
    mean: 0.5,
    winRate: 0.25,
  });
  expect(s.interval![0]).toBeCloseTo(0.5 - 1.96 * Math.sqrt(0.5 / 3 / 4));
  expect(
    summarize([pair(0, "win", "loss"), pair(1, "loss", "win")]).interval,
  ).toEqual([0.5, 0.5]);
});
test("resume identity ignores canonicalization notices but rejects changed conditions", () => {
  expect(
    sameConfig(config, { ...config, player: { ...team, fixes: [] } }),
  ).toBe(true);
  for (const patch of [
    { iterations: 10000 },
    { build: "other" },
    { beliefHash: "other" },
    { player: { ...team, relaxed: true } },
  ])
    expect(sameConfig(config, { ...config, ...patch })).toBe(false);
});
test("only ordered complete pairs are appended, exports preserve perspective", () => {
  const next = appendPair(run, pair(0, "win", "loss"));
  expect(next.pairs).toHaveLength(1);
  expect(() => appendPair(next, pair(0, "win"))).toThrow();
  expect(() => appendPair(next, pair(2, "win"))).toThrow();
  expect(resultsCsv(next)).toContain(
    '"0","one","2","loss","0","2","123","1","2"',
  );
});
