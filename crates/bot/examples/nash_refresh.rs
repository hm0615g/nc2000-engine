//! Native Rust drop-in runner for `web/tests/nash-refresh.mjs` schedules.
//!
//! Usage (from any directory):
//!   nash_refresh GAMES.jsonl NEW_OUTPUT_DIR [workers]
//!
//! Environment variables (matching `web/tests/nash-refresh.mjs` plus native extensions):
//!   NASH_POOLS=FILE          Optional extra `{teams, mixtures}` JSON file.
//!   NASH_SAVE_LOGS=1         Save full battle protocol logs to `NEW_OUTPUT_DIR/logs/<index>.json`.
//!   NASH_PREVIEW_ITERS=N     Override live preview iterations (default: `live-search.json` `unknownPreview`).
//!   NASH_BATTLE_ITERS=N      Override battle iteration cap/budget (default: `live-search.json` `battle.adaptive`).
//!   NASH_BATTLE_MODE=MODE    `adaptive` (default) or `fixed` (fixed `agent.step(N)` per battle decision).
//!   NASH_PREVIEW_CACHE=0|1   Exact singleflight Turn-0 live preview cache (default: `1`).
//!
//! Emits a single-line JSON summary on stdout. Uses `ProtocolAgent` + `PlayerChannel`
//! + `DecisionSampler` with identical configuration to `crates/wasm/src/lib.rs`
//! (`WasmProtocolSearcher`) so `actionSha256` can be verified against WASM runs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use nc2000_bot::compiled_preview;
use nc2000_bot::decision::{DecisionPolicy, DecisionSampler, PolicyMode};
use nc2000_bot::player::PlayerChannel;
use nc2000_bot::preview::{MetaPool, MetaTeam};
use nc2000_bot::smmcts::SelRule;
use nc2000_bot::{ProtocolAgent, RmConfig, SplitMix64};
use nc2000_engine::battle::{Outcome, PokemonSet};
use nc2000_engine::dex::Dex;
use nc2000_engine::state::Battle;
use nc2000_engine::validate::{validate_team, Learnsets};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const CANDIDATE_FILE: &str = "nash-refresh-v1/candidate";
const INCUMBENT_FILE: &str = "kodoku-r3/conclusion-equilibrium-deme";
const TEAM_FILES: [&str; 3] = [
    "kodoku-beq-v1/novel_evolved_teams",
    "team-pool-v2/team-pool",
    INCUMBENT_FILE,
];

