# Disclosed-Pool Nash Equilibrium Search (PSRO Handoff)

Last updated: 2026-11-22 (`team-pool-v3` flagship `s0053-lefties` defined; `11,380` total strict NO-CACHE LIVE games recorded)

This document records the authoritative state, verified production `243k/729k` & strictly NO-CACHE live holdout measurements, tooling/compute architecture, measured dead ends, standing regulation & statistical constraints, and exact resume point for the NC2000 disclosed-pool Nash equilibrium search.

---

## 1. Exact Game Definition, Legality & Stopping Criterion

- **Disclosed Pool**: $P = \{(S_k, p_k, q_k)\}_{k=1}^K$ where $S_k$ are **pairwise-distinct** public Turn-0 shells (equivalence classes of legal 6-Pokémon rosters whose opponent-side Turn-0 `PlayerChannel` frame is byte-identical), outer weights $p_k > 0$ ($\sum_k p_k = 1$), and inner conditional variant distributions $q_k \in \Delta(\mathcal{V}(S_k))$.
- **Production Search Settings** (`data/live-search.json`): `unknownPreview: 729000`, `battle.adaptive: 243000` (13.5k check interval / 0.01 gap), `c = 0.4`, `legacy` policy, 500-turn cap = `0.5`, seat-swapped pairs.
- **Shell-Level Bilinear Decomposition** (`crates/bot/src/belief.rs:1181-1225`): Once Turn 0 reveals shell $S_k$, `Belief` filters `alive` strictly to $S_k$ and renormalizes within $S_k$. Outer payoffs are bilinear in $p$; inner camouflage $q_k$ is nonlinear (confirmed `+5.2 to +14.4 pp` shifts across `APEX4`, `v2Cz:hpi-whpb`, and `v3:hpiroar-whpb`).
- **Strict Japanese Nintendo Cup 2000 Cart Legality (Standing Rule)**:
  - Every pool member and challenger MUST pass BOTH the WASM `Validator` (`new Validator(new Dex(fs.readFileSync("data/gen2stadium2.json", "utf8")))`, `v.validateTeam(JSON.stringify(team))`) AND strict Japanese NC2000 cartridge legality.
  - Because `data/learnsets-gen2.json` encodes flat Showdown `gen2nintendocup2000noohkostadium2strict` sets (including Event Moves and without cross-move egg-parent checks), manually exclude:
    - NYPC USA event moves (e.g., **`Lovely Kiss` Snorlax** — explicitly excluded by user).
    - Incompatible Gen 2 breeding combinations (e.g., **`Belly Drum + Rock Slide` Marowak**).
- **Robustness & Cleanliness Priorities**:
  - Prioritize broad miscellaneous-field robustness (`Misc-50` high mean, `0/50` swept `0/4` preferred) and zero genetic hitchhikers (species-, move-, item-, and IV/EV-level).
- **Stopping Criterion (Certification Bar)**:
  - Worst-case legal single-shell challenger — **explicitly including ALL in-pool shells themselves and all same-shell bare component deviations** — achieves one-sided 95% pair-level Student-t upper confidence bound **`UCB95_t <= 55.0%`** ($\varepsilon_{\text{tol}} = +5\text{ pp}$) on strictly uncached (`NASH_PREVIEW_CACHE=0`, `previewSeeds` omitted, `preview.source == 'live'` on `100%` of sides, `cache_hits == 0`) independent holdout seeds at production `243k/729k`.

---

## 2. Production Tooling & Hybrid Compute Fabric

All core tooling is committed on `master` (through `7c139bff`):

- **Native Runners**:
  - Local Apple Silicon CS-PGO binary: `target/native-pgo/release/examples/nash_refresh` (`tools/build-native-pgo.sh`).
  - Cross-compiled Linux `x86_64` binary for Colab workers: `target/colab-x86_64/nash_refresh` (`tools/build-colab-x86.sh`, `zigbuild`).
  - Verified **8/8 exact `actionSha256` parity** vs WASM and **56/56 bit-exact** Turn-0 `NASH_PREVIEW_CACHE=1`.
- **Persistent `K=16` Empirical `Legacy` Preview Distribution Cache (`4b186d12`)**:
  - `NASH_PREVIEW_DIST_CACHE`, `NASH_PREVIEW_DIST_K=16`, `NASH_PREVIEW_DIST_ONLY` in `crates/bot/examples/nash_refresh.rs`, wired through `tools/mini2_nash.py`, `tools/nash-elim.py`, and `tools/nash-goal.py`.
  - Precomputes exact 16-seed empirical `729k` team-preview selection distributions per `(profile_hash_A, profile_hash_B)` so screening/matrix runs avoid `previewSeeds` bucket-correlation collapse while remaining fast.
  - Strict override: `NASH_PREVIEW_CACHE=0` always forces `source = 'live'` (`0` cache hits) for final holdout certifications.
