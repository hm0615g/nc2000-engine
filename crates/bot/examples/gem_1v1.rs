//! Fast 1v1 & Adversarial Switch-In evaluator for the Minor-Pokemon Gem-Finder.
//!
//! Evaluates `(candidate, anchor)` pairs using exact `nc2000-engine` mechanics
//! in two modes:
//!   - `--mode root`: Fast Turn-1 `SkuctSearch` root evaluation (late-half on-policy mean).
//!   - `--mode play`: Full played-out 1v1 games using `RmAgent` (captures PP/Curse/stall).
//!
//! Both modes compute:
//!   - `lead`: direct 1v1 win rate for candidate (P1) vs anchor (P2)
//!   - `sw_all`: adversarial switch-in win rate (min over all relevant anchor Turn-1 moves)
//!   - `sw_no_sleep`: adversarial switch-in win rate excluding Turn-1 sleep-inducing moves
//!   - `worst_move` / `worst_move_nosleep`: anchor move realizing each minimum
//!   - `cap_rate`: fraction of games hitting `max_turns` (play mode)

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use conformance::load_dex;
use nc2000_bot::smmcts::{SelRule, SkuctSearch};
use nc2000_bot::{play_game, Agent, GameResult, RmAgent, RmConfig, SplitMix64};
use nc2000_engine::battle::{Outcome, PokemonSet, SearchChoice};
use nc2000_engine::dex::{toid, Dex, MoveId};
use nc2000_engine::state::{Battle, Status};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize)]
struct InputDoc {
    anchors: Vec<AnchorEntry>,
    candidates: Vec<CandEntry>,
    #[serde(default)]
    cells: Option<Vec<[usize; 2]>>,
}

#[derive(Debug, Deserialize, Clone)]
struct AnchorEntry {
    id: String,
    set: PokemonSet,
    #[serde(default)]
    switch_moves: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Clone)]
struct CandEntry {
    id: String,
    species: String,
    tier: String,
    is_major: bool,
    set: PokemonSet,
}

#[derive(Debug, Serialize)]
struct CellOutput {
    cand_idx: usize,
    anchor_idx: usize,
    cand_id: String,
    anchor_id: String,
    lead: f64,
    sw_all: f64,
    sw_no_sleep: f64,
    worst_move: String,
    worst_move_nosleep: String,
    cap_rate: f64,
    avg_turns: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Root,
    Play,
}

struct Cli {
    input: PathBuf,
    out: PathBuf,
    mode: Mode,
    iters: u32,
    sw_iters: u32,
    games: usize,
    sw_samples: usize,
    max_turns: u16,
    threads: usize,
    seed: u64,
}

fn parse_cli() -> Cli {
    let mut args = std::env::args().skip(1);
    let mut input = None;
    let mut out = None;
    let mut mode = Mode::Root;
    let mut iters = 600u32;
    let mut sw_iters = 160u32;
    let mut games = 16usize;
    let mut sw_samples = 6usize;
    let mut max_turns = 150u16;
    let mut threads = 10usize;
    let mut seed = 20261122u64;

    while let Some(a) = args.next() {
        match a.as_str() {
            "--input" => input = Some(PathBuf::from(args.next().expect("--input value"))),
            "--out" => out = Some(PathBuf::from(args.next().expect("--out value"))),
            "--mode" => {
                mode = match args.next().expect("--mode value").as_str() {
                    "root" => Mode::Root,
                    "play" => Mode::Play,
                    other => panic!("unknown mode {other}"),
                };
            }
            "--iters" => iters = args.next().unwrap().parse().unwrap(),
            "--sw-iters" => sw_iters = args.next().unwrap().parse().unwrap(),
            "--games" => games = args.next().unwrap().parse().unwrap(),
            "--sw-samples" => sw_samples = args.next().unwrap().parse().unwrap(),
            "--max-turns" => max_turns = args.next().unwrap().parse().unwrap(),
            "--threads" => threads = args.next().unwrap().parse().unwrap(),
            "--seed" => seed = args.next().unwrap().parse().unwrap(),
            other => panic!("unknown arg {other}"),
        }
    }
    Cli {
        input: input.expect("--input required"),
        out: out.expect("--out required"),
        mode,
        iters,
        sw_iters,
        games,
        sw_samples,
        max_turns,
        threads: threads.max(1),
        seed,
    }
}

fn is_skipped_switch_move(key: &str) -> bool {
    matches!(
        key,
        "explosion"
            | "selfdestruct"
            | "roar"
            | "whirlwind"
            | "rest"
            | "sleeptalk"
            | "recover"
            | "milkdrink"
            | "softboiled"
            | "synthesis"
            | "moonlight"
            | "morningsun"
            | "spikes"
            | "batonpass"
            | "protect"
            | "detect"
            | "endure"
            | "meanlook"
            | "spiderweb"
            | "pursuit"
            | "destinybond"
            | "snore"
    )
}

