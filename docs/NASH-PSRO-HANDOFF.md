# Disclosed-Pool Nash Equilibrium Search (PSRO Handoff)

Last updated: 2026-11-20 (HEAD prior to this doc: `20ee1b61`)

This document records the current state, verified production `243k/729k` measurements, tooling, measured dead ends, and exact resume point for the NC2000 disclosed-pool Nash equilibrium search.

---

## 1. Exact Game Definition & Stopping Criterion

- **Disclosed Pool**: $P = \{(S_k, p_k, q_k)\}_{k=1}^K$ where $S_k$ are **pairwise-distinct** public Turn-0 shells (equivalence classes of legal 6-Pokémon rosters whose opponent-side Turn-0 `PlayerChannel` frame is byte-identical), outer weights $p_k > 0$ ($\sum_k p_k = 1$), and inner conditional variant distributions $q_k \in \Delta(\mathcal{V}(S_k))$.
- **Production Search Settings** (`data/live-search.json`): `unknownPreview: 729000`, `battle.adaptive: 243000` (13.5k check interval / 0.01 gap), `c = 0.4`, `legacy` policy, 500-turn cap = `0.5`, seat-swapped pairs.
- **Shell-Level Bilinear Decomposition** (`crates/bot/src/belief.rs:1181-1225`): Once Turn 0 reveals shell $S_k$, `Belief` filters `alive` strictly to $S_k$ and renormalizes within $S_k$. Outer payoffs are bilinear in $p$, so archived production games keyed by canonical `(profile_hash_A, profile_hash_B)` are 100% reusable across all outer reweightings. Inner camouflage $q_k$ is nonlinear (measured shifts of `+12.5` to `+14.4 pp` in `data/nash-camouflage-v1/screen-summary.json`).
- **Rule Invariant**: Outer Nash matrices MUST obey **1 distinct public shell = 1 row**; multi-variant shells enter as a single `(S_k, q_k)` row.
- **Stopping Criterion**: Worst-case legal single-shell challenger $(T, u)$ achieves one-sided 95% upper confidence bound $\le 55\%$ ($\varepsilon_{\text{tol}} = +5\text{ pp}$) at production `243k/729k` on independent holdout seeds.

---

## 2. Production Tooling & Dual-Node Compute (`mini1` + `mini2`)

All tooling is committed on `master` (through `20ee1b61`):

- **Native Runner**: `crates/bot/examples/nash_refresh.rs` built with 2-stage LLVM CS-PGO (`tools/build-native-pgo.sh`, binary at `target/native-pgo/release/examples/nash_refresh`). Verified **8/8 exact `actionSha256` parity** vs WASM (`data/nash-camouflage-v1/colab-parity.json`) and **56/56 bit-exact** Turn-0 `NASH_PREVIEW_CACHE=1`.
- **Dual-Node Dispatcher**: `tools/mini2_nash.py` dispatches chunks across `mini1` (local Apple M6, 10 workers) and `mini2` (subordinate Apple M6, 11 workers via `mini2-barn`, per `~/agents/operations/mini2.md`) using dynamic work-stealing and atomic staging/collection.
  - Observed combined production `243k/729k` throughput: **~1,252 games/hr** vs 2-variant `S07_70_30` (280 games in 13.4 min); **~400–600 games/hr** on heavy 4v×5v multi-variant shells.
  - Google Colab / Spot-VM / TPU offloading is permanently retired (`AGENTS.md` §4).
- **Shell-Level Round-Robin**: `tools/nash-roundrobin.py` (`verify`, `prepare`, `run --mini2`, `analyze`, `self-test`). Enforces single-shell rows, pairwise-distinct `shell_hash`, Node WASM Turn-0 byte-identical/distinct `PlayerChannel` frame verification, exact `profile_hash`-matched ledger reuse, stratified conditional weighting, RM+ Nash + bootstrap, and per-variant `by_variant` diagnostics vs $P^*$.
- **PSRO Challenger Screener**: `tools/nash-screen.py` (`prepare`, `run --mini2 --merge-ledger`, `summarize`). Supports singleton and multi-variant single-shell challengers, exact `covered_mass` reporting, and stratified one-sided 95% bounds.
- **Statistics & Ledger**: `tools/nash-stats.py` (Betting CS, stratified Empirical Bernstein/Hoeffding, RM+ Nash + bootstrap, UCB-gated `next_cells` requiring `ev + 1.645 * ev_se > 0.50`) and `tools/nash-ledger.py`.