- **Unified `barn` Hybrid Dispatch (`7c139bff`, `tools/mini2_nash.py`)**:
  - Transparently fans chunks across `mini1` (local Apple M6) + `mini2` (LAN Apple M6) + up to `2×` Colab `G4` (`znver5` 48 vCPU, 4 slots × 12t) = **10 parallel compute slots** governed by `~/.config/barn/policy.json`.
  - Observed combined throughput:
    - **`3,848 – 4,723 games/hr`** at `K=16` production `243k/729k`
    - **`3,232 – 3,419 games/hr`** on strictly NO-CACHE LIVE production `243k/729k`
    - **`40,000 – 73,947 games/hr`** at `9k/81k` screening
  - **Chunking Rule**: Because `default_task_timeout_s = 600` and 10 slots execute concurrently, keep chunks fine-grained (`16–24` games/chunk, `>= 20–60` chunks; `--rows-per-chunk 4` for `precompute-preview-dist`) to saturate all 10 slots with zero timeout requeues.
  - Note: Check `barn status` before and after cloud bursts; `AGENTS.md` §4 still prohibits ad-hoc manual Colab notebooks/shells outside `barn`.
- **Strict Data Hygiene Rule (Learned via Audit)**:
  - **NEVER** pool legacy `previewSeeds`-fixed runs (`prod-grand11/12`, `prod-psro9`, `prod-qopt-upgrades`, `prod-ext-audit`, etc.) into current Nash payoff matrices. Those runs reused `3–6` `previewSeeds` per edge and suffered up to `-28pp` out-of-sample holdout regression. Filter equilibrium inputs strictly to `preview.source in ('dist-cache', 'live')` (`K=16` or NO-CACHE LIVE runs).

---

## 3. On-Disk Artifacts (Gitignored under `target/` — Do Not Wipe)

> [!IMPORTANT]
> Preserve `target/nash-work/elim-v1/` and `target/nash-ledger.jsonl` before any `cargo clean` or `rm -rf target`.

- `target/nash-work/elim-v1/preview-dist-k16.json`: **`2,736` exact `K=16` (`729k`) empirical preview distributions** (`2,129` non-pure).
- `target/nash-work/elim-v1/custom-teams.json`: All WASM-validated, strict JP-NC2000-legal custom teams and inner-mixture profiles (`v3:hpiroar-whpb`, `v2Cz:hpi-whpb`, `p53:zap-peck-talk-30`, `s0053:opt-zpr-pwd-30`, `s10:eggy-stun`, `s13:champ-talk`, etc.; regulation-excluded `Lovely Kiss` Snorlax and `Belly Drum + Rock Slide` Marowak variants have been deleted).
- **Three Strictly NO-CACHE Live Holdout Certification Reports (`4,372` uncached `243k/729k` live games, `0` cache hits)**:
  - `target/nash-work/elim-v1/holdout-s0053-pwd-cert-report.json` (`720g` LIVE, `s0053:opt-zpr-pwd-30`)
  - `target/nash-work/elim-v1/holdout-v2Cz-hpi-whpb-report.json` (`1,860g` LIVE, `v2Cz:hpi-whpb`)
  - `target/nash-work/elim-v1/holdout-v3-hpiroar-report.json` (`1,792g` LIVE, `v3:hpiroar-whpb`)
- `target/nash-work/elim-v1/breakers-audit-9k/results.jsonl`: `1,248g` `9k/81k` Misc-50 audit of the 4 `v3` breakers + 8×8 round-robin uncovering the 3-shell RPS Nash equilibrium.
- `target/nash-work/elim-v1/misc-pb84-9k/results.jsonl`: `100g` Misc-50 audit of `pb-84` (`57.0%`, `minPk = 7.0%` on Slot 6 `Umbreon50`).
- `target/nash-work/swiss-v1/catalog.json`: Validated 691-team legal catalog (510 distinct shells, 0 illegal).
- `target/colab-x86_64/nash_refresh`: Prebuilt Linux `x86_64` native runner ready for `barn` Colab `G4` slots.

---

## 4. Verified Experimental Results (`2026-11-20` – `2026-11-22`)

### 4.1 Discovery & Full 6-Slot Optimization of the `s0053` Single-Core Shell

Starting from catalog shell `deme-r1-04-s0053` (`Electrode50 / Suicune55 / Cloyster50 / Marowak55 / Snorlax50 / Zapdos55`), which smashed the 5-shell All-Ironclad pool `P5` at **`76.7% ± 4.1%`** (`K=16`, `z=+6.48`) and set a `78.0%` `Misc-50` record, systematic single-core optimization progressed through five validated stages:

1. **Genetic Stat/Item Cleanup (`s0053:opt-zpr-pwd-30`)**:
   - Removed non-Rest `Mint Berry` on `Zapdos` (`Bright Powder` won a 6-item `744g` `K=16` tournament at `61.7%`), fixed `Zapdos` `ivs=31` / `evs.atk=255` on Drill Peck sets and `ivs.atk=22 / evs.atk=0` on `HP Ice` sets.
   - **720g NO-CACHE Holdout** (`holdout-s0053-pwd-cert-report.json`, `srcs={'live': 1440}`): Pool WR `52.36%`, `3/8` PASS (`cban-6 45.0%`, `s0053-mint-opt 36.25%`, `moudoku 33.75%`); worst challenger was bare `s0053-zpr-opt-powder` (`55.0%`). Disproved Mint-minority camouflage (zero `729k` preview distortion on 4/5 threats).