fn is_sleep_move(key: &str) -> bool {
    matches!(
        key,
        "sleeppowder" | "spore" | "lovelykiss" | "hypnosis" | "sing"
    )
}

fn relevant_switch_moves(dex: &Dex, anchor: &AnchorEntry) -> Vec<(MoveId, String, bool)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let src: Vec<String> = match &anchor.switch_moves {
        Some(ms) if !ms.is_empty() => ms.clone(),
        _ => anchor.set.moves.clone(),
    };
    for m in &src {
        let key = toid(m);
        if is_skipped_switch_move(&key) || !seen.insert(key.clone()) {
            continue;
        }
        if let Some(mid) = dex.moves.id(&key) {
            let sleep = is_sleep_move(&key);
            out.push((mid, key, sleep));
        }
    }
    if out.is_empty() {
        for m in &anchor.set.moves {
            let key = toid(m);
            if let Some(mid) = dex.moves.id(&key) {
                let sleep = is_sleep_move(&key);
                out.push((mid, key, sleep));
                break;
            }
        }
    }
    out
}

fn dummy_sac_for(cand: &PokemonSet) -> PokemonSet {
    let sp = if toid(&cand.species) == "magikarp" {
        "Caterpie"
    } else {
        "Magikarp"
    };
    PokemonSet {
        name: sp.to_string(),
        species: sp.to_string(),
        item: String::new(),
        ability: "No Ability".to_string(),
        moves: vec!["Splash".to_string()],
        level: 50,
        evs: None,
        ivs: None,
        happiness: Some(255),
        gender: Some("M".to_string()),
    }
}

fn start_1v1_battle(dex: &Dex, seed: &str, p1: &PokemonSet, p2: &PokemonSet) -> Battle {
    let mut b = Battle::from_fixture(dex, seed, std::slice::from_ref(p1), std::slice::from_ref(p2))
        .expect("1v1 from_fixture");
    b.set_log_enabled(false);
    let c0 = b.legal_choices(dex, 0)[0];
    let c1 = b.legal_choices(dex, 1)[0];
    b.apply_choices(dex, [Some(c0), Some(c1)]).expect("1v1 preview");
    b
}

/// Starts a switch-in battle: P1 switches `cand` in on Turn 1 while P2 executes `anchor_move`.
/// Returns `Ok(Battle)` ready for Turn 2 pure 1v1, or `Err(terminal_score_for_p1)`.
fn start_switchin_battle(
    dex: &Dex,
    seed: &str,
    cand: &PokemonSet,
    anchor: &PokemonSet,
    anchor_move: MoveId,
) -> Result<Battle, f64> {
    let sac = dummy_sac_for(cand);
    let p1_team = [sac, cand.clone()];
    let mut b = Battle::from_fixture(dex, seed, &p1_team, std::slice::from_ref(anchor))
        .expect("switchin from_fixture");
    b.set_log_enabled(false);
    // Team preview: P1 picks [1, 2] (sac lead, cand bench), P2 picks [1].
    b.apply_choices(
        dex,
        [Some(SearchChoice::Team([1, 2, 0])), Some(SearchChoice::Team([1, 0, 0]))],
    )
    .expect("switchin preview");

    // Turn 1: P1 switches to slot 2 (cand), P2 executes `anchor_move`.
    b.apply_choices(
        dex,
        [Some(SearchChoice::Switch(2)), Some(SearchChoice::Move(anchor_move))],
    )
    .expect("switchin turn1");

    // Check if battle ended (e.g., anchor self-KO'd or cand fainted).
    if let Some(out) = b.outcome() {
        return Err(match out {
            Outcome::P1Win => 1.0,
            Outcome::P2Win => 0.0,
            Outcome::Tie => 0.5,
        });
    }
    // If cand fainted on Turn 1 switch-in, P1 lost the 1v1 immediately.
    if b.sides[0].roster[1].fainted || b.sides[0].roster[1].hp <= 0 {
        return Err(0.0);
    }
    // If anchor fainted on Turn 1 (e.g. recoil) while cand survived, P1 won.
    if b.sides[1].roster[0].fainted || b.sides[1].roster[0].hp <= 0 {
        return Err(1.0);
    }

    // Kill sac (roster slot 0) so from Turn 2 onward it is strictly 1v1 (no legal switches).
    b.sides[0].roster[0].hp = 0;
    b.sides[0].roster[0].fainted = true;
    b.sides[0].roster[0].status = Status::Fnt;
    b.sides[0].pokemon_left = 1;
    b.sides[0].total_fainted = 1;
    Ok(b)
}

