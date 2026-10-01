// Interval and test helpers for the `?fork` result tables. The tests use the
// same rules as tools/summarize-counterfactual.py.

export const pct = (v: number) => `${(v * 100).toFixed(1)}%`;

export function wilson(k: number, n: number): [number, number] {
  if (n === 0) return [0, 1];
  const z = 1.96;
  const p = k / n;
  const d = 1 + (z * z) / n;
  const c = (p + (z * z) / (2 * n)) / d;
  const h = (z * Math.sqrt((p * (1 - p)) / n + (z * z) / (4 * n * n))) / d;
  return [Math.max(0, c - h), Math.min(1, c + h)];
}

function comb(n: number, k: number): number {
  let r = 1;
  for (let i = 1; i <= k; i++) r = (r * (n - k + i)) / i;
  return r;
}

/** Two-sided Fisher exact test on independent wins/losses. */
export function fisherP(wa: number, la: number, wb: number, lb: number): number {
  const na = wa + la;
  const nb = wb + lb;
  const w = wa + wb;
  const total = comb(na + nb, w);
  const prob = (k: number) => (comb(na, k) * comb(nb, w - k)) / total;
  const observed = prob(wa);
  let p = 0;
  for (let k = Math.max(0, w - nb); k <= Math.min(na, w); k++)
    if (prob(k) <= observed * (1 + 1e-9)) p += prob(k);
  return Math.min(1, p);
}

/** Two-sided exact McNemar test from the discordant pair counts. */
export function mcnemarP(plus: number, minus: number): number {
  const d = plus + minus;
  if (d === 0) return 1;
  let log = -d * Math.LN2;
  let tail = Math.exp(log);
  for (let k = 1; k <= Math.min(plus, minus); k++) {
    log += Math.log((d - k + 1) / k);
    tail += Math.exp(log);
  }
  return Math.min(1, 2 * tail);
}

export function download(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: "application/x-ndjson" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