2. **1-Slot JP-Legal Zapdos Poison Pill (`p53:zap-peck-talk-30`)**:
   - `70% s0053-zpr-opt-powder + 30% p53-zap-peck-talk` (`TBolt/Drill Peck/Rest/SleepTalk @ Bright Powder`): `Misc-50 = 71.0%` (`0/50` swept `0/4`, `minPk 20.5%`), beat `11/14` challengers at `K=16` (`pb-84 57.7%`, `s0043 55.7%`, `hc75 66.5%`), but `Cloyster`'s move hitchhiker (`Curse` without `Surf`) left it broken by `pb-131__65e4fe` (`33.3%`, `n=60`), `s10:eggy-stun` (`42.9%`, `n=60`), and bare `p53-zap-peck-talk` (`42.1%`, `n=40`).
3. **Full Slots 1–4 Cleanup (`v2C-zaptalk`)**:
   - `Electrode`: duplicate `Reflect -> Light Screen`; `Suicune`: `Scope Lens -> Mint Berry` (`+9.3pp`); `Cloyster`: dead `Curse -> Surf` (`+8.5 to +14.8pp`); `Zapdos`: `RestTalk Drill Peck @ Bright Powder` beat `TWave/Reflect` by `+10 to +17.5pp` once Slots 1–4 were clean.
   - Bare `v2C-zaptalk` averaged `62.6%` across 14 `K=16` challengers and `65.6%` (`147/171 >= 50%`) across all 171 S5 shells (`sweep-v2Cz-9k`), but lost to `pb-122` (`29.2%`) and `pb-87` (`29.2%`) at `K=16`.
4. **2-Slot Multi-Axis Camouflage (`v2Cz:hpi-whpb`)**:
   - `50% v2Cz-p-talkhpi` (`HP Ice RestTalk Zapdos + Fire Blast Marowak`) + `50% v2Cz-whpb` (`Drill Peck RestTalk Zapdos + HP Bug Marowak`): `Misc-50 = 73.5%` (`0/50` swept `0/4`, `le25 = 4/50`, `minPk 19.5%`), **all-time 174-shell sweep peak `66.0%` (`153/174 >= 50%`)**.
   - **1,860g NO-CACHE Holdout** (`holdout-v2Cz-hpi-whpb-report.json`, `srcs={'live': 3720}`, defender WR `52.85%`): **9/16 PASS** (`moudoku 35.0%`, `s0043 36.4%`, `pb-63 39.2%`, `hc75 40.0%`, `s13 40.0%`, `s0053-zpr-opt-powder 41.8%`, `cban-11 45.5%`, `v2C-zaptalk 45.0%`, `v2Cz-whpb 46.9%`); `pb-131` flipped to `49.3%` Ch (`UCB 56.8%`); failed on `pb-84` (`57.9%`), `v2Cz-p-talkhpi` (`56.9%`), `pb-122` (`54.5%`), `s10:eggy-stun` (`54.3%`), `pb-6__c5606b` (`52.7%`), `pb-87` (`51.0%`).
5. **Single-Core Peak: `v3:hpiroar-whpb` (50% `v3-hpi-roar` + 50% `v2Cz-whpb`)**:
   - **100% JP NC2000 cart-legal**, **all-time camo `Misc-50 = 77.0%`** (`0/50` swept `0/4`, `le25 = 5/50`, `minPk = 15.5%` pooled / `12.0%` on `v3-hpi-roar` / `19.0%` on `v2Cz-whpb`), 175-shell sweep `63.5%` (`144/175 >= 50%`, all-time lowest `<30%` count `5/175`).

#### Exact Verified Specs of `v3:hpiroar-whpb` (`custom-teams.json`)

Shared 4 slots (identical in both `v3-hpi-roar` and `v2Cz-whpb`):
- **Electrode L50** @ `Miracle Berry` — `Thunderbolt / Explosion / Light Screen / Thunder Wave` (`ivs: all 31`, `evs: all 255`)
- **Cloyster L50 M** @ `Gold Berry` — `Surf / Ice Beam / Spikes / Explosion` (`ivs: all 31`, `evs: all 255`)
- **Marowak L55 M** @ `Thick Club` — `Earthquake / Rock Slide / Hidden Power Bug / Swords Dance` (`ivs: 30/26/26/30/30/30`, `evs: all 255`)
- **Snorlax L50 M** @ `Leftovers` — `Body Slam / Earthquake / Self-Destruct / Curse` (`ivs: all 31`, `evs: all 255`)

