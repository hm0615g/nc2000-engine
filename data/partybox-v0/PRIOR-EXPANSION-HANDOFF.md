# PartyBox Meta Pool & Prior Expansion Handoff (belief-pool-v4)

## 1. Overview & Context

In this session, we conducted a metagame census against two Japanese Gen2 competitive community repositories:
1. **Po-sense rental database** (`http://psense.lib.net/_/PDINPUT2.cgi?sim=&ACTION=TOP1`): 28 CBAN teams (verified: all already inventoried in `data/team-inventory-v1/inventory.json`).
2. **PartyBox popularity ranking** (`http://www.q9con.net/pokemon/PartyBox/index.php?sort=point`): 128 registered teams crawled, parsed, and canonicalized under `gen2nintendocup2000noohkostadium2strict`.

### Key Empirical Findings
- **103 Legal NC2000 Teams**: 103 of the 128 PartyBox teams strictly comply with NC2000 Stadium 2 strict rules (the remainder were 11 Little Cup Lv5, 6 Horn Drill/Fissure OHKO-allowed, and 8 rule/item mismatches).
- **78 Novel Signatures**: 78 teams possess 6-species signatures completely absent from all prior datasets (`inventory.json`, `team-pool-v2`, `belief-pool-v3`, `majinjima-assassins.json`).
- **APEX4 Resilience**: APEX4 (`vc-kodoku-nash-a`, 4 variants of Snorlax55 / Cloyster50 / Gengar50 / Heracross50 / Marowak50 / Starmie50) achieved **$\ge 50\%$ win rate against every single one of the 123 community challengers** (PartyBox 103 + psense 25), with mean payoff 0.7345 and minimum payoff 0.5487 against `pb-100` (わんおー「カビゴンSA+フォレトス」).

---

## 2. The Architectural Constraint: Opponent Prior Invariant

When considering enriching the standard catalog (`team-pool-v2`), a critical architectural constraint governs bot fairness and human gameplay:
- The bot searches using a Bayesian belief prior (`crates/bot/src/belief.rs`, currently `data/belief-pool-v3/belief-pool.json`).
- If an off-prior archetype (such as `pb-100` Forretress or `pb-46` Meganium) is added to `team-pool-v2` while absent from the prior:
  1. The bot assigns **probability zero** to the opponent's true team archetype and hidden variables (moves, items, DVs).
  2. The bot cannot narrow hypotheses as moves are revealed and falls back to a uniform or degenerate belief.
  3. A human playing that party gains an artificial information asymmetry, causing that party to win excessively ("過剰に勝ち越す").
  4. Furthermore, `tools/compile-preview-artifact.py` enforces `assert not row['fallback'], 'Known catalog public roster absent from belief'`, preventing deployment of catalog teams absent from the prior.

### Decision for this Session
- Enrich `team-pool-v2` **only** with teams already included in `belief-pool-v3`.
- Specifically, `sample-06` (117「寝言カビゴンライコウWA (キチカビ3)」 / `pb-76` / `cban-5`) is already present in `belief-pool-v3` with identical movesets, levels, and DVs (weight 0.007843).
- Promoted `sample-06` to `team-pool-v2` (catalog size: 26 -> 27).
- Persist all PartyBox data and top candidates for a dedicated `belief-pool-v4` expansion session.

---

## 3. Preserved Artifacts & Data Locations

All data crawled, canonicalized, and measured during this investigation has been permanently committed to the repository:

1. **`data/partybox-v0/teams.json`**:
   - 103 legal Gen2 NC2000 teams.
   - Includes full metadata: PartyBox ID (`pb-1` .. `pb-128`), team title, author, point score, 6 Pokemon sets with moves, items, DVs, EVs, and levels.
2. **`data/partybox-v0/eval-report.json`**:
   - Full Bayesian team preview Nash gauntlet evaluation results for all 123 community challengers vs APEX4 (at `ns = 3` seeds).
3. **`data/kodoku-r3/partybox-assassins.json`**:
   - Top 25 hardest PartyBox assassins ranked by lowest APEX4 payoff.
