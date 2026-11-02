#!/usr/bin/env python3
"""Verify collected ICO experiment hashes and write a reviewable result report."""
import argparse
import hashlib
import html
import json
import re
import shutil
import statistics
from pathlib import Path


def read(path):
    return json.loads(path.read_text())


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--work", type=Path, required=True)
    p.add_argument("--collected", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--site", type=Path, required=True)
    a = p.parse_args()
    repo = Path(__file__).resolve().parents[1]
    solver_hash = hashlib.sha256((repo/"tpu/src/gen2_tpu/ico.py").read_bytes()).hexdigest()
    # Audit the one export-format correction independently of the solver.
    for path in sorted((a.work/"census").glob("*.game.json")):
        original, normalized = read(path), read(a.work/"normalized"/path.name)
        old = original["players"][1]["infosets"][0]["actions"]
        new = normalized["players"][1]["infosets"][0]["actions"]
        strip = lambda values: [re.sub(r"^type-index=\d+;", "", v) for v in values]
        assert strip(old) == strip(new)
        original["players"][1]["infosets"][0]["actions"] = new
        assert original == normalized, "normalization must change labels only"
    stages = {}
    for stage, inputs, count in [("final", "normalized", 30), ("refine", "refine8", 2), ("depth1", "depth1", 2)]:
        folder = a.collected/f"{stage}-results"
        assert read(folder/"complete.json") == {"complete": True, "games": count}
        rows = []
        for src in sorted(folder.glob("*.result.json")):
            r = read(src)
            game = a.work/inputs/src.name.replace(".result.json", ".game.json")
            assert hashlib.sha256(game.read_bytes()).hexdigest() == r["game_sha256"]
            assert r["runtime"]["solver_sha256"] == solver_hash
            assert r["runtime"]["backend"] == "tpu"
            assert r["metadata"]["baseline_iterations"] == 81000
            assert r["unrestricted"]["iterations"] == 4096
            assert not r["full_game_certified"]
            write(a.out/f"{stage}-results"/src.name, r)
            rows.append(r)
        assert len(rows) == count
        stages[stage] = rows
    assert read(a.collected/"transfer-results/complete.json")["complete"]
    transfers = []
    for src in sorted((a.collected/"transfer-results").glob("*.result.json")):
        r = read(src)
        assert r["training_game_sha256"] in {x["game_sha256"] for x in stages["final"]}
        assert r["holdout_game_sha256"] in {x["game_sha256"] for x in stages["refine"]}
        assert r["training_seed"] != r["holdout_seed"]
        write(a.out/"transfer-results"/src.name, r)
        transfers.append(dict(opponent=src.name.removesuffix(".result.json"), **r))
    assert len(transfers) == 2
    rows = stages["final"]
    for name in ("census", "refine8", "depth1"):
        for file in ("manifest.json", "export-summary.json"):
            write(a.out/"provenance"/f"{name}-{file}", read(a.work/name/file))
    write(a.out/"provenance/label-normalization.json", read(a.work/"normalized/label-normalization.json"))
    for name in ("wasm-parity.json", "wasm-hidden-witness.json"):
        write(a.out/name, read(a.work/name))
    cpu = read(a.work/"cpu-check/apex-c08-surgical-142.result.json")
    tpu = next(r for r in stages["final"] if r["metadata"]["opponent"] == "apex-c08-surgical-142")
    assert cpu["game_sha256"] == tpu["game_sha256"]
    assert max(cpu["estimated_root_loss_interval"][0], tpu["estimated_root_loss_interval"][0]) <= min(cpu["estimated_root_loss_interval"][1], tpu["estimated_root_loss_interval"][1])
    write(a.out/"cpu-check/apex-c08-surgical-142.result.json", cpu)
    witness = read(a.work/"wasm-hidden-witness.json")
    assert len(witness["hidden_state_choice_collisions"]) == 2
    source_paths = ["tpu/src/gen2_tpu/ico.py", "crates/bot/examples/ico_export.rs",
                    "tools/ico-colab-worker.py", "tools/ico-holdout.py", "tools/ico-audit.py",
                    "tools/ico-wasm-parity.cjs", "tools/ico-report.py", "tpu/tests/test_ico.py"]
    write(a.out/"provenance/final-source-hashes.json", {
        f: hashlib.sha256((repo/f).read_bytes()).hexdigest() for f in source_paths})

    def item(r):
        return dict(opponent=r["metadata"]["opponent"],
                    root_loss=r["estimated_root_loss_interval"],
                    saddle_gap=r["unrestricted"]["saddle_gap"],
                    baseline_gap=r["baseline_root"]["saddle_gap"],
                    frontier=r["sampled_game_frontier_interval"])

    security = {mode: [min(r[mode][b] for r in rows) for b in ("lower", "upper")]
                for mode in ("unrestricted", "baseline_root")}
    security["root_loss"] = [security["unrestricted"][0]-security["baseline_root"][1],
                             security["unrestricted"][1]-security["baseline_root"][0]]
    security["lower_bound_rosters"] = {mode: min(rows, key=lambda r:r[mode]["lower"])["metadata"]["opponent"]
                                      for mode in ("unrestricted", "baseline_root")}
    summary = dict(
        format="nc2000-ico-audit-summary-v1", date="2026-11-03",
        scope="Finite first-battle-decision surrogate; all subsequent decisions optimized; not match win-rate improvement",
        full_game_certified=False, backend=rows[0]["runtime"],
        baseline_iterations=81000, iterations=4096,
        public_rosters=len(rows),
        opponent_type_cases=sum(len(r["metadata"]["opponent_types"]) for r in rows),
        hidden_worlds=sum(len(r["metadata"]["worlds"]) for r in rows),
        expanded_nodes=sum(r["metadata"]["nodes"] for r in rows),
        mean_root_loss=[statistics.mean(r["estimated_root_loss_interval"][i] for r in rows) for i in range(2)],
        median_root_loss=[statistics.median(r["estimated_root_loss_interval"][i] for r in rows) for i in range(2)],
        max_saddle_gap=max(r["unrestricted"]["saddle_gap"] for r in rows),
        adversarial_roster_security=security,
        mean_saddle_gap_by_iteration={str(n): statistics.mean(next(c["saddle_gap"] for c in r["unrestricted"]["checkpoints"] if c["iterations"] == n) for r in rows) for n in (64,256,1024,4096)},
        native_hidden_choice_counterexamples=witness["hidden_state_choice_collisions"],
        census_native_disagreements=sum(r["metadata"]["baseline_hidden_world_disagreements"] for r in rows),
        census=sorted([item(r) for r in rows], key=lambda r:r["root_loss"][0], reverse=True),
        independent_chance=[item(r) for r in stages["refine"]],
        depth_one=[item(r) for r in stages["depth1"]],
        frozen_root_transfer=[dict(opponent=r["opponent"], interval=r["heldout_root_gain_interval"]) for r in transfers],
        limitations=["Horizon two decision transitions; heuristic unresolved frontier remains [0,1]",
                     "Four chance samples per transition in census; eight independent samples in refinement",
                     "Top two compiled selections only; own probabilities renormalized",
                     "Finite catalog/prior types; no claim about all legal opponent parties",
                     "Initial engine fixture seed fixed; initial hidden chance not marginalized",
                     "Single production bot seed in census; separate seed 17011 yields two information-invariance counterexamples",
                     "Float64 best-response numerical brackets, not statistical confidence intervals or formal interval certificates"])
    write(a.out/"summary.json", summary)

    def interval(v):
        return f"{v[0]*100:.3f}–{v[1]*100:.3f}"
    table = "".join(f'<tr><td>{html.escape(r["opponent"])}</td><td>{interval(r["root_loss"])}</td><td>{r["saddle_gap"]*100:.4f}</td></tr>' for r in summary["census"])
    sensitivity = ""
    for r in summary["independent_chance"]:
        name = r["opponent"]
        first = next(x for x in summary["census"] if x["opponent"] == name)
        transfer = next(x for x in summary["frozen_root_transfer"] if x["opponent"] == name)
        shallow = next(x for x in summary["depth_one"] if x["opponent"] == name)
        sensitivity += f'<tr><td>{html.escape(name)}</td><td>{interval(first["root_loss"])}</td><td>{interval(r["root_loss"])}</td><td>{interval(transfer["interval"])}</td><td>{interval(shallow["root_loss"])}</td></tr>'
    limits = "".join(f"<li>{html.escape(s)}</li>" for s in summary["limitations"])
    page = f'''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>ICO vs production 81k — NC2000</title><style>
body{{font:16px/1.6 system-ui,sans-serif;background:#f7f8fa;color:#182437;margin:0}}main{{max-width:1100px;margin:auto;padding:36px 20px 70px}}h1{{font-size:2rem;line-height:1.2}}h2{{margin-top:2rem}}.notice{{background:#fff1cf;padding:18px;border-left:4px solid #ba7100}}.metrics{{display:flex;gap:20px;flex-wrap:wrap}}.metric{{background:white;padding:16px 24px;border:1px solid #dce2e9}}.metric strong{{display:block;font-size:1.6rem}}table{{border-collapse:collapse;width:100%;background:white}}th,td{{padding:8px 12px;text-align:left;border-bottom:1px solid #dce2e9}}th{{background:#eaf0f5}}td:not(:first-child){{font-variant-numeric:tabular-nums}}.scroll{{overflow:auto}}a{{color:#165bad}}code{{font-size:.9em}}small{{color:#536176}}
</style><main><small>NC2000 research · 2026-11-03 · Colab TPU v5e-1</small>
<h1>Information-Constrained Oracle vs production 81k</h1>
<p>ICO shares one policy across identical legal information histories. Hidden opponent sets, unrevealed choices and random seeds cannot select different actions within that information set.</p>
<p class="notice"><b>These are finite-model evaluation-score gaps, not measured win-rate gains.</b> The full-game frontier remains unresolved at approximately [0,1]. More CFR iterations cannot remove that uncertainty.</p>
<div class="metrics"><div class="metric"><strong>{summary["public_rosters"]}</strong>public rosters</div><div class="metric"><strong>{summary["hidden_worlds"]:,}</strong>hidden worlds, each at 81k</div><div class="metric"><strong>{interval(summary["median_root_loss"])}</strong>median score-point gap</div></div>
<h2>What was compared</h2><p>Our shipped APEX4 draw weights; adversarial opponent loadout, selection and battle strategy. Full legal battle actions, horizon two decision transitions, four empirical chance samples per transition, top two compiled selections. Both comparisons optimize continuation. CFR+ ran for 4,096 iterations; independent float64 best responses bracket each finite-game value. The census expanded {summary["expanded_nodes"]:,} nodes.</p><p>Allowing the opponent to choose the worst of these 30 rosters gives ICO {interval(security["unrestricted"])} vs native root {interval(security["baseline_root"])} score points: a gap of {interval(security["root_loss"])}. This robust finite-model comparison is separate from the descriptive median.</p>
<h2>Information-boundary finding in the production bot</h2><p>Two pairs of hidden worlds produced different production WASM choices despite identical player-visible histories and the same constructor seed (17011). Native and WASM actions, visits and means agree. This is a reproducible counterexample to information invariance in the tested production path; the cause is not yet localized. The census seed (20261103) had no such choice disagreement. ICO uses structural information-set sharing and passed the hidden-world and perfect-recall tests.</p>
<p>Worlds 50/54: <code>switch 2</code> vs <code>move icepunch</code>. Worlds 70/74: <code>move rockslide</code> vs <code>switch 2</code>. <a href="wasm-hidden-witness.json">Replay inputs and observation hashes</a>.</p>
<h2>Independent chance and horizon sensitivity</h2><p>Values below are score points on a 0–100 scale. The frozen-root column transfers the census root policy to the independent eight-sample game, optimizing only its continuation. These two selected cases do not supply population confidence intervals.</p>
<div class="scroll"><table><thead><tr><th>Opponent</th><th>Depth 2 / 4 samples</th><th>Depth 2 / 8 independent</th><th>Frozen root / independent</th><th>Depth 1 / 64 samples</th></tr></thead><tbody>{sensitivity}</tbody></table></div>
<h2>All 30 cases</h2><p>Loss is ICO value minus actual native first-choice value, each with optimized continuation. The residual column is the unrestricted finite-game best-response gap. Intervals are numerical optimization brackets, not confidence intervals.</p>
<div class="scroll"><table><thead><tr><th>Opponent</th><th>Root score-point gap</th><th>Residual score-point gap</th></tr></thead><tbody>{table}</tbody></table></div>
<h2>Limits and the path to the ideal ICO</h2><ul>{limits}</ul><p>Expand selection and private-type support, integrate initial hidden chance, extend decision histories, and obtain validated continuation bounds. Terminal exact rewards or valid bounds are necessary for a full-game convergence claim.</p>
<p><a href="summary.json">Machine-readable summary</a> · <a href="wasm-parity.json">Native/WASM comparator checks</a></p></main></html>'''
    a.site.mkdir(parents=True, exist_ok=True)
    (a.site/"index.html").write_text(page)
    for name in ("summary.json", "wasm-hidden-witness.json", "wasm-parity.json"):
        shutil.copyfile(a.out/name, a.site/name)
    (a.out/"README.md").write_text(f'''# ICO v1: production 81k audit

This is a **finite first-decision surrogate comparison**, not a measured match
win-rate gain and not a certified gap to the full-game ICO. See
[`docs/ICO.md`](../../docs/ICO.md) for the information contract and solver.

## Findings

- Colab TPU v5e-1, JAX {summary["backend"]["jax"]}, 4,096 CFR+ iterations per solve.
- {summary["public_rosters"]} public opponent rosters, {summary["opponent_type_cases"]} roster/loadout cases, {summary["hidden_worlds"]} hidden worlds and
  {summary["expanded_nodes"]:,} expanded nodes. Each world received an actual 81,000-iteration native search.
- Median root loss: **{interval(summary["median_root_loss"])} score points**;
  equal-roster mean: **{interval(summary["mean_root_loss"])} score points** on a 0–100 scale.
- Adversarial choice across all 30 rosters: ICO {interval(security["unrestricted"])} vs
  native root {interval(security["baseline_root"])}; gap **{interval(security["root_loss"])}**.
  Lower-bound rosters: ICO `{security["lower_bound_rosters"]["unrestricted"]}`;
  native root `{security["lower_bound_rosters"]["baseline_root"]}`.
- Maximum unrestricted finite-game residual: {summary["max_saddle_gap"]*100:.4f} score points.
  Numerical solver error is distinct from sampling, support and frontier error.
- Full-game frontier bounds remain approximately **[0,1]**. Actual win-rate
  improvement remains unmeasured. Do not sum local losses into a match gain.
- Two production WASM information-invariance counterexample pairs at constructor
  seed 17011: worlds 50/54 choose `switch 2` / `move icepunch`; worlds 70/74 choose
  `move rockslide` / `switch 2`. Within each pair, own sets and complete
  player-visible histories are identical. Actions, visits and means agree with
  the native comparator. The production cause has not been localized or fixed.
  The primary census seed 20261103 had no native choice disagreement.

## Independent-chance and depth checks

| Opponent | Census h2/s4 | Independent h2/s8 | Frozen census root on independent h2/s8 | h1/s64 |
|---|---:|---:|---:|---:|
''' + "".join(
        f'| {r["opponent"]} | {interval(next(x for x in summary["census"] if x["opponent"] == r["opponent"])["root_loss"])} | {interval(r["root_loss"])} | {interval(next(x for x in summary["frozen_root_transfer"] if x["opponent"] == r["opponent"])["interval"])} | {interval(next(x for x in summary["depth_one"] if x["opponent"] == r["opponent"])["root_loss"])} |\n'
        for r in summary["independent_chance"]) + '''
All numbers are score points, not win-rate percentage points. The two follow-up
cases were selected after inspecting the census. The independent chance seed
is 20261119; the bot seed stays 20261103. Frozen-root transfer optimizes
continuation on the new finite game; it is not a full frozen-policy match test.

## Artifacts and provenance

`summary.json` contains every case and the convergence checkpoints' aggregate.
`final-results/`, `refine-results/`, `depth1-results/` and `transfer-results/`
retain value brackets, root policies and game/runtime hashes. `cpu-check/`
records the overlapping CPU comparison. `wasm-hidden-witness.json` contains
self-contained replay inputs, visible-history hashes and exact search outputs.

The original census manifest predates a setup-action-label disambiguation and
the mathematically equivalent TPU gather implementation. The normalization
manifest records original/final game hashes: only duplicate setup labels were
prefixed with a type index; sequences, information keys, masses, payoffs and
native choices were unchanged. Current exports emit unique labels directly.
Result runtime hashes identify the actual solver used. Final source hashes
are recorded separately. Engine inputs and production code were unchanged.

Large raw game trees remain in ignored `tmp/ico-v1`; regenerate them with the
export commands below. Hashes identify collected artifacts; timing fields and
floating-point device differences mean regenerated bytes need not match.

## Reproduction

```sh
cargo build --release -p nc2000-bot --example ico_export
python3 tools/ico-audit.py --out tmp/ico-repeat --horizon 2 --samples 4 --threads 4
python3 tools/ico-audit.py --out tmp/ico-independent --opponents rental-cban-8 sample-07 \\
  --horizon 2 --samples 8 --seed 20261119 --max-nodes 12000000
python3 tools/ico-audit.py --out tmp/ico-depth --opponents rental-cban-8 sample-07 \\
  --horizon 1 --samples 64 --seed 20261119
node tools/ico-wasm-parity.cjs data/ico-v1/wasm-hidden-witness.json
```

Solve exported games using `gen2_tpu.ico`, or upload them and the module to
Colab and call `tools/ico-colab-worker.py`. After collecting results,
`tools/ico-report.py` verifies hashes and generates the summary and review page.

Validation: 6 native exporter tests; 9 Python solver tests on CPU and TPU;
9 existing blind-search tests; 3 primary native/WASM cases and 4 witness cases;
self-contained witness replay. Hidden-bit non-revelation, perfect recall,
Kuhn poker's exact value and unresolved-frontier reporting are covered.
''')
    print(json.dumps({k:v for k,v in summary.items() if k not in ("census","limitations","backend")}, indent=2))


if __name__ == "__main__":
    main()