---

## 3. On-Disk Artifacts (Gitignored under `target/` — Do Not Blindly Wipe)

> [!IMPORTANT]
> `target/nash-ledger.jsonl` holds **1,994 unique production `243k/729k` games** (997 complete seat pairs), and `target/nash-work/` holds all PSRO & catalog data. Preserve these paths before running any `cargo clean` or `rm -rf target`.

- `target/nash-ledger.jsonl`: **1,994** unique production `243k/729k` games (1,444 archived + 270 from $P^{(0)}$ + 280 from PSRO Iteration 1).
- `target/nash-work/swiss-v1/catalog.json`: Validated 691-team legal catalog (510 distinct shells, 0 illegal).
- `target/nash-work/phase1-cut/`: 50,460 games at `9k/81k` against 5 fixed anchors (661 non-dummy teams at uniform 60g + top 180 at +60 fresh games).
  - **Use strictly as a search priority queue**, NOT an aggressive permanent cut. Only the conservative Phase-1 cut (`overall_u95 < 0.40` with shell protection = 103 teams + 30 synthetic `supp-cov-*` fillers) is retained as eliminated; 558 teams (387 shells) remain active.
- `target/nash-work/psro-v1/pools-p0.json`: Verified 4-shell `P0` pool (`S07_70_30`, `APEX4`, `D5`, `sample06`).
- `target/nash-work/psro-v1/p0-matrix/rr-report.json`: Complete 4×4 production `243k/729k` $P^{(0)}$ report.
- `target/nash-work/psro-v1/iter1-br-vs-s07/summary-prod.json`: Complete PSRO Iteration 1 production `243k/729k` challenger screen vs `S07_70_30`.

---

## 4. Production `243k/729k` Results So Far

### 4.1 PSRO Iteration 0 — Initial 4-Shell Production Matrix $P^{(0)}$

372 complete production games (186 seat pairs: 51 reused from ledger + 270 new games), **100% covered across all 82 variant-pairs (`min_subcell_pairs >= 2` everywhere)**:

| Shell | vs `S07_70_30` | vs `APEX4` | vs `D5` | vs `sample06` | Mean vs Field | RM+ Nash (Boot P10 / Med / P90) |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: |
| **`S07_70_30`** | `50.0%` | `58.45% ± 6.75%` (26p) | `53.33% ± 7.84%` (24p) | `55.42% ± 9.35%` (24p) | `55.73%` | **`100.0%`** (`0% / 54.5% / 100%`) |
| **`APEX4`** | `41.55% ± 6.75%` | `50.0%` | `56.75% ± 7.84%` (40p) | `55.00% ± 5.82%` (32p) | `51.10%` | `0.0%` (`0% / 0% / 53.7%`) |
| **`D5`** | `46.67% ± 7.84%` | `43.25% ± 7.84%` | `50.0%` | `56.25% ± 7.00%` (40p) | `48.72%` | `0.0%` (`0% / 0% / 60.9%`) |
| **`sample06`** | `44.58% ± 9.35%` | `45.00% ± 5.82%` | `43.75% ± 7.00%` | `50.0%` | `44.44%` | `0.0%` (`0% / 0% / 38.5%`) |