#[derive(Clone, Deserialize, Serialize)]
struct Task {
    #[serde(default)]
    block: Value,
    #[serde(default)]
    cell: Value,
    #[serde(rename = "rowSeat")]
    row_seat: usize,
    ids: [String; 2],
    seeds: [u32; 2],
    #[serde(rename = "battleSeed")]
    battle_seed: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    picks: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mixtures: Option<[String; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    policies: Option<[String; 2]>,
}

#[derive(Deserialize)]
struct TeamFile {
    teams: Vec<TeamEntry>,
}

#[derive(Clone, Deserialize)]
struct TeamEntry {
    id: String,
    sets: Vec<PokemonSet>,
    #[serde(default)]
    weight: Option<f64>,
}

#[derive(Deserialize)]
struct ExtraPoolsFile {
    teams: Vec<TeamEntry>,
    mixtures: BTreeMap<String, Vec<MixtureMember>>,
}

#[derive(Deserialize)]
struct MixtureMember {
    id: String,
    weight: f64,
}

#[derive(Deserialize)]
struct LiveSearchCfg {
    #[serde(rename = "unknownPreview")]
    unknown_preview: u32,
    battle: LiveBattleCfg,
}

#[derive(Deserialize)]
struct LiveBattleCfg {
    adaptive: u32,
}

#[derive(Deserialize)]
struct SearchProfilesCfg {
    blind: BlindProfileCfg,
}

#[derive(Deserialize)]
struct BlindProfileCfg {
    c: f64,
}

#[derive(Deserialize)]
struct CompiledPreviewFile {
    format: String,
    #[serde(default)]
    status: Option<String>,
    build: String,
    c: f64,
    #[serde(rename = "priorSha256")]
    prior_sha256: String,
    #[serde(rename = "dexSha256")]
    dex_sha256: String,
    policies: HashMap<String, Vec<String>>,
}

struct CompiledPreviewPicker {
    artifact: CompiledPreviewFile,
    public_rosters: HashMap<String, Option<String>>,
    enabled: bool,
}

impl CompiledPreviewPicker {
    fn load(root: &Path, dex_bytes: &[u8], pool_bytes: &[u8]) -> Self {
        let raw = fs::read_to_string(root.join("data/compiled-preview-v1.json"))
            .expect("read data/compiled-preview-v1.json");
        let artifact: CompiledPreviewFile =
            serde_json::from_str(&raw).expect("parse data/compiled-preview-v1.json");
        let enabled = artifact.format == "nc2000-compiled-preview-v1"
            && artifact.status.as_deref() != Some("retired-training-v3")
            && artifact.dex_sha256 == sha256_hex(dex_bytes)
            && artifact.prior_sha256 == sha256_hex(pool_bytes);
        let mut public_rosters: HashMap<String, Option<String>> = HashMap::new();
        if enabled {
            for key in artifact.policies.keys() {
                let parsed: Value = serde_json::from_str(key).expect("compiled preview key json");
                let own = parsed[0].as_array().expect("compiled preview own array");
                let roster: Vec<Value> = own.iter().map(|p| json!([p[0], p[1]])).collect();
                let facts: Vec<Value> = own
                    .iter()
                    .map(|p| json!([p[0], p[1], p[2], !p[8].is_null()]))
                    .collect();
                let roster_str = serde_json::to_string(&roster).unwrap();
                let facts_str = serde_json::to_string(&facts).unwrap();
                match public_rosters.get(&roster_str) {
                    Some(existing) if existing.as_deref() != Some(facts_str.as_str()) => {
                        public_rosters.insert(roster_str, None);
                    }
                    None => {
                        public_rosters.insert(roster_str, Some(facts_str));
                    }
                    _ => {}
                }
            }
        }
        Self {
            artifact,
            public_rosters,
            enabled,
        }
    }