Divergent 2 camouflage axes (`50% / 50%`):
- **`v3-hpi-roar` (50%)**:
  - **Suicune L55** @ `Mint Berry` — `Surf / Roar / Rest / Sleep Talk` (`ivs: 31/3/31/31/31/31`, `evs.atk: 0`)
  - **Zapdos L55** @ `Bright Powder` — `Thunderbolt / Hidden Power Ice / Rest / Sleep Talk` (`ivs: 31/22/26/31/31/31`, `evs.atk: 0`)
- **`v2Cz-whpb` (50%)**:
  - **Suicune L55** @ `Mint Berry` — `Surf / Ice Beam / Rest / Sleep Talk` (`ivs: 31/3/31/31/31/31`, `evs.atk: 0`)
  - **Zapdos L55** @ `Bright Powder` — `Thunderbolt / Drill Peck / Rest / Sleep Talk` (`ivs: all 31`, `evs: all 255`)

---

### 4.2 Single-Shell Structural Ceiling Proof (`1,792g` Strict NO-CACHE Holdout on `v3:hpiroar-whpb`)

Report: `target/nash-work/elim-v1/holdout-v3-hpiroar-report.json` (`preview_sources = {'live': 3584}`, `cache_hits = 0`, Defender overall WR = `50.50%`: `v3-hpi-roar 52.34%` [`896g`], `v2Cz-whpb 48.66%` [`896g`]).

| Challenger | LIVE Games | Ch WR | SE | `UCB95_t` | Pass `<=55%` | Notes |
|---|---:|---:|---:|---:|:---:|---|
| **`mjj-2014-offantom`** | 120 | **70.00%** | 3.98% | **76.65%** | FAIL | **Structural breaker #1**: sent `Snorlax+Porygon2(DE/Curse/DT/Recover @ Powder)+Electrode` `120/120`; `v3` sent `Zapdos+Cloyster+Snorlax` `114/120` (Roar Suicune `0.4%` across holdout) |
| **`pb-6__c5606b`** | 112 | **61.61%** | 4.41% | **68.99%** | FAIL | **Structural breaker #2**: Magnet Thunder RestTalk Zapdos + Cloyster + Snorlax |
| **`pb-84`** | 152 | **61.84%** | 3.85% | **68.25%** | FAIL | **Structural breaker #3**: Magnet Thunder RestTalk Raikou + Machamp + Cloyster/Gligar |
| **`pb-122`** | 120 | **58.33%** | 4.47% | **65.81%** | FAIL | **Structural breaker #4**: Mint Raikou + Reflect Exeggutor + Cloyster |
| `v2Cz-p-talkhpi` | 152 | 55.26% | 4.14% | 62.17% | FAIL | Same-shell Fire Blast Marowak / Ice Beam Suicune deviation |
| `v3-hpi-hpb` | 100 | 53.00% | 4.83% | 61.09% | FAIL | Same-shell Ice Beam Suicune + HP Ice Zapdos + HP Bug Marowak |
| `pb-131__65e4fe` | 152 | 51.97% | 4.13% | 58.85% | FAIL (SE) | Near-50% (`z = +0.48`) |
| **`s10:eggy-stun`** | 120 | **45.83%** | 4.49% | **53.34%** | **PASS** | Flipped & certified! |
| **`v3-hpi-roar`** (bare) | 120 | **44.17%** | 4.77% | **52.13%** | **PASS** | Both bare components strictly beaten by the 50/50 mix! |
| **`hc75-3rd-bean`** | 100 | **43.00%** | 5.15% | **51.64%** | **PASS** | |
| **`v2Cz-whpb`** (bare) | 120 | **44.17%** | 4.46% | **51.62%** | **PASS** | |
| **`mjj-type-QA-moudoku`** | 100 | **43.00%** | 4.74% | **50.95%** | **PASS** | |
| **`pb-87`** | 112 | **37.50%** | 4.98% | **45.82%** | **PASS** | Flipped from `70.8%` vs bare `v2C-zaptalk`! |
| **`deme-r2-05-s0043`** | 100 | **31.00%** | 5.32% | **39.93%** | **PASS** | |
| **`s13:champ-talk`** | 112 | **30.36%** | 4.35% | **37.64%** | **PASS** | |