Key findings from $P^{(0)}$:
- All 3 `S07_70_30` leads were statistically non-significant at 5% one-sided ($z = +0.42$ to $+1.25$; on the 9 fresh `S07_70_30 vs APEX4` pairs alone, the split was `50.0%` vs `67.6%` in archive).
- `S07_70_30` picked `Zapdos Lv55 + Cloyster Lv50 + Snorlax Lv50` in **92/92 (100%)** of new `P0` games, and all 3 `P0` opponents (`APEX4`, `D5`, `sample06`) share species axes (`Starmie/Gengar/Marowak/Skarmory/Suicune/Raikou`) favorable to `Sample07`.
- `by_variant` diagnostics (production `243k/729k`):
  - `S07_70_30`: `sample-07` (Leftovers, 30%) beat `APEX4` `58.3%`, `D5` `65.0%`, `sample06` `58.3%`; `g2-c4` (Pink Bow, 70%) scored `58.5%` / `48.3%` / `54.2%`.
  - `APEX4`: `apex-c04` (40%) was by far the strongest (`55.0% ± 6.1%` vs `S07_70_30`, `81.3%` vs `sample06`), while `c08` (`33.3%`), `r1-s0148` (`35.0%`), and `r2-s0142` (`31.0%`) lagged heavily vs `S07_70_30`.
  - `D5`: `g2-c3-legal` (`60.0%`), `g3-c0-legal` (`54.2%`), `s0030` (`50.0%`) competed with `S07_70_30`; `g3-c5-legal` dragged (`23.3%` vs `S07_70_30`, `25.0%` vs `APEX4`).
  - `sample06`: `sample-06` (`55.0%`) and `s0199` (`51.7%`) held up vs `S07_70_30`; `s0082` dragged (`28.3%`).

### 4.2 PSRO Iteration 1 — 10 Diverse External-Shell Challengers vs `S07_70_30`

280 production `243k/729k` games (14 stratified pairs = 28 games each, 100% `covered_mass`, 13.4 min on `mini1 + mini2`):

| Rank | Challenger (Bare Singleton) | vs `S07_70_30` | $z$ (L95) | vs `g2-c4` (70%) / `orig` (30%) | Species |
| :---: | :--- | :---: | :---: | :---: | :--- |
| **1** | **`mjj-2014-thdrcomb__56b67d`** | **`69.44% ± 9.10%`** | **`+2.14` (`54.5%`)** | `77.8%` / `50.0%` | Zapdos, Tyranitar, Venusaur, Steelix, Cloyster, Snorlax |
| **2** | **`mjj-2010-firecomb__ddddbb`** | **`69.44% ± 9.71%`** | **`+2.00` (`53.5%`)** | `77.8%` / `50.0%` | Moltres, Cloyster, Snorlax, Gengar, Machamp, Raikou |
| **3** | **`rental-cban-8`** | **`68.56% ± 10.17%`** | **`+1.83` (`51.8%`)** | `72.2%` / `60.0%` | Tyranitar, Exeggutor, Venusaur, Golem, Snorlax, Miltank |
| 4 | `deme-r3-b2-s0013` | `55.11% ± 9.83%` | `+0.52` | `44.4%` / `80.0%` | Raikou, Starmie, Venusaur, Golem, Snorlax, Miltank |
| 5 | `rental-cban-6__4681ec` | `50.89% ± 8.98%` | `+0.10` | `55.6%` / `40.0%` | Zapdos, Miltank, Snorlax, Cloyster, Exeggutor, Machamp |
| 6 | `supp-deme-r1-00-s0042` | `50.00% ± 9.52%` | `0.00` | `50.0%` / `50.0%` | Zapdos, Tyranitar, Heracross, Steelix, Machamp, Snorlax |
| 7 | `pb-74` | `47.00% ± 9.98%` | `-0.30` | `50.0%` / `40.0%` | Zapdos, Miltank, Exeggutor, Golem, Cloyster, Snorlax |
| 8 | `pb-91__ddca9f` | `44.00% ± 9.03%` | `-0.66` | `50.0%` / `30.0%` | Porygon2, Zapdos, Golem, Snorlax, Suicune, Venusaur |
| 9 | `mjj-2013-trinormal` | `40.44% ± 7.16%` | `-1.33` | `27.8%` / `70.0%` | Snorlax, Electrode, Tauros, Cloyster, Porygon2, Skarmory |
| 10 | `deme-r1-04-s0053` | `40.11% ± 7.92%` | `-1.25` | `44.4%` / `30.0%` | Electrode, Suicune, Cloyster, Marowak, Snorlax, Zapdos |