    fn pick(
        &self,
        agent: &ProtocolAgent,
        seed: u32,
        c: f64,
    ) -> Option<String> {
        if !self.enabled {
            return None;
        }
        if compiled_preview::build_fingerprint() != self.artifact.build || c != self.artifact.c {
            return None;
        }
        let battle = agent.battle()?;
        let observer = agent.observer()?;
        let search = agent.search()?;
        if !search.is_preview() {
            return None;
        }
        let key = compiled_preview::key(battle, agent.side())?;
        let parsed: Value = serde_json::from_str(&key).ok()?;
        let foe_str = serde_json::to_string(&parsed[1]).ok()?;
        let observed: Vec<Value> = observer
            .mons()
            .iter()
            .map(|m| {
                json!([
                    m.species.0,
                    m.level,
                    format!("{:?}", m.gender),
                    m.preview_has_item
                ])
            })
            .collect();
        let observed_str = serde_json::to_string(&observed).ok()?;
        if self.public_rosters.get(&foe_str).and_then(|v| v.as_deref()) != Some(observed_str.as_str())
        {
            return None;
        }
        let choices = self.artifact.policies.get(&key)?;
        if choices.is_empty() {
            return None;
        }
        Some(choices[(seed as usize) % choices.len()].clone())
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn read_data(root: &Path, name: &str) -> String {
    let path = root.join("data").join(format!("{name}.json"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn canonical_no_spaces(s: &str) -> String {
    s.chars().filter(|&ch| ch != ' ').collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BattleMode {
    Adaptive,
    Fixed,
}

impl BattleMode {
    fn as_str(self) -> &'static str {
        match self {
            BattleMode::Adaptive => "adaptive",
            BattleMode::Fixed => "fixed",
        }
    }
}

struct CachedPreview {
    policy: Option<DecisionPolicy<String>>,
}

struct PreviewCache {
    slots: Mutex<HashMap<[u8; 32], Arc<OnceLock<CachedPreview>>>>,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl PreviewCache {
    fn new() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        }
    }

    fn get_or_compute(
        &self,
        key: [u8; 32],
        compute: impl FnOnce() -> Option<DecisionPolicy<String>>,
    ) -> Option<DecisionPolicy<String>> {
        let cell = {
            let mut map = self.slots.lock().unwrap();
            map.entry(key)
                .or_insert_with(|| Arc::new(OnceLock::new()))
                .clone()
        };
        let mut computed = false;
        let entry = cell.get_or_init(|| {
            computed = true;
            CachedPreview { policy: compute() }
        });
        if computed {
            self.misses.fetch_add(1, Ordering::Relaxed);
        } else {
            self.hits.fetch_add(1, Ordering::Relaxed);
        }
        entry.policy.clone()
    }
}

fn preview_cache_key(
    side: usize,
    seed: u32,
    preview_iters: u32,
    blind_c: f64,
    own_sets_json: &str,
    frame_lines: &[String],
    req_json: &str,
    prior: &MetaPool,
    agent: &ProtocolAgent,
    team_json_map: &HashMap<String, String>,
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([side as u8]);
    h.update(seed.to_le_bytes());
    h.update(preview_iters.to_le_bytes());
    h.update(blind_c.to_bits().to_le_bytes());
    h.update((own_sets_json.len() as u64).to_le_bytes());
    h.update(own_sets_json.as_bytes());
    h.update((frame_lines.len() as u64).to_le_bytes());
    for line in frame_lines {
        h.update((line.len() as u64).to_le_bytes());
        h.update(line.as_bytes());
    }
    h.update((req_json.len() as u64).to_le_bytes());
    h.update(req_json.as_bytes());
    if let Some(belief) = agent.belief() {
        let alive = belief.alive();
        h.update([1u8]);
        h.update((alive.len() as u64).to_le_bytes());
        for &idx in alive {
            let cand = &prior.teams[idx];
            let sets_json = team_json_map
                .get(&cand.id)
                .map(String::as_str)
                .unwrap_or("");
            h.update((cand.id.len() as u64).to_le_bytes());
            h.update(cand.id.as_bytes());
            h.update((sets_json.len() as u64).to_le_bytes());
            h.update(sets_json.as_bytes());
            let w_bits = cand.weight.map(f64::to_bits).unwrap_or(u64::MAX);
            h.update(w_bits.to_le_bytes());
        }
    } else {
        h.update([0u8]);
    }
    h.finalize().into()
}

struct SharedState {
    dex: Dex,
    teams: HashMap<String, Vec<PokemonSet>>,
    team_json_map: HashMap<String, String>,
    disclosed: HashMap<String, MetaPool>,
    default_pool: Option<MetaPool>,
    picker: Option<CompiledPreviewPicker>,
    preview_iters: u32,
    battle_iters: u32,
    battle_mode: BattleMode,
    blind_c: f64,
    log_dir: Option<PathBuf>,
    preview_cache: Option<PreviewCache>,
}

fn run_game(shared: &SharedState, index: usize, task: &Task) -> Value {
    let policies = task
        .policies
        .clone()
        .unwrap_or_else(|| ["current".to_string(), "current".to_string()]);
    for p in &policies {
        assert!(
            p == "current" || p == "current81",
            "unsupported policy {p:?} in native nash_refresh (only current / current81 supported)"
        );
    }
    let picks = task.picks.as_deref().unwrap_or(&[]);
    if let Some(mixtures) = &task.mixtures {
        assert!(picks.is_empty() || picks.len() == 2);
        for (side, mix) in mixtures.iter().enumerate() {
            let pool = shared
                .disclosed
                .get(mix)
                .unwrap_or_else(|| panic!("unknown disclosed mixture: {mix}"));
            assert!(
                pool.teams.iter().any(|t| t.id == task.ids[side]),
                "realized id {} missing from mixture {mix}",
                task.ids[side]
            );
        }
    }

    let sets: [Vec<PokemonSet>; 2] = [
        shared
            .teams
            .get(&task.ids[0])
            .unwrap_or_else(|| panic!("unknown team id: {}", task.ids[0]))
            .clone(),
        shared
            .teams
            .get(&task.ids[1])
            .unwrap_or_else(|| panic!("unknown team id: {}", task.ids[1]))
            .clone(),
    ];

    let battle_seed_str = SplitMix64::new(task.battle_seed as u64).battle_seed();
    let mut battle = Battle::from_fixture(&shared.dex, &battle_seed_str, &sets[0], &sets[1])
        .expect("Battle::from_fixture");
    let mut channels = [PlayerChannel::new(0), PlayerChannel::new(1)];

    let cfg = RmConfig {
        rule: SelRule::Ucb,
        c: shared.blind_c,
        hp_buckets: 16,
        ..RmConfig::default()
    };

    let priors: [MetaPool; 2] = std::array::from_fn(|side| {
        if let Some(mixtures) = &task.mixtures {
            shared.disclosed[&mixtures[1 - side]].clone()
        } else {
            shared
                .default_pool
                .as_ref()
                .expect("default belief pool required when mixtures absent")
                .clone()
        }
    });

    let mut agents: [ProtocolAgent; 2] = std::array::from_fn(|side| {
        let mut agent = ProtocolAgent::new(
            &shared.dex,
            side,
            priors[side].clone(),
            cfg.clone(),
            task.seeds[side] as u64,
        );
        agent.set_own_team(sets[side].clone());
        agent
    });
    let mut decisions: [DecisionSampler<String>; 2] =
        std::array::from_fn(|side| DecisionSampler::new(task.seeds[side] as u64));

    let mut preview: [Option<Value>; 2] = [None, None];
    let mut preview_beliefs: [Option<Vec<String>>; 2] = [None, None];
    let mut transcript = Sha256::new();
    let began = Instant::now();
    let mut first = true;

    while battle.outcome().is_none() && battle.turn < 500 {
        let needs = battle.needs_choice();
        let mut inputs: [Option<String>; 2] = [None, None];
        for side in 0..2 {
            if !needs[side] {
                continue;
            }
            let frame = channels[side]
                .frame(&mut battle, &shared.dex)
                .expect("PlayerChannel::frame");
            let agent = &mut agents[side];
            for line in &frame.lines {
                agent.push_line(&shared.dex, line);
            }
            decisions[side].reset();
            let req_json = serde_json::to_string(&frame.request).unwrap();
            let owed = agent
                .on_request(&shared.dex, &req_json)
                .expect("ProtocolAgent::on_request");
            assert!(owed, "expected pending decision on side {side}");

            let mut chosen: Option<String> = None;
            let mut source = "live";

            if task.mixtures.is_some() {
                let belief: Value =
                    serde_json::from_str(&agent.belief_info()).expect("belief_info json");
                let fallback = belief["fallback"].as_bool().unwrap_or(true);
                let candidates: Vec<String> = belief["candidates"]
                    .as_array()
                    .expect("belief candidates")
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect();
                assert!(
                    !fallback && candidates.iter().any(|c| c == &task.ids[1 - side]),
                    "disclosure lost the played team"
                );
                if first {
                    preview_beliefs[side] = Some(candidates);
                }
            }

            if first && task.mixtures.is_none() {
                let is_legal = |pick: &str| {
                    let norm = canonical_no_spaces(pick);
                    frame
                        .legal_actions
                        .iter()
                        .any(|a| canonical_no_spaces(a) == norm)
                };
                if let Some(picker) = &shared.picker {
                    if let Some(pick) = picker.pick(agent, task.seeds[side], shared.blind_c) {
                        if is_legal(&pick) {
                            chosen = Some(pick);
                            source = "table";
                        }
                    }
                }
                if let Some(forced) = picks.get(side).filter(|s| !s.is_empty()) {
                    assert!(is_legal(forced), "illegal forced selection: {forced}");
                    chosen = Some(forced.clone());
                    source = "forced";
                }
            }

            if chosen.is_none() {
                let cached_policy = if first
                    && agent.belief().is_some_and(|b| !b.is_fallback())
                {
                    if let Some(cache) = &shared.preview_cache {
                        let own_json = shared
                            .team_json_map
                            .get(&task.ids[side])
                            .map(String::as_str)
                            .unwrap_or("");
                        let key = preview_cache_key(
                            side,
                            task.seeds[side],
                            shared.preview_iters,
                            shared.blind_c,
                            own_json,
                            &frame.lines,
                            &req_json,
                            &priors[side],
                            agent,
                            &shared.team_json_map,
                        );
                        let dex = &shared.dex;
                        let preview_iters = shared.preview_iters;
                        let is_current = policies[side] == "current";
                        Some(cache.get_or_compute(key, || {
                            if is_current {
                                agent
                                    .step_adaptive_battle(dex, preview_iters)
                                    .expect("step_adaptive_battle");
                            } else {
                                agent.step(dex, preview_iters).expect("step");
                            }
                            agent.decision_policy(dex, PolicyMode::Legacy)
                        }))
                    } else {
                        None
                    }
                } else {
                    None
                };

                let policy = match cached_policy {
                    Some(p) => p,
                    None => {
                        if first {
                            if policies[side] == "current" {
                                agent
                                    .step_adaptive_battle(&shared.dex, shared.preview_iters)
                                    .expect("step_adaptive_battle");
                            } else {
                                agent.step(&shared.dex, shared.preview_iters).expect("step");
                            }
                        } else if policies[side] == "current" {
                            match shared.battle_mode {
                                BattleMode::Adaptive => {
                                    agent
                                        .step_adaptive_battle(&shared.dex, shared.battle_iters)
                                        .expect("step_adaptive_battle");
                                }
                                BattleMode::Fixed => {
                                    agent.step(&shared.dex, shared.battle_iters).expect("step");
                                }
                            }
                        } else {
                            agent.step(&shared.dex, 81_000).expect("step");
                        }
                        agent.decision_policy(&shared.dex, PolicyMode::Legacy)
                    }
                };

                if decisions[side].committed().is_none() {
                    if let Some(policy) = policy {
                        if policy.source() == "request-projected" {
                            agent.projections += 1;
                        }
                        decisions[side].commit(policy);
                    }
                }
                let committed = decisions[side]
                    .committed()
                    .expect("expected committed decision");
                chosen = Some(committed.input.clone());
            }

            if first && task.mixtures.is_some() {
                if let Some(forced) = picks.get(side).filter(|s| !s.is_empty()) {
                    let norm = canonical_no_spaces(forced);
                    assert!(
                        frame
                            .legal_actions
                            .iter()
                            .any(|a| canonical_no_spaces(a) == norm),
                        "illegal forced selection: {forced}"
                    );
                    chosen = Some(forced.clone());
                    source = "forced";
                }
            }

            let input = chosen.expect("decision input");
            if first {
                preview[side] = Some(json!({"source": source, "input": input}));
            }
            inputs[side] = Some(input);
        }

        let mut line = serde_json::to_string(&inputs).unwrap();
        line.push('\n');
        transcript.update(line.as_bytes());

        for (side, input) in inputs.iter().enumerate() {
            if let Some(inp) = input {
                battle
                    .choose(&shared.dex, side, inp)
                    .unwrap_or_else(|e| panic!("battle.choose(side={side}, {inp:?}): {e:?}"));
            }
        }
        first = false;
    }

    let elapsed_ms = began.elapsed().as_secs_f64() * 1000.0;
    let outcome = match battle.outcome() {
        Some(Outcome::P1Win) => "p1",
        Some(Outcome::P2Win) => "p2",
        Some(Outcome::Tie) => "tie",
        None => "cap",
    };

    if let Some(log_dir) = &shared.log_dir {
        let log_path = log_dir.join(format!("{index}.json"));
        fs::write(log_path, serde_json::to_string(&battle.log).unwrap()).expect("write log");
    }

    let mut obj = serde_json::to_value(task).unwrap();
    let map = obj.as_object_mut().unwrap();
    map.insert("index".into(), json!(index));
    map.insert("preview".into(), json!(preview));
    map.insert("outcome".into(), json!(outcome));
    map.insert("turns".into(), json!(battle.turn));
    map.insert("actionSha256".into(), json!(format!("{:x}", transcript.finalize())));
    map.insert("ms".into(), json!(elapsed_ms.round() as u64));
    if task.mixtures.is_some() {
        map.insert("previewBeliefs".into(), json!(preview_beliefs));
    }
    obj
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 || args.len() > 4 {
        eprintln!("usage: {} GAMES.jsonl NEW_OUTPUT_DIR [workers]", args[0]);
        std::process::exit(2);
    }
    let task_file = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    let default_workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let workers: usize = args
        .get(3)
        .map(|s| s.parse().expect("invalid workers count"))
        .unwrap_or(default_workers);
    assert!(workers > 0, "workers must be > 0");

    let task_bytes = fs::read(&task_file)
        .unwrap_or_else(|e| panic!("read {}: {e}", task_file.display()));
    let source = String::from_utf8(task_bytes.clone()).expect("utf8 task file");
    let trimmed = source.trim();
    let tasks: Vec<Task> = if trimmed.starts_with('[') {
        serde_json::from_str(trimmed).expect("parse JSON array tasks")
    } else {
        trimmed
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("parse JSONL task"))
            .collect()
    };
    assert!(!tasks.is_empty(), "empty task schedule");

    // Refuse to mix builds or overwrite an earlier run (matches nash-refresh.mjs).
    fs::create_dir(&output).unwrap_or_else(|e| panic!("create {}: {e}", output.display()));
    let log_dir = if env::var("NASH_SAVE_LOGS").ok().as_deref() == Some("1") {
        let dir = output.join("logs");
        fs::create_dir(&dir).unwrap_or_else(|e| panic!("create {}: {e}", dir.display()));
        Some(dir)
    } else {
        None
    };

    let root = env::var("NC2000_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let manifest_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("workspace root")
                .to_path_buf();
            if manifest_root.join("data/live-search.json").exists() {
                manifest_root
            } else {
                env::current_dir().expect("cwd")
            }
        });
    let dex_bytes = fs::read(root.join("data/gen2stadium2.json")).expect("read data/gen2stadium2.json");
    let dex = Dex::from_json(std::str::from_utf8(&dex_bytes).unwrap()).expect("parse dex");
    let learnsets_text =
        fs::read_to_string(root.join("data/learnsets-gen2.json")).expect("read learnsets");
    let learnsets = Learnsets::from_json(&learnsets_text).expect("parse learnsets");

    let live: LiveSearchCfg = serde_json::from_str(&read_data(&root, "live-search")).unwrap();
    let profiles: SearchProfilesCfg =
        serde_json::from_str(&read_data(&root, "search-profiles")).unwrap();

    let mut teams: HashMap<String, Vec<PokemonSet>> = HashMap::new();
    let mut team_json_map: HashMap<String, String> = HashMap::new();
    for name in TEAM_FILES {
        let tf: TeamFile = serde_json::from_str(&read_data(&root, name)).unwrap();
        for t in tf.teams {
            team_json_map.insert(t.id.clone(), serde_json::to_string(&t.sets).unwrap());
            teams.insert(t.id, t.sets);
        }
    }

    let has_disclosure = tasks.iter().any(|t| t.mixtures.is_some());
    let needs_picker = tasks.iter().any(|t| t.mixtures.is_none());

    let mut disclosed: HashMap<String, MetaPool> = HashMap::new();
    if has_disclosure {
        let cand: TeamFile = serde_json::from_str(&read_data(&root, CANDIDATE_FILE)).unwrap();
        disclosed.insert(
            "candidate".into(),
            MetaPool {
                teams: cand
                    .teams
                    .into_iter()
                    .map(|t| MetaTeam {
                        id: t.id,
                        sets: t.sets,
                        weight: t.weight,
                    })
                    .collect(),
            },
        );
        let inc: TeamFile = serde_json::from_str(&read_data(&root, INCUMBENT_FILE)).unwrap();
        disclosed.insert(
            "incumbent".into(),
            MetaPool {
                teams: inc
                    .teams
                    .into_iter()
                    .map(|t| MetaTeam {
                        id: t.id,
                        sets: t.sets,
                        weight: t.weight,
                    })
                    .collect(),
            },
        );
        let deme_sets = teams["deme-r1-00-s0030"].clone();
        disclosed.insert(
            "deme".into(),
            MetaPool {
                teams: vec![MetaTeam {
                    id: "deme-r1-00-s0030".into(),
                    sets: deme_sets,
                    weight: Some(1.0),
                }],
            },
        );
    }

    let pools_text = env::var("NASH_POOLS")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|p| fs::read_to_string(&p).unwrap_or_else(|e| panic!("read NASH_POOLS {p}: {e}")));

    if let Some(ref extra_text) = pools_text {
        let extra: ExtraPoolsFile = serde_json::from_str(extra_text).expect("parse NASH_POOLS");
        for t in extra.teams {
            assert!(!teams.contains_key(&t.id), "extra team ID already exists: {}", t.id);
            let sets_json = serde_json::to_string(&t.sets).unwrap();
            let v = validate_team(&dex, &learnsets, &sets_json);
            assert!(v["ok"].as_bool() == Some(true), "invalid team {}: {v}", t.id);
            team_json_map.insert(t.id.clone(), sets_json);
            teams.insert(t.id, t.sets);
        }
        for (name, members) in extra.mixtures {
            assert!(!disclosed.contains_key(&name), "duplicate mixture: {name}");
            let mut seen = HashSet::new();
            let mut meta_teams = Vec::with_capacity(members.len());
            for m in members {
                assert!(seen.insert(m.id.clone()), "duplicate member {} in {name}", m.id);
                let sets = teams
                    .get(&m.id)
                    .unwrap_or_else(|| panic!("unknown team {} in mixture {name}", m.id))
                    .clone();
                meta_teams.push(MetaTeam {
                    id: m.id,
                    sets,
                    weight: Some(m.weight),
                });
            }
            disclosed.insert(name, MetaPool { teams: meta_teams });
        }
    }

    for (name, pool) in &disclosed {
        let sum: f64 = pool.teams.iter().map(|t| t.weight.unwrap_or(0.0)).sum();
        assert!(
            (sum - 1.0).abs() < 1e-10,
            "mixture {name} weights sum to {sum}"
        );
        for t in &pool.teams {
            assert!(t.weight.unwrap_or(0.0) > 0.0, "non-positive weight in {name}");
        }
    }

    // Validate every referenced team once up front (matches nash-refresh.mjs).
    let mut validated = HashSet::new();
    for t in &tasks {
        for id in &t.ids {
            if validated.insert(id.clone()) {
                let sets_json = team_json_map
                    .get(id)
                    .unwrap_or_else(|| panic!("unknown team id: {id}"));
                let v = validate_team(&dex, &learnsets, sets_json);
                assert!(v["ok"].as_bool() == Some(true), "invalid {id}: {v}");
            }
        }
    }

    let (default_pool, picker) = if needs_picker {
        let pool_bytes = fs::read(root.join("data/belief-pool-v3/belief-pool.json"))
            .expect("read data/belief-pool-v3/belief-pool.json");
        let pool: MetaPool = serde_json::from_slice(&pool_bytes).expect("parse belief-pool");
        let picker = CompiledPreviewPicker::load(&root, &dex_bytes, &pool_bytes);
        (Some(pool), Some(picker))
    } else {
        (None, None)
    };

    let mut inputs: Vec<&str> = TEAM_FILES.to_vec();
    inputs.push("live-search");
    inputs.push("search-profiles");
    if needs_picker {
        inputs.push("compiled-preview-v1");
        inputs.push("belief-pool-v3/belief-pool");
    }
    if has_disclosure {
        inputs.push(CANDIDATE_FILE);
    }
    let mut data_hashes = serde_json::Map::new();
    for k in inputs {
        let text = read_data(&root, k);
        data_hashes.insert(k.to_string(), json!(sha256_hex(text.as_bytes())));
    }
    let exe_bytes = env::current_exe()
        .ok()
        .and_then(|p| fs::read(p).ok())
        .unwrap_or_default();
    let preview_iters: u32 = env::var("NASH_PREVIEW_ITERS")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().expect("invalid NASH_PREVIEW_ITERS"))
        .unwrap_or(live.unknown_preview);
    let battle_iters: u32 = env::var("NASH_BATTLE_ITERS")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().expect("invalid NASH_BATTLE_ITERS"))
        .unwrap_or(live.battle.adaptive);
    let battle_mode = match env::var("NASH_BATTLE_MODE").ok().as_deref() {
        None | Some("") | Some("adaptive") => BattleMode::Adaptive,
        Some("fixed") => BattleMode::Fixed,
        Some(other) => panic!("invalid NASH_BATTLE_MODE: {other} (expected adaptive or fixed)"),
    };
    let preview_cache_enabled = match env::var("NASH_PREVIEW_CACHE").ok().as_deref() {
        None | Some("") | Some("1") | Some("true") => true,
        Some("0") | Some("false") => false,
        Some(other) => panic!("invalid NASH_PREVIEW_CACHE: {other}"),
    };
    let is_custom_budget = preview_iters != live.unknown_preview
        || battle_iters != live.battle.adaptive
        || battle_mode != BattleMode::Adaptive;

