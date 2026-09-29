//! Team-pool rebuild harness (docs/TEAM-POOL-REBUILD-PLAN.md): seed-paired
//! team-vs-team cells under one declared agent condition, one JSONL per
//! cell, resumable, with per-game selection and move-use records for the
//! execution audit.
//!
//!   cargo run --release -p nc2000-bot --example team_eval -- \
//!       --teams FILE [--teams FILE ...]        {teams:[{id, sets}]} files
//!       (--round-robin IDS_FILE | --cells CELLS_FILE)
//!       --agent open:300 [--agent-col SPEC]   row / column agent
//!       [--belief-pool FILE]                  blind agents' candidate pool
//!       [--belief-pool-col FILE]              the column agent's, when it differs
//!       --seeds N --seed-base S --out DIR [--threads T] [--max-turns 500]
//!       [--force-row-picks ROW_ID=SpeciesA,SpeciesB,SpeciesC]
//!       [--dump-logs DIR]                     full protocol log per game
//!
//! IDS_FILE: one team id per line (every unordered pair is a cell, row =
//! the lexicographically smaller id). CELLS_FILE: one `row col` pair per
//! line (directed: row is the team being evaluated).
//!
//! Agent specs: `open:ITERS:C` (sets public, picks hidden),
//! `blind:ITERS:C` (public info + belief pool), `skuct:ITERS:C` (true
//! state). The shipped bot is `blind:27000:0.4` (`data/search-profiles.json`);
//! no spec ponders. A spec without `:C` means `RmConfig::default()`'s 1.0 and
//! is refused unless `--allow-default-c` is given, which exists only to
//! reproduce runs recorded before `:C` was written.
//!
//! Every record carries `cond`: the bot build fingerprint (engine + bot
//! sources), each side's belief-pool file fingerprint, the turn cap, the
//! seed base, and the fixed preview/ponder behavior. Resume refuses a
//! directory holding a record written under any other condition.
//!
//! Pairing: game k of every cell uses the same battle seed and the same
//! agent seeds, derived from (--seed-base, k, side) only; each k is played
//! twice with the row team on p1 and then p2. Tasks run k-major, so a
//! partially finished run has every cell at nearly the same n.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use conformance::load_dex;
use nc2000_bot::preview::{MetaPool, MetaTeam};
use nc2000_bot::smmcts::SelRule;
use nc2000_bot::{Agent, BlindAgent, GameResult, OpenAgent, RmAgent, RmConfig, SplitMix64};
use nc2000_engine::battle::{Outcome, PokemonSet, SearchChoice};
use nc2000_engine::dex::Dex;
use nc2000_engine::state::Battle;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Open,
    Blind,
    Skuct,
}

#[derive(Clone, Debug)]
struct Spec {
    kind: Kind,
    iters: u32,
    /// UCB exploration constant; `None` = `RmConfig::default()` (1.0).
    c: Option<f64>,
}

impl Spec {
    fn parse(s: &str, allow_default_c: bool) -> Spec {
        let mut parts = s.split(':');
        let kind = match parts.next().unwrap() {
            "open" => Kind::Open,
            "blind" => Kind::Blind,
            "skuct" => Kind::Skuct,
            _ => panic!("unknown agent spec {s}"),
        };
        let iters = parts.next().unwrap_or("300").parse().unwrap_or_else(|_| panic!("bad iterations in {s}"));
        let c = parts.next().map(|c| c.parse().unwrap_or_else(|_| panic!("bad c in {s}")));
        assert!(
            c.is_some() || allow_default_c,
            "agent spec {s} has no exploration constant; write it as KIND:ITERS:C"
        );
        Spec { kind, iters, c }
    }

    /// Stored in every game record; resume refuses a directory written
    /// under another label. A spec without `:C` keeps its historical label.
    fn label(&self) -> String {
        let kind = match self.kind {
            Kind::Open => "open",
            Kind::Blind => "blind",
            Kind::Skuct => "skuct",
        };
        match self.c {
            Some(c) => format!("{kind}:{}:{c}", self.iters),
            None => format!("{kind}:{}", self.iters),
        }
    }

    fn build(&self, seed: u64, pool: Option<&Arc<MetaPool>>) -> Box<dyn Agent> {
        let base = RmConfig { iterations: self.iters, rule: SelRule::Ucb, ..Default::default() };
        let cfg = RmConfig { c: self.c.unwrap_or(base.c), ..base };
        match self.kind {
            Kind::Open => Box::new(OpenAgent::new(cfg, None, seed)),
            Kind::Blind => Box::new(BlindAgent::new(
                cfg,
                pool.expect("blind agents need --belief-pool").clone(),
                None,
                seed,
            )),
            Kind::Skuct => Box::new(RmAgent::new(cfg, seed)),
        }
    }