**Key Structural Diagnosis**:
- `v3:hpiroar-whpb` is an extraordinary single-core shell (`8/15` top challengers certified `<=55%` on strict NO-CACHE, beating both of its own bare components `55.83%` each, `Misc-50 = 77.0%`).
- However, **all 4 decisive breakers (`58.3–70.0%`) exploit the fixed species-shell weakness of `s0053`**: zero Ghost type and zero Rock/Steel resist against Normal/Electric/Evasion (`Porygon2` Double Team/Curse and `Magnet Thunder` RestTalk Electric cores). Also note a subtle pick-rate defect on `v3-hpi-roar`: while Slot 2 (`Roar Suicune`) was picked `12.0%` in `Misc-50`, it dropped to **`0.4%`** across the 15 holdout challengers (`v2Cz-whpb`'s `Ice Beam Suicune` held `10.5%`), so `v3:hpiroar-whpb` cannot solve `mjj-2014-offantom` or `pb-84` via inner tweaks alone.

---

### 4.3 Breaker Misc-50 Audit & Discovery of a Clean 3-Shell RPS Nash Equilibrium (`breakers-audit-9k`, `1,248g`)

Auditing the 4 breakers + `pb-131` on `Misc-50` (`200g`, `4g/opp`, except `pb-84` in `misc-pb84-9k`, `100g`, `2g/opp`) and running a full 8×8 round-robin (`16g/edge` at `9k/81k`) revealed a clean, non-dominated 3-shell Rock-Paper-Scissors cycle:

| Shell | Misc-50 | Swept `0%` / `<=25%` | Min Slot Pick (Hitchhiker Audit) | 8×8 RM+ Nash Weight | EV vs Nash |
|---|---:|---:|---|---:|---:|
| **`mjj-2014-thdrcomb__56b67d`** | **`72.1%`** *(misc-gauntlet)* | **`0/50`** *(thdr_30_70)* | **CLEAN (`21.9%` prod)** — Rock/Steel (`Tyranitar55 + Steelix50`) + dual phazing (`Whirlwind/Roar`) + `Sandstorm` | **`45.4%`** | `50.00%` |
| **`pb-84`** | **`57.0%`** *(misc-pb84)* | `11/50` *(2g/opp)* | **HITCHHIKER (`7.0%` Slot 6 `Umbreon50` + `Cloyster @ Berry Juice`)** | **`27.3%`** | `50.00%` |
| **`v3:hpiroar-whpb`** | **`77.0%`** *(misc-v3)* | **`0/50`** / `5/50` | **CLEAN in Misc (`15.5%`)** *(note `Roar Suicune` `0.4%` vs top-15 holdout)* | **`27.2%`** | `50.00%` |
| `pb-131__65e4fe` | `68.5%` | `0/50` / `5/50` | CLEAN (`15.0%` `Steelix50`) | `0.0%` | `49.43%` |
| `pb-122` | `52.5%` | `5/50` / `18/50` | **DEAD (`2.0%` `Electrode50`)** | `0.0%` | `44.31%` |
| `mjj-2014-offantom` | `59.5%` | `2/50` / `10/50` | CLEAN (`14.5%` `Charizard`) | `0.0%` | `42.60%` |
| `s10:eggy-stun` | `66.0%` | — | CLEAN (`21.9%` prod) | `0.0%` | `32.40%` |
| `pb-6__c5606b` | `65.5%` | `2/50` / `10/50` | CLEAN (`11.5%` `Gengar`) | `0.0%` | `27.28%` |

**Why the 3-Shell Triangle Works Structurally (Non-Dominated RPS Cycle)**:
- **`mjj-2014-thdrcomb__56b67d`** crushes `pb-84` (`68.8%` 9k; **`78.1% ± 7.9%`** `n=32` `K=16` prod!), `pb-6` (`81.2%`), `s10` (`81.2%`), `offantom` (`62.5%`), and `pb-122` (`62.5%`).
- **`pb-84`** crushes `v3:hpiroar-whpb` (`81.2%` 9k; **`61.8%`** `n=152` LIVE prod), `pb-131` (`81.2%`), `offantom` (`75.0%`), `pb-6` (`68.8%`), and `pb-122` (`62.5%`).
- **`v3:hpiroar-whpb`** crushes `mjj-2014-thdrcomb__56b67d` (`68.8%` 9k; `s0053` family beats `thdr` `73–81%` at `K=16` prod!), `s13` (`69.6%` LIVE), `s0043` (`69.0%` LIVE), `pb-87` (`62.5%` LIVE), `hc75` (`57.0%` LIVE), and `s10` (`54.2%` LIVE).
- Every off-support challenger in the 8-shell field is suppressed to **`<= 49.43%`** against the 3-shell equilibrium, and **all 3 in-pool shells tie at exactly `50.00%`** (satisfying Nash support optimality!).

#### Exact Catalog Rosters of the Other Two Triangle Pillars (For Next-Session Optimization)

- **`pb-84`** (`data/belief-pool-v3/belief-pool.json`):
  1. `Snorlax L55 M @ Miracle Berry` — `Return / Curse / Sleep Talk / Rest` (`all 31`)
  2. `Raikou L55 @ Magnet` — `Thunder / Hidden Power Ice / Sleep Talk / Rest` (`31/22/26/31/31/31`)
  3. `Machamp L50 M @ Scope Lens` — `Cross Chop / Rock Slide / Sleep Talk / Rest` (`all 31`)
  4. `Cloyster L50 M @ Berry Juice` — `Spikes / Ice Beam / Reflect / Explosion` (`all 31`)  ← **item hitchhiker (`Berry Juice`)**
  5. `Gligar L50 M @ Gold Berry` — `Earthquake / Wing Attack / Screech / Counter` (`all 31`)
  6. `Umbreon L50 F @ Leftovers` — `Mean Look / Charm / Pursuit / Baton Pass` (`31/2/31/31/31/31`)  ← **`7.0%` pick-rate hitchhiker**
- **`mjj-2014-thdrcomb__56b67d`** (`data/kodoku-r3/step7-deme-promotes.json`):
  1. `Zapdos L55 @ Miracle Berry` — `Thunderbolt / Whirlwind / Rest / Hidden Power Ice` (`31/22/26/31/31/31`)
  2. `Tyranitar L55 M @ Leftovers` — `Rock Slide / Earthquake / Ice Beam / Dynamic Punch` (`all 31`)
  3. `Venusaur L50 M @ PRZ Cure Berry` — `Razor Leaf / Leech Seed / Sleep Powder / Synthesis` (`31/6/31/31/31/31`)
  4. `Steelix L50 M @ Polkadot Bow` — `Earthquake / Sandstorm / Roar / Explosion` (`all 31`)
  5. `Cloyster L50 M @ Gold Berry` — `Ice Beam / Spikes / Curse / Explosion` (`all 31`)  ← *note: check `Curse -> Surf` cleanup!*
  6. `Snorlax L50 M @ Mint Berry` — `Body Slam / Fire Blast / Rest / Self-Destruct` (`all 31`)

---

## 5. Measured Dead Ends & Standing Anti-Patterns (Do Not Repeat)

1. **Legacy `previewSeeds` Runs in Nash Matrices**: Pooling runs with `3–6` fixed `previewSeeds` per edge caused `-14 to -28pp` holdout collapse (`sample-10`) and fabricated `deme-r1-00-s0030` advantages (`0` `K=16` games vs `pb-131/s0043/hc75`). Exclude all legacy `previewSeeds` runs from equilibrium math.
2. **Mixing a Dominated Shell (`65% p53 + 35% deme-r1-00-s0030`)**: Violates Nash support (if $A$ strictly beats $B$, pure $A$ exploits any convex mix of $A, B$ at `54–59%`). Always verify non-dominated cycles and **always include every in-pool shell itself in the holdout challenger list**.
3. **Non-JP-NC2000 Moves Passing WASM `Validator`**: Never use `Lovely Kiss` Snorlax (NYPC USA event) or `Belly Drum + Rock Slide` Marowak (incompatible Gen 2 egg moves).
4. **Weak-Variant Camouflage (`s0053-mint-opt` at 30%, `v3:hpiroar-tri20`, `thdr:zap-talk-30-70` vs `APEX4`)**: Camouflage lifts EV iff every component is strong individually AND shifts opponent Turn-0/in-battle beliefs; weak minority fillers (`s0053-mint-opt`, `v3-zpwd-hpi`) drag EV down.
5. **Genetic Hitchhikers**:
   - `s0053` `Cloyster` with `Curse` (cost `-8.5 to -14.8pp`, caused `pb-131` loss) and non-Rest `Mint Berry` Zapdos.
   - `D5` evolved variants (`Skarmory @ Never-Melt Ice/Magnet`, `Raikou @ Mystic Water`).
   - `pb-43` (`0.0%` Gengar), `hc75-3rd-bean` (`1.8%` Cloyster), `pb-122` (`2.0%` Electrode, `Misc-50 52.5%`), `pb-84` (`7.0%` Umbreon50 + `Berry Juice` Cloyster).
6. **Alternative `Snorlax` Bases on `s0053`**: `RestTalk`, `Mint + Fire Blast`, and `Double-Edge + EQ` all underperformed `Body Slam / EQ / Self-Destruct / Curse @ Leftovers` as the `s0053` base.
7. **`3k/9k` Surrogate ($r=0.110$), MLMC Control Variates (`1.08×`), Same-Shell Duplicate Rows, and Aggressive Singleton Cutting**: All permanently rejected.

---

## 6. Exact Next-Session Resume Point

The `barn` daemon is currently idle (`cu_rate_hr: 0.0`, `cu_balance: 2309.75`, `target/colab-x86_64/nash_refresh` built and ready). Execute these three steps in order:

1. **Clean & Optimize `pb-84` → `pb-84_opt` (and optionally check `thdr` Cloyster `Curse -> Surf`)**:
   - Fix `pb-84`'s two blatant hitchhikers: replace `7.0%` `Umbreon L50 @ Leftovers` (e.g., `Gengar50`, `Exeggutor50`, `Zapdos50`, `Skarmory50`, `Steelix50`, or `Starmie50` — verifying strict JP NC2000 cart legality + WASM `Validator`!) and upgrade `Cloyster L50 @ Berry Juice` (e.g., `Gold Berry` / `Leftovers` + `Surf`, shifting `Gligar`/`Umbreon` items cleanly).
   - Screen candidates at `9k/81k` (~2 min on `barn`) targeting **`Misc-50 >= 66%`**, `minPk >= 12%`, and preserving `pb-84`'s decisive wins vs `v3:hpiroar-whpb`, `mjj-2014-offantom`, `pb-6__c5606b`, `pb-122`, and `pb-131__65e4fe`.
   - Also check whether cleaning `mjj-2014-thdrcomb__56b67d`'s `Cloyster` (`Curse -> Surf`) and/or giving `thdr` or `pb-84_opt` clean 2-variant camouflage improves the triangle without breaking RPS balance (and check if `v3-hpi-roar`'s `0.4%` `Roar Suicune` pick rate against top-15 holdout is better kept or swapped to `v2Cz-p-talkhpi` / `v3-hpi-hpb` inside the 3-shell mix).