fn eval_root_value(dex: &Dex, battle: &Battle, iters: u32, seed: u64) -> f64 {
    if let Some(o) = battle.outcome() {
        return match o {
            Outcome::P1Win => 1.0,
            Outcome::P2Win => 0.0,
            Outcome::Tie => 0.5,
        };
    }
    let cfg = RmConfig {
        iterations: iters,
        c: 0.4,
        rule: SelRule::Ucb,
        ..Default::default()
    };
    let mut search = SkuctSearch::new(battle, dex, cfg, seed);
    let warmup = iters / 2;
    let mut sum = 0.0f64;
    let mut cnt = 0u32;
    for i in 0..iters {
        let (r0, _) = search.step_one(dex);
        if i >= warmup {
            sum += r0;
            cnt += 1;
        }
    }
    if cnt == 0 {
        0.5
    } else {
        sum / cnt as f64
    }
}

fn play_1v1_game(
    dex: &Dex,
    mut battle: Battle,
    iters: u32,
    max_turns: u16,
    seed0: u64,
    seed1: u64,
) -> (f64, bool, u16) {
    let cfg = RmConfig {
        iterations: iters,
        c: 0.4,
        rule: SelRule::Rm,
        ..Default::default()
    };
    let mut a0 = RmAgent::new(cfg.clone(), seed0);
    let mut a1 = RmAgent::new(cfg, seed1);
    let mut agents: [&mut dyn Agent; 2] = [&mut a0, &mut a1];
    let res = play_game(dex, &mut battle, &mut agents, max_turns).expect("play_1v1_game");
    let turns = battle.turn;
    match res {
        GameResult::Outcome(Outcome::P1Win) => (1.0, false, turns),
        GameResult::Outcome(Outcome::P2Win) => (0.0, false, turns),
        GameResult::Outcome(Outcome::Tie) => (0.5, false, turns),
        GameResult::TurnCapped => (0.5, true, turns),
    }
}

