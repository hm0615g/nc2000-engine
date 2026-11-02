//! Build a finite information-constrained battle game for gen2_tpu.ico.
//! Research-only: no production source, profile, or compiled preview changes.
//! Information keys are own sets + complete PlayerChannel frames + own actions.
//! Hidden world IDs, PRNG state, opponent choices and true stats NEVER enter keys.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Instant;

use conformance::{fixture::repo_root, load_dex};
use nc2000_bot::{
    compiled_preview,
    eval::{eval_leaf, EvalWeights},
    player::{action_input, PlayerChannel, PlayerFrame},
    preview::{load_meta_pool, MetaPool, MetaTeam},
    smmcts::SelRule,
    Belief, BlindSearch, Observer, RmConfig, SplitMix64,
};
use nc2000_engine::{
    battle::{Outcome, PokemonSet, SearchChoice},
    dex::Dex,
    prng::{BattleRng, Prng},
    state::Battle,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InformationSet {
    key: String,
    parent: usize,
    actions: Vec<String>,
    sequences: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline: Option<Vec<f64>>,
}

#[derive(Default, Serialize)]
struct Player {
    sequence_count: usize,
    infosets: Vec<InformationSet>,
    #[serde(skip)]
    index: HashMap<String, usize>,
}

impl Player {
    fn new() -> Self {
        Self {
            sequence_count: 1,
            ..Self::default()
        }
    }

    fn intern(&mut self, key: String, parent: usize, actions: Vec<String>) -> usize {
        if let Some(&i) = self.index.get(&key) {
            let old = &self.infosets[i];
            assert_eq!(
                old.parent, parent,
                "imperfect recall: identical observation with different own action sequence"
            );
            assert_eq!(
                old.actions, actions,
                "hidden-state-dependent legal action set"
            );
            return i;
        }
        let i = self.infosets.len();
        let sequences: Vec<_> =
            (self.sequence_count..self.sequence_count + actions.len()).collect();
        self.sequence_count += actions.len();
        self.index.insert(key.clone(), i);
        // Structural equality above governs merging. Hashes only compact the
        // exported labels; the Python validator rejects any duplicate labels.
        let label = format!("sha256:{:x}", Sha256::digest(key.as_bytes()));
        self.infosets.push(InformationSet {
            key: label,
            parent,
            actions,
            sequences,
            baseline: None,
        });
        i
    }
}

// Observation histories are append-only and include own attempted actions (also
// when the public log hides a failed/sleep-cancelled choice). No information is
// gained merely because the exporter enumerated another player's action first.
#[derive(Clone)]
struct History {
    own: Value,
    events: Vec<Value>,
    cursor: usize,
}

impl History {
    fn new(own: &[PokemonSet]) -> Self {
        Self {
            own: serde_json::to_value(own).unwrap(),
            events: vec![],
            cursor: 0,
        }
    }
    fn observe(&mut self, b: &mut Battle, dex: &Dex, side: usize) -> PlayerFrame {
        // Reconstruct the channel to avoid changing the production PlayerChannel.
        let mut frame = PlayerChannel::new(side).frame(b, dex).unwrap();
        assert!(self.cursor <= frame.lines.len());
        let next = frame.lines.len();
        frame.lines = frame.lines[self.cursor..].to_vec();
        self.cursor = next;
        self.events.push(json!({"observation": frame}));
        frame
    }
    fn act(&mut self, action: &str) {
        self.events.push(json!({"own_action": action}));
    }
    fn key(&self) -> String {
        json!([self.own, self.events]).to_string()
    }
}

struct Exporter<'a> {
    dex: &'a Dex,
    pool: &'a MetaPool,
    players: [Player; 2],
    payoffs: Vec<(usize, usize, f64, f64, f64, f64)>,
    baseline_payoffs: Vec<(usize, usize, f64, f64, f64, f64)>,
    horizon: usize,
    samples: usize,
    seed: u64,
    bot_seed: u32,
    bot_iterations: u32,
    baseline_calls: usize,
    baseline_audits: usize,
    baseline_disagreements: usize,
    nodes: usize,
    max_nodes: usize,
    baseline_records: Vec<Value>,
    exact_chance: bool,
    chance_cap: usize,
}