2. **Production `K=16` (`243k/729k`) 3-Shell Matrix + Top-Challenger Verification**:
   - Precompute missing `K=16` entries into `target/nash-work/elim-v1/preview-dist-k16.json` (`--rows-per-chunk 4` on `barn`).
   - Run strict `K=16` (`243k/729k`, NO legacy `previewSeeds` data!) for the 3×3 triangle `{mjj-2014-thdrcomb_opt, v3:hpiroar-whpb, pb-84_opt}` (`>= 48g/edge`) plus external checks against `{pb-131__65e4fe, mjj-2014-offantom, pb-6__c5606b, pb-122, hc75-3rd-bean, s10:eggy-stun, deme-r2-05-s0043, s13:champ-talk, APEX4, D2/deme-r1-00-s0030, S07_0_100, pb-66, pb-87}` and any bare component variants.
   - Solve exact RM+ Nash weights $P^* = (p_{\text{thdr}}, p_{\text{pb84}}, p_{\text{v3}})$ and confirm every in-pool shell scores `~50%` and every external/bare challenger scores `<= 50%` (add a 4th pillar only if a genuine non-dominated breaker beats the 3-shell mixture at `K=16`).
3. **Final Strictly NO-CACHE (`NASH_PREVIEW_CACHE=0`) Holdout Certification**:
   - Run `tools/nash-goal.py prepare-holdout` + `barn` dispatch + `analyze-holdout` with **`previewSeeds` omitted** (`preview_sources == {'live': 2*N}`, `cache_hits == 0`) at `100–140` games per challenger (`SE <= 3.5–4.0pp`), **explicitly including ALL 3 in-pool shells themselves + all same-shell bare deviations + top external challengers**, to certify **`all_pass_ucb95_t_le_55 == True`**.

