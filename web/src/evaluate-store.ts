import type { EvaluationRun } from "./evaluate-core";

const storageKey = "nc2000-evaluations";

export async function saveRun(run: EvaluationRun): Promise<void> {
  const runs = await loadRuns();
  sessionStorage.setItem(
    storageKey,
    JSON.stringify([run, ...runs.filter((r) => r.id !== run.id)]),
  );
}

export async function loadRuns(): Promise<EvaluationRun[]> {
  const runs: EvaluationRun[] = JSON.parse(
    sessionStorage.getItem(storageKey) ?? "[]",
  );
  return runs
    .filter((r) => r.version === 1)
    .sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}