impl Exporter<'_> {
    fn baseline(&mut self, b: &Battle, obs: &Observer) -> Vec<f64> {
        let belief = Belief::new(self.dex, self.pool, obs);
        let cfg = RmConfig {
            rule: SelRule::Ucb,
            c: 0.4,
            hp_buckets: 16,
            ..Default::default()
        };
        let mut rng = SplitMix64::new(self.bot_seed as u64);
        rng.next(); // observe at compiled preview
        let search_seed = rng.next(); // observe at first battle decision
        let mut search = BlindSearch::new(b, self.dex, cfg, 0, search_seed);
        let start = Instant::now();
        // Exactly the fixed-budget WasmBlindSearcher step/best path, no
        // BlindAgent early stop, root pruning, open pinning or timed solver.
        search.step(self.dex, &belief, obs, self.bot_iterations);
        assert_eq!(search.iterations(), self.bot_iterations);
        let picked = search.best().unwrap();
        self.baseline_records
            .push(json!({"iterations":search.iterations(),
            "seed": search_seed, "browser_constructor_seed":self.bot_seed,
            "seconds": start.elapsed().as_secs_f64(),
            "choice": action_input(self.dex, picked), "visits": search.visits(),
            "actions":search.actions().iter().map(|c|action_input(self.dex,*c)).collect::<Vec<_>>(),
            "means":search.means()}));
        self.baseline_calls += 1;
        search
            .actions()
            .iter()
            .map(|c| if *c == picked { 1. } else { 0. })
            .collect()
    }

    fn expand(
        &mut self,
        mut b: Battle,
        mut history: [History; 2],
        parent: [usize; 2],
        mass: f64,
        depth: usize,
        root_obs: &Observer,
        baseline_path: bool,
    ) {
        self.nodes += 1;
        assert!(
            self.nodes <= self.max_nodes,
            "ICO export node limit exceeded; no partial artifact written"
        );
        let terminal = b.outcome().map(|o| match o {
            Outcome::P1Win => 1.,
            Outcome::P2Win => 0.,
            Outcome::Tie => 0.5,
        });
        if terminal.is_some() || depth == self.horizon {
            let value =
                terminal.unwrap_or_else(|| eval_leaf(&b, self.dex, &EvalWeights::default()));
            let (lo, hi) = terminal.map_or((0., 1.), |v| (v, v));
            self.payoffs
                .push((parent[0], parent[1], mass, value, lo, hi));
            if baseline_path {
                self.baseline_payoffs
                    .push((parent[0], parent[1], mass, value, lo, hi));
            }
            return;
        }
        let frames: [PlayerFrame; 2] =
            std::array::from_fn(|s| history[s].observe(&mut b, self.dex, s));
        let choices: [Vec<SearchChoice>; 2] = std::array::from_fn(|s| b.legal_choices(self.dex, s));
        let ids: [Option<usize>; 2] = std::array::from_fn(|s| {
            if choices[s].is_empty() {
                None
            } else {
                Some(self.players[s].intern(
                    history[s].key(),
                    parent[s],
                    frames[s].legal_actions.clone(),
                ))
            }
        });
        assert!(
            ids.iter().any(Option::is_some),
            "nonterminal state without any decision"
        );
        let mut actual_baseline = None;
        if depth == 0 {
            if let Some(i) = ids[0] {
                let previous = self.players[0].infosets[i].baseline.clone();
                let baseline = self.baseline(&b, root_obs);
                actual_baseline = baseline.iter().position(|p| *p > 0.);
                if let Some(p) = previous {
                    if p != baseline {
                        self.baseline_disagreements += 1;
                    }
                    self.baseline_audits += 1;
                } else {
                    self.players[0].infosets[i].baseline = Some(baseline);
                }
            }
        }
        for a in 0..choices[0].len().max(1) {
            for z in 0..choices[1].len().max(1) {
                let indices = [a, z];
                let picked = std::array::from_fn(|s| choices[s].get(indices[s]).copied());
                let seq = std::array::from_fn(|s| {
                    ids[s].map_or(parent[s], |i| {
                        self.players[s].infosets[i].sequences[indices[s]]
                    })
                });
                let mut next_history = history.clone();
                for s in 0..2 {
                    if let Some(c) = picked[s] {
                        next_history[s].act(&action_input(self.dex, c));
                    }
                }
                if self.exact_chance {
                    for (next, prob) in exact_successors(self.dex, &b, picked, self.chance_cap) {
                        self.expand(
                            next,
                            next_history.clone(),
                            seq,
                            mass * prob,
                            depth + 1,
                            root_obs,
                            baseline_path && (depth != 0 || actual_baseline.is_none_or(|i| i == a)),
                        );
                    }
                } else {
                    for k in 0..self.samples {
                        let mut next = b.clone();
                        // Common random numbers across joint actions and hidden worlds.
                        // Seed/sample identity is never part of either information key.
                        let chance_seed = self
                            .seed
                            .wrapping_add((depth as u64 + 1) * 0x10001)
                            .wrapping_add(k as u64 * 0x9e3779b9);
                        next.prng = BattleRng::seeded(Prng::new(chance_seed));
                        next.apply_choices(self.dex, picked).unwrap();
                        self.expand(
                            next,
                            next_history.clone(),
                            seq,
                            mass / self.samples as f64,
                            depth + 1,
                            root_obs,
                            baseline_path && (depth != 0 || actual_baseline.is_none_or(|i| i == a)),
                        );
                    }
                }
            }
        }
    }
}