fn eval_cell(
    dex: &Dex,
    cand: &CandEntry,
    anchor: &AnchorEntry,
    cand_idx: usize,
    anchor_idx: usize,
    sw_moves: &[(MoveId, String, bool)],
    cli: &Cli,
) -> CellOutput {
    let cell_seed = cli
        .seed
        .wrapping_add((cand_idx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add((anchor_idx as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    let mut rng = SplitMix64::new(cell_seed);

    let mut total_games = 0usize;
    let mut cap_games = 0usize;
    let mut turn_sum = 0u64;

    let lead = match cli.mode {
        Mode::Root => {
            let b = start_1v1_battle(dex, &rng.battle_seed(), &cand.set, &anchor.set);
            eval_root_value(dex, &b, cli.iters, rng.next())
        }
        Mode::Play => {
            let mut sum = 0.0;
            for _ in 0..cli.games {
                let b = start_1v1_battle(dex, &rng.battle_seed(), &cand.set, &anchor.set);
                let (sc, capped, t) =
                    play_1v1_game(dex, b, cli.iters, cli.max_turns, rng.next(), rng.next());
                sum += sc;
                total_games += 1;
                if capped {
                    cap_games += 1;
                }
                turn_sum += t as u64;
            }
            sum / cli.games as f64
        }
    };

    let mut sw_all = 1.0f64;
    let mut worst_move = String::new();
    let mut sw_no_sleep = 1.0f64;
    let mut worst_move_nosleep = String::new();
    let mut any_nosleep = false;

    for (mid, mkey, is_sleep) in sw_moves {
        let score = match cli.mode {
            Mode::Root => {
                let mut sum = 0.0;
                for _ in 0..cli.sw_samples {
                    let bseed = rng.battle_seed();
                    let sc = match start_switchin_battle(dex, &bseed, &cand.set, &anchor.set, *mid)
                    {
                        Ok(b) => eval_root_value(dex, &b, cli.sw_iters, rng.next()),
                        Err(term) => term,
                    };
                    sum += sc;
                }
                sum / cli.sw_samples as f64
            }
            Mode::Play => {
                let mut sum = 0.0;
                for _ in 0..cli.games {
                    let bseed = rng.battle_seed();
                    let (sc, capped, t) =
                        match start_switchin_battle(dex, &bseed, &cand.set, &anchor.set, *mid) {
                            Ok(b) => play_1v1_game(
                                dex,
                                b,
                                cli.sw_iters,
                                cli.max_turns,
                                rng.next(),
                                rng.next(),
                            ),
                            Err(term) => (term, false, 1),
                        };
                    sum += sc;
                    total_games += 1;
                    if capped {
                        cap_games += 1;
                    }
                    turn_sum += t as u64;
                }
                sum / cli.games as f64
            }
        };

        if score < sw_all || worst_move.is_empty() {
            sw_all = score;
            worst_move = mkey.clone();
        }
        if !*is_sleep && (score < sw_no_sleep || !any_nosleep) {
            sw_no_sleep = score;
            worst_move_nosleep = mkey.clone();
            any_nosleep = true;
        }
    }
    if !any_nosleep {
        sw_no_sleep = sw_all;
        worst_move_nosleep = worst_move.clone();
    }

    CellOutput {
        cand_idx,
        anchor_idx,
        cand_id: cand.id.clone(),
        anchor_id: anchor.id.clone(),
        lead,
        sw_all,
        sw_no_sleep,
        worst_move,
        worst_move_nosleep,
        cap_rate: if total_games > 0 {
            cap_games as f64 / total_games as f64
        } else {
            0.0
        },
        avg_turns: if total_games > 0 {
            turn_sum as f64 / total_games as f64
        } else {
            0.0
        },
    }
}

fn main() {
    let t0 = Instant::now();
    let cli = Arc::new(parse_cli());
    let doc: InputDoc =
        serde_json::from_str(&fs::read_to_string(&cli.input).expect("read input")).expect("parse");
    let dex = Arc::new(load_dex());

    let anchor_sw: Arc<Vec<Vec<(MoveId, String, bool)>>> = Arc::new(
        doc.anchors
            .iter()
            .map(|a| relevant_switch_moves(&dex, a))
            .collect(),
    );
    let anchors = Arc::new(doc.anchors);
    let candidates = Arc::new(doc.candidates);

    let cells: Vec<[usize; 2]> = match doc.cells {
        Some(cs) => cs,
        None => {
            let mut v = Vec::with_capacity(candidates.len() * anchors.len());
            for ci in 0..candidates.len() {
                for ai in 0..anchors.len() {
                    v.push([ci, ai]);
                }
            }
            v
        }
    };
    let n_cells = cells.len();
    let cells = Arc::new(cells);
    let next_idx = Arc::new(AtomicUsize::new(0));
    let results: Arc<Mutex<Vec<Option<CellOutput>>>> =
        Arc::new(Mutex::new((0..n_cells).map(|_| None).collect()));

    let mut handles = Vec::new();
    for _ in 0..cli.threads {
        let dex = dex.clone();
        let anchors = anchors.clone();
        let anchor_sw = anchor_sw.clone();
        let candidates = candidates.clone();
        let cells = cells.clone();
        let next_idx = next_idx.clone();
        let results = results.clone();
        let cli = cli.clone();
        handles.push(std::thread::spawn(move || {
            let mut local = Vec::new();
            loop {
                let idx = next_idx.fetch_add(1, Ordering::Relaxed);
                if idx >= cells.len() {
                    break;
                }
                let [ci, ai] = cells[idx];
                let out = eval_cell(
                    &dex,
                    &candidates[ci],
                    &anchors[ai],
                    ci,
                    ai,
                    &anchor_sw[ai],
                    &cli,
                );
                local.push((idx, out));
            }
            let mut guard = results.lock().unwrap();
            for (idx, out) in local {
                guard[idx] = Some(out);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let final_cells: Vec<CellOutput> = Arc::try_unwrap(results)
        .ok()
        .unwrap()
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|o| o.unwrap())
        .collect();

    if let Some(parent) = cli.out.parent() {
        fs::create_dir_all(parent).ok();
    }
    let out_doc = json!({
        "mode": match cli.mode { Mode::Root => "root", Mode::Play => "play" },
        "iters": cli.iters,
        "sw_iters": cli.sw_iters,
        "games": cli.games,
        "sw_samples": cli.sw_samples,
        "cells": final_cells,
    });
    fs::write(&cli.out, serde_json::to_string(&out_doc).unwrap()).expect("write out");

    let secs = t0.elapsed().as_secs_f64();
    let summary = json!({
        "mode": match cli.mode { Mode::Root => "root", Mode::Play => "play" },
        "candidates": candidates.len(),
        "anchors": anchors.len(),
        "cells": n_cells,
        "threads": cli.threads,
        "secs": (secs * 100.0).round() / 100.0,
        "cells_per_sec": ((n_cells as f64 / secs.max(1e-6)) * 10.0).round() / 10.0,
        "out": cli.out.to_string_lossy(),
    });
    println!("{}", summary);
}