---

## 7. Strictly NO-CACHE LIVE Audit, `s0053-lefties` Discovery & `team-pool-v3` Release Pool (`2026-11-22`)

### 7.1 Methodological Audit Findings (Primary-Source Verification)

- **Winner's Curse on Small-$n$ `K=16` Selection**: Across 31 overlapping `(defender, challenger)` edges between `K=16` (`n=20–24/edge`) and strict `LIVE` holdouts, challenger WR jumped **`+6.70pp`** on `LIVE` (`20/31` edges, Stouffer `Z = +3.50`, `p = 0.00023`). `dist-cache` itself is mechanically unbiased (`K16 ChWR >= 40%`: `+1.14pp`), while `K16 ChWR < 40%` regressed `+12.63pp` to the mean (`v2-H` also dropped `61.7% -> 46.5%` on `K=16` retest).
- **`v2Cz:hpi-whpb` vs `v3:hpiroar-whpb` Lateral Trade-off**: Across 12 common LIVE challengers, unweighted defender WR was `52.09%` vs `53.01%` (`+0.92pp`, `p = 0.62`); `v3` worsened against all 4 top breakers (`pb-84`, `pb-6`, `pb-122`, `pb-131`) while `Roar Suicune` dropped to `0.4%` pick rate.
- **`breakers-audit-9k` 3-Shell Cycle Unproven at `243k`**: `9k_ps` correlated only `r = 0.325` (`58.6%` sign agreement) with `243k LIVE`, `breakers-audit-9k` fixed `previewSeeds` (`6` unique on 15/28 edges), 500× bootstrap picked `('thdr', 'pb84', 'pb131')` (`30.2%`) over `('thdr', 'pb84', 'v3')` (`26.4%`), and 18/28 edges are unmeasured at `243k`.
- **Policy Change**: All subsequent evaluations switched to **100% strictly NO-CACHE LIVE (`243k/729k`, `preview.source == 'live'`, `previewSeeds` omitted, `NASH_PREVIEW_CACHE=0`)**.

### 7.2 Strictly NO-CACHE LIVE `v2Cz` Item & Structural Optimization (`7,008` New LIVE Games)

1. **RestTalk Zapdos Item Verification** (`target/nash-work/elim-v1/prod-live-v2Cz-zap-items/results.jsonl`, **`2,520g` LIVE** across 5 items vs `Bright Powder` baseline on top-8 breakers + `96g` H2H):
   - Omnibus 8-breaker mean across all 6 items showed tight clustering (`41.93%–45.57%`, $\chi^2(5) = 1.315$), with **`v2Cz-zi:lefties`** (`Zapdos @ Leftovers` + `Snorlax @ Pink Bow`) ranking **#1 on every metric**: 8-breaker mean `45.57%` vs `Powder` `43.96%`, H2H `51.0%` (`96g`), highest Zapdos pick rate (`69.8%`), and worst-breaker floor `39.6%` (`offantom` jumped `28.3% -> 45.8%`, `p = 0.0075` vs other 5 items).
   - `PRZ Cure Berry` (`41.93%`, Zap pick `49.2%`) and `Miracle Berry` (`42.71%`) trailed; `HPI` vs `WHPB` complementarity confirmed across `2,910g` LIVE (`pb-87 +20.0pp` HPI, `pb-122 +13.1pp` WHPB).