// Enumerate the raw draw tree without state-only quotienting: two outcomes
// with equal battle state can still have different observation histories.
// Keep all logs, including public misses and cancelled moves. No sampling or
// silent fallback if the requested exact cell exceeds its declared work cap.
fn exact_successors(
    dex: &Dex,
    base: &Battle,
    choices: [Option<SearchChoice>; 2],
    cap: usize,
) -> Vec<(Battle, f64)> {
    let mut pending = vec![(Vec::<usize>::new(), 1.)];
    let mut leaves = vec![];
    let mut runs = 0;
    while let Some((script, mass)) = pending.pop() {
        runs += 1;
        assert!(
            runs <= cap,
            "exact chance cap exceeded; no sampled fallback or partial artifact"
        );
        let mut b = base.clone();
        b.prng = BattleRng::enumerating(script.clone());
        b.apply_choices(dex, choices).unwrap();
        let oracle = b.prng.oracle.take().unwrap();
        if script.len() == oracle.trace.len() {
            leaves.push((b, mass));
        } else {
            let draw = &oracle.trace[script.len()];
            for (i, &count) in draw.counts.iter().enumerate() {
                if count > 0 {
                    let mut prefix = script.clone();
                    prefix.push(i);
                    pending.push((prefix, mass * count as f64 / (1u64 << 32) as f64));
                }
            }
        }
    }
    assert!((leaves.iter().map(|(_, p)| p).sum::<f64>() - 1.).abs() < 1e-8);
    leaves
}

fn coalesce(
    rows: Vec<(usize, usize, f64, f64, f64, f64)>,
) -> Vec<(usize, usize, f64, f64, f64, f64)> {
    let mut cells = BTreeMap::<(usize, usize), [f64; 4]>::new();
    for (a, b, p, v, lo, hi) in rows {
        let cell = cells.entry((a, b)).or_default();
        for (x, y) in cell.iter_mut().zip([p, p * v, p * lo, p * hi]) {
            *x += y;
        }
    }
    cells
        .into_iter()
        .map(|((a, b), [p, v, lo, hi])| {
            (
                a,
                b,
                p,
                (v / p).clamp(0., 1.),
                (lo / p).clamp(0., 1.),
                (hi / p).clamp(0., 1.),
            )
        })
        .collect()
}

fn top_picks(
    b: &mut Battle,
    dex: &Dex,
    side: usize,
    compiled: &Value,
    count: usize,
) -> Vec<(SearchChoice, f64)> {
    let key = compiled_preview::key(b, side).unwrap();
    let mut counts = BTreeMap::<String, usize>::new();
    if let Some(rows) = compiled["policies"][&key].as_array() {
        for row in rows {
            *counts.entry(row.as_str().unwrap().to_string()).or_default() += 1;
        }
    }
    let legal = b.legal_choices(dex, side);
    let mut ranked: Vec<_> = counts
        .into_iter()
        .filter_map(|(input, n)| {
            legal
                .iter()
                .find(|c| c.to_input(dex) == input)
                .map(|c| (*c, n))
        })
        .collect();
    assert!(
        !ranked.is_empty(),
        "compiled preview missing: choose a catalog opponent"
    );
    ranked.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.to_input(dex).cmp(&b.0.to_input(dex)))
    });
    ranked.truncate(count);
    let total: usize = ranked.iter().map(|p| p.1).sum();
    ranked
        .into_iter()
        .map(|(p, n)| (p, n as f64 / total as f64))
        .collect()
}