    let mut manifest = serde_json::Map::new();
    if let Some(ref pt) = pools_text {
        manifest.insert("poolsSha256".into(), json!(sha256_hex(pt.as_bytes())));
    }
    manifest.insert("tasks".into(), json!(sha256_hex(&task_bytes)));
    manifest.insert("runner".into(), json!("native-rust"));
    manifest.insert("nativeBinary".into(), json!(sha256_hex(&exe_bytes)));
    if is_custom_budget {
        manifest.insert(
            "budget".into(),
            json!({
                "previewIters": preview_iters,
                "battleIters": battle_iters,
                "battleMode": battle_mode.as_str(),
            }),
        );
    }
    manifest.insert("previewCache".into(), json!(preview_cache_enabled));
    manifest.insert("data".into(), Value::Object(data_hashes));
    fs::write(
        output.join("manifest.json"),
        serde_json::to_string(&Value::Object(manifest)).unwrap(),
    )
    .expect("write manifest.json");

    let shared = Arc::new(SharedState {
        dex,
        teams,
        team_json_map,
        disclosed,
        default_pool,
        picker,
        preview_iters,
        battle_iters,
        battle_mode,
        blind_c: profiles.blind.c,
        log_dir,
        preview_cache: preview_cache_enabled.then(PreviewCache::new),
    });