    fn needs_log(&self) -> bool {
        self.kind != Kind::Skuct
    }
}

/// Wraps an agent and forces its team-preview answer.
struct ForcedPicks {
    inner: Box<dyn Agent>,
    picks: [u8; 3],
}

impl Agent for ForcedPicks {
    fn name(&self) -> String {
        format!("forced{:?}:{}", self.picks, self.inner.name())
    }

    fn choose(
        &mut self,
        battle: &Battle,
        dex: &Dex,
        side: usize,
        choices: &[SearchChoice],
    ) -> SearchChoice {
        let forced = SearchChoice::Team(self.picks);
        // Let the inner agent see the preview (it builds its per-game state
        // there), then override the answer.
        let own = self.inner.choose(battle, dex, side, choices);
        if matches!(choices[0], SearchChoice::Team(_)) && choices.contains(&forced) {
            forced
        } else {
            own
        }
    }
}

/// Remembers the team-preview answer (the selection, lead first), which the
/// protocol log only reveals for mons that actually come out.
struct RecordPicks {
    inner: Box<dyn Agent>,
    picks: Option<[u8; 3]>,
}

impl Agent for RecordPicks {
    fn name(&self) -> String {
        self.inner.name()
    }

    fn choose(
        &mut self,
        battle: &Battle,
        dex: &Dex,
        side: usize,
        choices: &[SearchChoice],
    ) -> SearchChoice {
        let c = self.inner.choose(battle, dex, side, choices);
        if let SearchChoice::Team(p) = c {
            self.picks = Some(p);
        }
        c
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn flags(args: &[String], name: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, a)| *a == name)
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect()
}

fn load_teams(paths: &[String]) -> HashMap<String, Vec<PokemonSet>> {
    let mut out = HashMap::new();
    for p in paths {
        let text = std::fs::read_to_string(p).unwrap_or_else(|e| panic!("read {p}: {e}"));
        let pool: MetaPool = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {p}: {e}"));
        for MetaTeam { id, sets, .. } in pool.teams {
            if out.insert(id.clone(), sets).is_some() {
                panic!("team id {id} defined twice");
            }
        }
    }
    out
}

fn read_lines(path: &str) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {path}: {e}"))
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

fn file_fingerprint(path: &str) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3));
    format!("fnv1a64:{hash:016x}")
}

fn cell_file(out: &Path, row: &str, col: &str) -> PathBuf {
    out.join(format!("cell-{row}__{col}.jsonl"))
}