Key findings from Iteration 1:
- **3 distinct bare-singleton archetypes statistically beat `S07_70_30` at ~69% (`L95 > 51.8%`)**, primarily exploiting Pink Bow Snorlax (`g2-c4`, 72–78%), while `rental-cban-8` beat both `Sample07` variants (`72.2%` / `60.0%`).
- Catalog siblings of all 3 winning shells (`mjj-2014-thdrcomb__75f85b`, `mjj-2010-firecomb`, `pb-78`) have identical moves/items, so none of the 3 winning shells have camouflage variants yet.

---

## 5. Measured Dead Ends & Anti-Patterns (Do Not Repeat)

1. **`3k/9k` surrogate** ($r = 0.110$) and **MLMC same-seed control variates** (`pair_corr = 0.267`, `1.08×` gain): rejected.
2. **Same-shell duplicate rows** (`rr21-v1`): violates game rules; blocked by `tools/nash-roundrobin.py`.
3. **Aggressive singleton cutting against camouflaged anchors**: applying `T3_StrongCore3` / `EB < 20%` permanently cut `0/22` `APEX` variants (including all 4 `APEX4` core types) due to `+12.5–14.4 pp` camouflage asymmetry. Never use bare-singleton `9k/81k` scores for aggressive permanent cuts.
4. **Deriving top-meta inner weights from `9k/81k` 661-team junk-field win rates**: `apex-c04` vs `r1-s0148` and `g2-c4` vs `sample-07` inverted completely between 661-team `9k/81k` and production `243k/729k` top-shell matchups. Always optimize inner weights $q_k$ strictly against the active production Nash support $P^{(t)}$.
5. **Conflating selection games with fresh holdout games**: pooling Step 2a + Step 2b hid `-12.6 pp` Winner's Curse regression on Top 15. Always evaluate stopping criteria on strictly independent holdout seeds.

---

## 6. Resume Point & Convergence Plan (Next Session)

PSRO converges (does **not** chase endlessly) once the active mixture $P^{(t)}$ spans the major complementary archetype pillars: specialist counters that exploit a single shell (like `100% S07_70_30`) lose to the other pillars in $P^{(t)}$, pushing the worst-case challenger win rate monotonically down toward `50%` until $\text{UCB}_{95} \le 55\%$.

Recommended resume sequence:
1. **Expand $P^{(0)} \to P^{(1)}$ (7×7 production `243k/729k` matrix)**: Add `mjj-2014-thdrcomb__56b67d`, `mjj-2010-firecomb__ddddbb`, and `rental-cban-8` alongside `{S07_70_30, APEX4, D5, sample06}` using `tools/nash-roundrobin.py prepare --ledger target/nash-ledger.jsonl` + `run --mini2`. Reuses all 372 `P0` games + 84 Iteration-1 games; needs ~260–280 new production games (~15 min on `mini1 + mini2`) to establish the first multi-pillar equilibrium $P^{(1)}$.
2. **Targeted Camouflage & Inner-Weight Refinement (only for positive-support shells of $P^{(1)}$)**:
   - Re-evaluate `Sample07` with higher `sample-07` Leftovers weight (e.g., `50/50` or `30/70`), prune dead-weight variants in `APEX4` / `D5` if supported, and craft Turn-0-identical camouflage variants for whichever new shells enter $P^{(1)}$'s support.
3. **PSRO Iteration 2+ (BR against *mixed* $P^{(t)}$)**: Search for any catalog shell or `teamgen` mutant beating the multi-pillar mixture $P^{(t)}$ at production `243k/729k`, stop once worst-case challenger one-sided 95% UCB $\le 55\%$ on independent holdout seeds.