4. **Engine / Gauntlet Tooling**:
   - `crates/bot/examples/meta_eco.rs`:
     - Subcommand `eval-partybox`: Gauntlet evaluator vs `data/partybox-v0/teams.json`.
     - Subcommands `converge` and `apex`: Incorporate `partybox-assassins.json` as external predator benchmark panel.

---

## 4. Top Candidate Archetypes for `belief-pool-v4`

The following archetypes from PartyBox represent metagame niches currently under-represented or absent from `belief-pool-v3`:

| ID | Author & Japanese Name | Key Species & Levels | Core Strategy / Threat | APEX4 Payoff |
|---|---|---|---|---|
| `pb-100` | わんおー「カビゴンSA+フォレトス」 | カビ55 / フォレトス50 / パル50 / カイリキー50 / バナ50 / スターミー50 | **Double Spikes + Rapid Spin + Phaze + Cross Chop**. Heaviest counter to standard stall/spikes. | 0.5487 |
| `pb-85` | はまっちゃった人「寝言カビSA最終バージョン」 | カビ55 / パル50 / カイリキー50 / ハガネール50 / ブラッキー50 / サンダー50 | Heavy physical defense + Steelix boom + Umbreon Mean Look / Charm support. | 0.5598 |
| `pb-62` | はまっちゃった人「55地震寝言ピント♀カビSA」 | カビ55 / ナッシー50 / カイリキー50 / スターミー50 / ハガネール50 / サンダー50 | Scope Lens Female Snorlax + dual boom (Exeggutor + Steelix). | 0.5747 |
| `pb-74` | stoic「サンダータンク2017」 | ミル55 / サンダー55 / パル50 / ナッシー50 / ゴローニャ50 / カビ50 | Zapdos Tank with L50 Golem (Explosion / Rock Slide / Roar phazing). | 0.5819 |
| `pb-46` | よし「55バンギカビWA」 | バンギ55 / カビ55 / パル50 / メガニウム50 / サンダー50 / ガラ50 | **Dual Screens Meganium** (Reflect / Light Screen / Leech Seed / Body Slam) supporting Tyranitar. | 0.6385 |
| `pb-13` | テリー「ガラ爆破」 | ガラ55 / カビ50 / パル50 / ナッシー50 / ゲンガー50 / ハガネール50 | **5-Boom Hyper Offense** paving way for L55 Thick Club Marowak sweep. | 0.6473 |
| `pb-67` | yasu「WAライコウカビ」 | ライコウ55 / カビ55 / ガラ50 / パル50 / カイリキー50 / ナッシー50 | Reflect + Light Screen Raikou55 enabling aggressive setup. | 0.6725 |

---

## 5. Execution Protocol for the Next Session (`belief-pool-v4`)

When undertaking the dedicated prior expansion, follow this step-by-step protocol:

1. **Candidate Clustering & Prior Weight Rebalancing**:
   - Merge the selected PartyBox archetypes into candidate pool.
   - Group candidates by archetype / camouflage clusters (`variantCluster`).
   - Assign prior weights reflecting empirical tournament / metagame frequency.
2. **Build & Freeze `belief-pool-v4`**:
   - Generate `data/belief-pool-v4/belief-pool.json`.
   - Compute and stamp `fnv1a64` fingerprint.
   - Update `data/search-profiles.json` and condition references.
3. **Re-measure Selection & Nash Equilibrium**:
   - Run `team_eval` under `belief-pool-v4` condition (`cond.belief_col` & `cond.belief_row`).
   - Solve Nash equilibrium mixture against the enriched prior.
4. **Catalog Enrichment**:
   - Promote desired novel archetypes (`pb-100` Forretress, `pb-46` Meganium, etc.) to `data/team-selection-v2/selection.json`.
   - Run `node tools/build-team-products-v2.js`.
   - Verify `tools/compile-preview-artifact.py` passes without fallback assertions.
   - Bake complete preview tables and deploy.