/// Per-side facts recovered from the protocol log: the picked species in
/// order of first appearance (lead first) and move uses per species.
fn log_summary(battle: &Battle, side: usize) -> (Vec<String>, BTreeMap<String, BTreeMap<String, u32>>) {
    let tag = if side == 0 { "p1a: " } else { "p2a: " };
    let mut appeared: Vec<String> = Vec::new();
    let mut moves: BTreeMap<String, BTreeMap<String, u32>> = BTreeMap::new();
    for line in &battle.log {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 4 {
            continue;
        }
        match parts[1] {
            "switch" | "drag" if parts[2].starts_with(tag) => {
                let species = parts[3].split(',').next().unwrap_or("").to_string();
                if !appeared.contains(&species) {
                    appeared.push(species);
                }
            }
            "move" if parts[2].starts_with(tag) => {
                let who = parts[2][tag.len()..].to_string();
                *moves.entry(who).or_default().entry(parts[3].to_string()).or_default() += 1;
            }
            _ => {}
        }
    }
    (appeared, moves)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dex = load_dex();
    let teams = load_teams(&flags(&args, "--teams"));
    let allow_default_c = args.iter().any(|a| a == "--allow-default-c");
    let spec_row = Spec::parse(&flag(&args, "--agent").expect("--agent KIND:ITERS:C"), allow_default_c);
    let spec_col = flag(&args, "--agent-col").map(|s| Spec::parse(&s, allow_default_c)).unwrap_or(spec_row.clone());
    let belief_pool: Option<Arc<MetaPool>> = flag(&args, "--belief-pool").map(|p| {
        Arc::new(
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}")))
                .unwrap_or_else(|e| panic!("parse {p}: {e}")),
        )
    });
    let belief_pool_col: Option<Arc<MetaPool>> = flag(&args, "--belief-pool-col")
        .map(|p| {
            Arc::new(
                serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}")))
                    .unwrap_or_else(|e| panic!("parse {p}: {e}")),
            )
        })
        .or_else(|| belief_pool.clone());
    let seeds: usize = flag(&args, "--seeds").map(|s| s.parse().unwrap()).unwrap_or(16);
    let seed_base: u64 = flag(&args, "--seed-base").map(|s| s.parse().unwrap()).unwrap_or(1);
    let max_turns: u16 = flag(&args, "--max-turns").map(|s| s.parse().unwrap()).unwrap_or(500);
    let threads: usize = flag(&args, "--threads").map(|s| s.parse().unwrap()).unwrap_or(4);
    let pool_row_fp = flag(&args, "--belief-pool").map(|p| file_fingerprint(&p));
    let pool_col_fp = flag(&args, "--belief-pool-col").map(|p| file_fingerprint(&p)).or_else(|| pool_row_fp.clone());
    let cond = json!({
        "bot": nc2000_bot::m17e_artifact::solver_build_fingerprint(),
        "belief_row": if spec_row.kind == Kind::Blind { pool_row_fp.clone() } else { None },
        "belief_col": if spec_col.kind == Kind::Blind { pool_col_fp.clone() } else { None },
        "max_turns": max_turns,
        "seed_base": seed_base,
        "preview": "live search, no baked tables",
        "ponder": "none",
    });
    let out = PathBuf::from(flag(&args, "--out").expect("--out DIR"));
    std::fs::create_dir_all(&out).unwrap();
    let dump_logs = flag(&args, "--dump-logs").map(PathBuf::from);
    if let Some(d) = &dump_logs {
        std::fs::create_dir_all(d).unwrap();
    }
    let forced: HashMap<String, Vec<String>> = flags(&args, "--force-row-picks")
        .iter()
        .map(|s| {
            let (id, sp) = s.split_once('=').expect("--force-row-picks ID=A,B,C");
            (id.to_string(), sp.split(',').map(str::to_string).collect())
        })
        .collect();

    let mut cells: Vec<(String, String)> = Vec::new();
    if let Some(p) = flag(&args, "--round-robin") {
        let mut ids = read_lines(&p);
        ids.sort();
        ids.dedup();
        for i in 0..ids.len() {
            for j in i + 1..ids.len() {
                cells.push((ids[i].clone(), ids[j].clone()));
            }
        }
    }
    if let Some(p) = flag(&args, "--cells") {
        for l in read_lines(&p) {
            let mut it = l.split_whitespace();
            let (a, b) = (it.next().unwrap(), it.next().expect("cells line: ROW COL"));
            cells.push((a.to_string(), b.to_string()));
        }
    }
    assert!(!cells.is_empty(), "no cells (--round-robin or --cells)");
    for (a, b) in &cells {
        for id in [a, b] {
            assert!(teams.contains_key(id), "unknown team id {id}");
        }
        assert_ne!(a, b, "a team cannot face itself in a cell");
    }

    // Resume: (cell, k, row_is_p1) already written.
    let mut done: HashSet<(usize, usize, bool)> = HashSet::new();
    for (ci, (a, b)) in cells.iter().enumerate() {
        if let Ok(text) = std::fs::read_to_string(cell_file(&out, a, b)) {
            for v in text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
                if v["agent"].as_str() != Some(&format!("{}|{}", spec_row.label(), spec_col.label())) {
                    panic!("{}: written under a different agent condition", cell_file(&out, a, b).display());
                }
                if v["cond"] != cond || v["forced"] != json!(forced.get(a)) {
                    panic!(
                        "{}: written under a different condition ({} vs {})",
                        cell_file(&out, a, b).display(),
                        v["cond"],
                        cond
                    );
                }
                done.insert((ci, v["k"].as_u64().unwrap() as usize, v["row_p1"].as_bool().unwrap()));
            }
        }
    }
    let mut tasks: Vec<(usize, usize, bool)> = Vec::new();
    for k in 0..seeds {
        for ci in 0..cells.len() {
            for row_p1 in [true, false] {
                if !done.contains(&(ci, k, row_p1)) {
                    tasks.push((ci, k, row_p1));
                }
            }
        }
    }
    eprintln!(
        "team_eval: {} cells x {} seeds x 2 sides, {} to play ({} done), row {} col {}, {} threads, cond {}",
        cells.len(),
        seeds,
        tasks.len(),
        done.len(),
        spec_row.label(),
        spec_col.label(),
        threads,
        cond
    );

    let files: Mutex<HashMap<usize, std::fs::File>> = Mutex::new(HashMap::new());
    let cursor = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let t0 = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = cursor.fetch_add(1, Ordering::Relaxed);
                if i >= tasks.len() {
                    break;
                }
                let (ci, k, row_p1) = tasks[i];
                let (row, col) = &cells[ci];
                let mut r = SplitMix64::new(seed_base ^ (k as u64 + 1).wrapping_mul(0xD1B5_4A32_D192_ED03));
                let bseed = r.battle_seed();
                let seed_p1 = r.next();
                let seed_p2 = r.next();
                let (t1, t2) = if row_p1 { (&teams[row], &teams[col]) } else { (&teams[col], &teams[row]) };
                let mut battle = Battle::from_fixture(&dex, &bseed, t1, t2).unwrap();
                battle.set_log_enabled(true);
                let (spec1, spec2) = if row_p1 { (&spec_row, &spec_col) } else { (&spec_col, &spec_row) };
                let _ = (spec1.needs_log(), spec2.needs_log());
                let (pool1, pool2) = if row_p1 {
                    (belief_pool.as_ref(), belief_pool_col.as_ref())
                } else {
                    (belief_pool_col.as_ref(), belief_pool.as_ref())
                };
                let mut a1 = spec1.build(seed_p1, pool1);
                let mut a2 = spec2.build(seed_p2, pool2);
                if let Some(sp) = forced.get(row) {
                    let sets = &teams[row];
                    let mut picks = [0u8; 3];
                    for (n, name) in sp.iter().enumerate() {
                        let pos = sets.iter().position(|s| &s.species == name)
                            .unwrap_or_else(|| panic!("{row} has no {name}"));
                        picks[n] = pos as u8 + 1;
                    }
                    let wrap = |inner| Box::new(ForcedPicks { inner, picks }) as Box<dyn Agent>;
                    if row_p1 {
                        a1 = wrap(a1);
                    } else {
                        a2 = wrap(a2);
                    }
                }
                let mut r1 = RecordPicks { inner: a1, picks: None };
                let mut r2 = RecordPicks { inner: a2, picks: None };
                let res = {
                    let mut pair: [&mut dyn Agent; 2] = [&mut r1, &mut r2];
                    nc2000_bot::play_game(&dex, &mut battle, &mut pair, max_turns).unwrap()
                };
                let selection = |r: &RecordPicks, sets: &[PokemonSet]| -> Vec<String> {
                    r.picks
                        .map(|p| p.iter().filter(|&&x| x > 0).map(|&x| sets[x as usize - 1].species.clone()).collect())
                        .unwrap_or_default()
                };
                let (sel1, sel2) = (selection(&r1, t1), selection(&r2, t2));
                let (row_sel, col_sel) = if row_p1 { (sel1, sel2) } else { (sel2, sel1) };
                let p1_score = match res {
                    GameResult::Outcome(Outcome::P1Win) => 1.0,
                    GameResult::Outcome(Outcome::P2Win) => 0.0,
                    _ => 0.5,
                };
                let (row_side, col_side) = if row_p1 { (0, 1) } else { (1, 0) };
                let (row_picks, row_moves) = log_summary(&battle, row_side);
                let (col_picks, col_moves) = log_summary(&battle, col_side);
                let rec = json!({
                    "k": k, "row_p1": row_p1, "seed": bseed,
                    "agent": format!("{}|{}", spec_row.label(), spec_col.label()),
                    "cond": cond, "forced": forced.get(row),
                    "score": if row_p1 { p1_score } else { 1.0 - p1_score },
                    "result": match res {
                        GameResult::Outcome(Outcome::P1Win) => "p1",
                        GameResult::Outcome(Outcome::P2Win) => "p2",
                        GameResult::Outcome(Outcome::Tie) => "tie",
                        GameResult::TurnCapped => "cap",
                    },
                    "turns": battle.turn,
                    "row_picks": row_picks, "col_picks": col_picks,
                    "row_selected": row_sel, "col_selected": col_sel,
                    "row_moves": row_moves, "col_moves": col_moves,
                    "row_left": battle.sides[row_side].pokemon_left,
                    "col_left": battle.sides[col_side].pokemon_left,
                });
                if let Some(d) = &dump_logs {
                    let name = format!("{row}__{col}-k{k}-{}.log", if row_p1 { "p1" } else { "p2" });
                    std::fs::write(d.join(name), battle.log.join("\n")).unwrap();
                }
                {
                    let mut f = files.lock().unwrap();
                    let file = f.entry(ci).or_insert_with(|| {
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(cell_file(&out, row, col))
                            .unwrap()
                    });
                    writeln!(file, "{rec}").unwrap();
                }
                let n = finished.fetch_add(1, Ordering::Relaxed) + 1;
                if n % 500 == 0 || n == tasks.len() {
                    let el = t0.elapsed().as_secs_f64();
                    eprintln!(
                        "  {n}/{} games, {:.0}s elapsed, {:.1} games/s, eta {:.0}s",
                        tasks.len(),
                        el,
                        n as f64 / el,
                        (tasks.len() - n) as f64 / (n as f64 / el)
                    );
                }
            });
        }
    });
}