fn flag(args: &[String], key: &str, default: &str) -> String {
    args.iter()
        .position(|s| s == key)
        .map(|i| args[i + 1].clone())
        .unwrap_or_else(|| default.into())
}

fn public_roster(b: &Battle, side: usize) -> String {
    json!(b.sides[side]
        .roster
        .iter()
        .map(|p| (
            p.species.0,
            p.level,
            format!("{:?}", p.gender),
            p.item.is_some()
        ))
        .collect::<Vec<_>>())
    .to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let foe_id = flag(&args, "--opponent", "sample-07");
    let output = flag(&args, "--out", "tmp/ico-game.json");
    let horizon: usize = flag(&args, "--horizon", "2").parse().unwrap();
    let samples: usize = flag(&args, "--samples", "2").parse().unwrap();
    let picks: usize = flag(&args, "--picks", "2").parse().unwrap();
    let seed: u64 = flag(&args, "--seed", "20261103").parse().unwrap();
    let bot_iterations: u32 = flag(&args, "--bot-iters", "81000").parse().unwrap();
    assert!(horizon > 0 && samples > 0 && picks > 0 && bot_iterations > 0);
    let root = repo_root();
    let dex = load_dex();
    let pool = load_meta_pool(&root.join("data/belief-pool-v3/belief-pool.json"));
    let catalog = load_meta_pool(&root.join("data/team-pool-v2/team-pool.json"));
    let nash: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("data/kodoku-r3/conclusion-equilibrium-deme.json"))
            .unwrap(),
    )
    .unwrap();
    let compiled: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("data/compiled-preview-v1.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        compiled["build"].as_str().unwrap(),
        compiled_preview::build_fingerprint(),
        "stale compiled preview"
    );
    let foe: &MetaTeam = catalog
        .teams
        .iter()
        .find(|t| t.id == foe_id)
        .expect("unknown catalog opponent");
    let own0: Vec<PokemonSet> = serde_json::from_value(nash["teams"][0]["sets"].clone()).unwrap();
    let mut reference = Battle::from_fixture(&dex, "1,2,3,4", &own0, &foe.sets).unwrap();
    let reference_public = public_roster(&reference, 1);
    let foe_picks = top_picks(&mut reference, &dex, 1, &compiled, picks);
    let mut foe_types = vec![(foe.id.clone(), foe.sets.clone())];
    // Preserve preview roster order and all visible fields. A changed hidden
    // loadout may not acquire a different information key just by file order.
    for team in &pool.teams {
        let reordered: Option<Vec<PokemonSet>> = foe
            .sets
            .iter()
            .map(|s| {
                team.sets
                    .iter()
                    .find(|t| t.species == s.species && t.level == s.level)
                    .cloned()
            })
            .collect();
        if let Some(sets) = reordered {
            if sets.len() != team.sets.len() {
                continue;
            }
            let b = Battle::from_fixture(&dex, "1,2,3,4", &own0, &sets).unwrap();
            if public_roster(&b, 1) == reference_public
                && !foe_types.iter().any(|(_, old)| {
                    serde_json::to_value(old).unwrap() == serde_json::to_value(&sets).unwrap()
                })
            {
                foe_types.push((team.id.clone(), sets));
            }
        }
    }
    let mut export = Exporter {
        dex: &dex,
        pool: &pool,
        players: [Player::new(), Player::new()],
        payoffs: vec![],
        baseline_payoffs: vec![],
        horizon,
        samples,
        seed,
        bot_seed: flag(&args, "--bot-seed", "20261103").parse().unwrap(),
        bot_iterations,
        baseline_calls: 0,
        baseline_audits: 0,
        baseline_disagreements: 0,
        nodes: 0,
        max_nodes: flag(&args, "--max-nodes", "5000000").parse().unwrap(),
        baseline_records: vec![],
        exact_chance: args.iter().any(|s| s == "--exact-chance"),
        chance_cap: flag(&args, "--chance-cap", "100000").parse().unwrap(),
    };
    let mut setup_actions = vec![];
    for (type_index, (id, _)) in foe_types.iter().enumerate() {
        for (p, _) in &foe_picks {
            setup_actions.push(format!(
                "type-index={type_index};type={id};{}",
                p.to_input(&dex)
            ));
        }
    }
    let setup_id = export.players[1].intern(
        "opponent-private-setup-before-own-draw-reveal".into(),
        0,
        setup_actions,
    );
    let setup_sequences = export.players[1].infosets[setup_id].sequences.clone();
    let mut worlds = 0;
    let mut world_manifest = vec![];
    let started = Instant::now();
    for own in nash["teams"].as_array().unwrap() {
        let own_sets: Vec<PokemonSet> = serde_json::from_value(own["sets"].clone()).unwrap();
        for (fi, (foe_type, foe_sets)) in foe_types.iter().enumerate() {
            let mut initial = Battle::from_fixture(&dex, "1,2,3,4", &own_sets, foe_sets).unwrap();
            initial.set_log_enabled(true);
            let mut histories = [History::new(&own_sets), History::new(foe_sets)];
            for s in 0..2 {
                histories[s].observe(&mut initial, &dex, s);
            }
            let own_picks = top_picks(&mut initial, &dex, 0, &compiled, picks);
            for &(a, wa) in &own_picks {
                for (zi, &(z, _)) in foe_picks.iter().enumerate() {
                    let mut b = initial.clone();
                    let mut h = histories.clone();
                    h[0].act(&action_input(&dex, a));
                    h[1].act(&action_input(&dex, z));
                    let mut obs = Observer::new(&b, 0);
                    b.apply_choices(&dex, [Some(a), Some(z)]).unwrap();
                    obs.observe(&b, &dex);
                    let weight = own["weight"].as_f64().unwrap() * wa;
                    let foe_sequence = setup_sequences[fi * foe_picks.len() + zi];
                    world_manifest.push(json!({"own":own["id"],"foe":foe_type,"own_pick":a.to_input(&dex),"foe_pick":z.to_input(&dex),"chance_weight":weight,"opponent_setup_sequence":foe_sequence}));
                    export.expand(b, h, [0, foe_sequence], weight, 0, &obs, true);
                    worlds += 1;
                    eprintln!(
                        "worlds={worlds} nodes={} infosets={}/{} elapsed={:.1}s",
                        export.nodes,
                        export.players[0].infosets.len(),
                        export.players[1].infosets.len(),
                        started.elapsed().as_secs_f64()
                    );
                }
            }
        }
    }
    let metadata = json!({"name":"Information-Constrained Oracle", "abbreviation":"ICO", "opponent":foe_id,
        "build":compiled_preview::build_fingerprint(),"horizon_decisions":horizon,"chance_samples":samples,
        "chance":if export.exact_chance {"exhaustive raw draw classes, float64 mass, public logs retained"} else {"sampled-independent-per-node empirical game; CRN across actions/worlds"},
        "exact_chance":export.exact_chance,"seed":seed,
        "own_distribution":"APEX4 shipped weights", "root_model":"adversarial opponent type from every prior loadout sharing public roster; adversarial top-k reference picks; own top-k compiled picks renormalized",
        "opponent_types":foe_types.iter().map(|t|&t.0).collect::<Vec<_>>(),
        "picks_per_type":picks,"worlds":world_manifest,"nodes":export.nodes,
        "frontier":"eval_leaf estimate plus [0,1] unresolved interval", "baseline_iterations":bot_iterations,
        "baseline_c":0.4,"baseline_hp_buckets":16,"baseline_policy":"legacy best, fixed-budget, no optional endgame solver",
        "baseline_calls":export.baseline_calls,"baseline_hidden_world_audits":export.baseline_audits,
        "baseline_hidden_world_disagreements":export.baseline_disagreements,
        "baseline_records":export.baseline_records,"elapsed_seconds":started.elapsed().as_secs_f64(),
        "information_key":"own team + complete player-visible protocol/request history + own attempted actions",
        "full_game_certified":false});
    let game = json!({"format":"nc2000-ico-sequence-v1","metadata":metadata,"players":export.players,
        "payoffs":coalesce(export.payoffs),"baseline_payoffs":coalesce(export.baseline_payoffs)});
    if let Some(parent) = Path::new(&output).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let file = std::fs::File::create(output).unwrap();
    serde_json::to_writer(std::io::BufWriter::new(file), &game).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn histories_hide_opponent_sets_picks_and_rng_but_reveal_own_actions() {
        let dex = load_dex();
        let pool = load_meta_pool(&repo_root().join("data/team-pool-v2/team-pool.json"));
        let a = &pool.teams[0].sets;
        let mut b = pool.teams[1].sets.clone();
        let mut first = Battle::from_fixture(&dex, "1,2,3,4", a, &b).unwrap();
        b[0].moves.reverse();
        b[0].item = "Leftovers".into();
        let mut second = Battle::from_fixture(&dex, "9,8,7,6", a, &b).unwrap();
        first.set_log_enabled(true);
        second.set_log_enabled(true);
        let mut h1 = History::new(a);
        let mut h2 = History::new(a);
        assert_eq!(
            h1.observe(&mut first, &dex, 0),
            h2.observe(&mut second, &dex, 0)
        );
        assert_eq!(h1.key(), h2.key());
        h1.act("move tackle");
        h2.act("move rest");
        assert_ne!(h1.key(), h2.key());
    }
    #[test]
    fn unrevealed_bench_selection_is_not_an_information_key() {
        let dex = load_dex();
        let pool = load_meta_pool(&repo_root().join("data/team-pool-v2/team-pool.json"));
        let a = &pool.teams[3].sets;
        let b = &pool.teams[0].sets;
        let mut first = Battle::from_fixture(&dex, "1,2,3,4", a, b).unwrap();
        first.set_log_enabled(true);
        let mut second = first.clone();
        let own = first.legal_choices(&dex, 0)[0];
        let foes = first.legal_choices(&dex, 1);
        let lead = match foes[0] {
            SearchChoice::Team(p) => p[0],
            _ => unreachable!(),
        };
        let alt = *foes
            .iter()
            .find(|c| match c {
                SearchChoice::Team(p) => p[0] == lead && **c != foes[0],
                _ => false,
            })
            .unwrap();
        let mut h1 = History::new(a);
        let mut h2 = History::new(a);
        h1.observe(&mut first, &dex, 0);
        h2.observe(&mut second, &dex, 0);
        for h in [&mut h1, &mut h2] {
            h.act(&own.to_input(&dex));
        }
        first
            .apply_choices(&dex, [Some(own), Some(foes[0])])
            .unwrap();
        second.apply_choices(&dex, [Some(own), Some(alt)]).unwrap();
        assert_eq!(
            h1.observe(&mut first, &dex, 0),
            h2.observe(&mut second, &dex, 0)
        );
        assert_eq!(h1.key(), h2.key());
    }
    #[test]
    fn exact_chance_retains_logs_and_unit_mass() {
        let dex = load_dex();
        let pool = load_meta_pool(&repo_root().join("data/team-pool-v2/team-pool.json"));
        let mut b = Battle::from_fixture(&dex, "1,2,3,4", &pool.teams[3].sets, &pool.teams[0].sets)
            .unwrap();
        b.set_log_enabled(true);
        // Preview is a small exact transition including switch-in/order RNG.
        let choices = std::array::from_fn(|s| Some(b.legal_choices(&dex, s)[0]));
        let original = b.log.len();
        let leaves = exact_successors(&dex, &b, choices, 10000);
        assert!(!leaves.is_empty());
        assert!((leaves.iter().map(|(_, p)| p).sum::<f64>() - 1.).abs() < 1e-10);
        assert!(leaves
            .iter()
            .all(|(b, _)| b.log_enabled && b.log.len() > original));
    }
    #[test]
    fn same_information_key_has_one_policy() {
        let mut p = Player::new();
        let i = p.intern("visible".into(), 0, vec!["a".into(), "b".into()]);
        assert_eq!(
            i,
            p.intern("visible".into(), 0, vec!["a".into(), "b".into()])
        );
        assert_eq!(p.infosets.len(), 1);
    }
    #[test]
    #[should_panic(expected = "imperfect recall")]
    fn rejects_recall_loss() {
        let mut p = Player::new();
        p.intern("same".into(), 0, vec!["a".into()]);
        p.intern("same".into(), 1, vec!["a".into()]);
    }
    #[test]
    #[should_panic(expected = "hidden-state-dependent")]
    fn rejects_action_mask_leak() {
        let mut p = Player::new();
        p.intern("same".into(), 0, vec!["a".into()]);
        p.intern("same".into(), 0, vec!["b".into()]);
    }
}