2. **`v4` Structural Variants & Factor Decomposition** (`target/nash-work/elim-v1/prod-live-v4-variants/results.jsonl`, **`2,688g` LIVE**, 7 variants × 384g on identical top-8 breakers):
   - **Factor decomposition resolved**: `v4:powder-laxbow` (`Zap @ Powder`, `Lax @ Pink Bow`, `Suicune @ Leftovers`) collapsed to `18.8%` vs `offantom` (`41.15%` overall), proving **`Leftovers` Zapdos** — not `Pink Bow` alone — drove the `offantom` gain (pooled pure `Leftovers` Zapdos `36.7%` [`88/240`] vs non-`Leftovers` `25.8%` [`93/360`], `+10.8pp`, `z = +2.83`, `p = 0.0046`).
   - **Rejected on LIVE**: `v4:lefties-ww-zap` (`Thunder/Whirlwind`, `36.72%`, `-7.24pp`, `p = 0.008`), `v4:synth-lm-tde` (`38.54%`, `0/8`), `v4:lefties-lax-camo` (`40.36%`, non-Self-Destruct Mint Lax half `37.0%`), `v4:item-camo-left-mag` (`41.15%`, `Magnet vs pb-84` did not replicate).
   - **Top tie**: `v2Cz-zi:lefties` (`45.57%`, best floor) tied `v4:lefties-lax-de` (`45.57%`) ahead of `v4:lefties-thunder` (`44.53%`, only build beating `v2Cz-p-talkhpi` `52.1%`). All 13 `v2Cz`-line builds peaked at `45.6%` across the top-8 exploiters.
3. **Final Independent Holdout Certification of `v2Cz-zi:lefties`** (`target/nash-work/elim-v1/holdout-v2Cz-lefties-report.json`, **`1,800g` LIVE** across 18 challengers; `2,280g` pooled with prior LIVE):
   - **Broad-8 Top Shells**: **`64.45%`** unweighted defender WR (**`8/8` certified `UCB95_t <= 55%`**: `thdr 72.9%`, `hc75 67.7%`, `s13 66.7%`, `s0043 65.6%`, `pb-63 63.5%`, `APEX4 62.5%`, `cban-11 60.4%`, `moudoku 56.2%`).
   - **Top-18 Overall**: **`54.63%`** unweighted defender WR (**`11/18` PASS `UCB95_t <= 55%`**), beating `v2Cz:hpi-whpb` on 14 common LIVE challengers (`53.04%` vs `51.12%`, `+1.92pp`), flipping `v2Cz-p-talkhpi` (`59.4%` holdout / `54.2%` pooled, PASS), zero Winner's Curse (top-8 breakers `45.57%` prior vs `45.31%` independent holdout), and zero dead slots (`s1:19.3% / s2:9.8% / s3:80.7% / s4:19.3% / s5:100.0% / s6:70.9%`).
   - **Worst-Case Exploiters**: Only 2/19 LIVE opponents below 40% pooled defender WR — `pb-84` (`35.1% ± 3.7%`, `n=168`) and `mjj-2014-offantom` (`36.3% ± 4.1%`, `n=168`, `+8.0pp` over `Bright Powder`'s `28.3%`); all remaining `17/19` are `>= 42.3%`.

### 7.3 `team-pool-v3` Release Pool Defined (Relaxed Release Milestone)

- Prioritizing release over strict 50% zero-exploit Nash equilibrium, defined **`data/team-pool-v3/`** via [`tools/build-team-products-v3.js`](../tools/build-team-products-v3.js):
  - [`data/team-pool-v3/team-pool.json`](../data/team-pool-v3/team-pool.json): **`33` teams across `32` pairwise-distinct shells** (`23` distinct Level-55 aces, `training-v3` normalized, `33/33` WASM `Validator` PASS, strict JP NC2000 cart legal, `drawWeight = 1/32` per shell summing to `1.000000`), with **`s0053-lefties`** (`s0053-lefties-hpi` + `s0053-lefties-dp`) as Rank 1–2 flagship strongest core (`nash: true, evolved: true`).
  - [`data/team-pool-v3/flagship-mixture.json`](../data/team-pool-v3/flagship-mixture.json): 50/50 `s0053-lefties` flagship mixture artifact.
  - Next release integration steps (pending confirmation): update `data/belief-pool-v3/belief-pool.json` (`tools/build-known-prior.mjs`), wire `web/` (`data.ts`, `vite.config.ts`, `pdinput*.ts`, `team-pool.ts`, `app.tsx`) to `team-pool-v3` + `flagship-mixture.json`, re-bake `data/compiled-preview-v1.json` (or verify live fallback), and run full web/WASM test suites.