    let wall_start = Instant::now();
    let next = AtomicUsize::new(0);
    let n_workers = workers.min(tasks.len());
    std::thread::scope(|s| {
        for _ in 0..n_workers {
            let shared = &shared;
            let tasks = &tasks;
            let next = &next;
            let output = &output;
            s.spawn(move || loop {
                let idx = next.fetch_add(1, Ordering::Relaxed);
                if idx >= tasks.len() {
                    break;
                }
                let result = run_game(shared, idx, &tasks[idx]);
                fs::write(
                    output.join(format!("{idx}.json")),
                    serde_json::to_string(&result).unwrap(),
                )
                .expect("write game result");
            });
        }
    });

    let (cache_hits, cache_misses) = shared
        .preview_cache
        .as_ref()
        .map(|c| {
            (
                c.hits.load(Ordering::Relaxed),
                c.misses.load(Ordering::Relaxed),
            )
        })
        .unwrap_or((0, 0));

    println!(
        "{}",
        json!({
            "games": tasks.len(),
            "output": output.display().to_string(),
            "workers": n_workers,
            "wall_ms": (wall_start.elapsed().as_secs_f64() * 1000.0).round() as u64,
            "preview_cache_hits": cache_hits,
            "preview_cache_misses": cache_misses,
        })
    );
}
