//! How Perish Song lines play out between two agents, on the arena's exact
//! schedule: the same team draws, battle seeds and (with --crn-seeds) agent
//! seeds as `arena`, so the scores it prints must equal the arena's for the
//! same arguments. Each decision is read off the live state before it is
//! applied, never off the protocol log.
//!
//!   combo_census <agentA> <agentB> [--games N] [--seed S] [--threads T]
//!       [--crn-seeds] [--perish-min N] [--max-turns M] [--jsonl FILE]
//!
//! Agent specs: (open|blind):ITERS[:C][:-perish_combo|:perish_escape] (shipped rules by default)
//!
//! Per side and game it counts: Perish Songs chosen (and how many of them
//! against a foe that could not switch), trapping moves chosen, and, at
//! Perish count 1, escapes (switch chosen), stays (move chosen while a switch
//! was legal) and trapped turns; plus mons that fainted at count 1.

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use conformance::fixture::repo_root;
use conformance::load_dex;
use nc2000_bot::preview::load_meta_pool;
use nc2000_bot::{Agent, SplitMix64};
use nc2000_engine::battle::PokemonSet;
use nc2000_engine::state::Battle;
use serde_json::json;

mod combo_common;
use combo_common::{is_trapper, play, Spec, Tally};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let spec_a = Spec::parse(&args[0]);
    let spec_b = Spec::parse(&args[1]);
    let games: usize = flag(&args, "--games").map_or(100, |v| v.parse().unwrap());
    let games = games + games % 2;
    let base_seed: u64 = flag(&args, "--seed").map_or(1, |v| v.parse().unwrap());
    let max_turns: u16 = flag(&args, "--max-turns").map_or(500, |v| v.parse().unwrap());
    let crn = args.iter().any(|a| a == "--crn-seeds");
    let threads: usize = flag(&args, "--threads")
        .map(|v| v.parse().unwrap())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));
    let dex = load_dex();
    let pool = Arc::new(load_meta_pool(
        &repo_root().join("data/meta-pool-v0/meta-pool.json"),
    ));
    let mut teams: Vec<Vec<PokemonSet>> = pool.teams.iter().map(|t| t.sets.clone()).collect();
    if let Some(n) = flag(&args, "--perish-min").map(|v| v.parse::<usize>().unwrap()) {
        teams.retain(|sets| sets.iter().filter(|s| is_trapper(s)).count() >= n);
    }
    eprintln!("{} teams", teams.len());

    let mut sched = SplitMix64::new(base_seed);
    let mut specs = Vec::new();
    for _ in 0..games / 2 {
        let t1 = sched.below(teams.len());
        let t2 = sched.below(teams.len());
        let seed = sched.battle_seed();
        specs.push((t1, t2, seed.clone(), true));
        specs.push((t1, t2, seed, false));
    }
    let cursor = AtomicUsize::new(0);
    let mut rows: Vec<(usize, f64, u16, [Tally; 2], usize, usize)> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let (specs, cursor, dex, teams, pool, spec_a, spec_b) =
                    (&specs, &cursor, &dex, &teams, &pool, &spec_a, &spec_b);
                scope.spawn(move || {
                    let mut out = Vec::new();
                    loop {
                        let i = cursor.fetch_add(1, Ordering::Relaxed);
                        if i >= specs.len() {
                            break;
                        }
                        let (t1, t2, seed, a_is_p1) = &specs[i];
                        let (sa, sb) = if crn {
                            let s =
                                base_seed ^ ((i / 2) as u64).wrapping_mul(0xA24B_AED4_963E_E407);
                            (s, s)
                        } else {
                            (
                                base_seed ^ (i as u64).wrapping_mul(0xA24B_AED4_963E_E407),
                                base_seed ^ (i as u64).wrapping_mul(0x9FB2_1C65_1E98_DF25),
                            )
                        };
                        let mut a = spec_a.build(sa, pool);
                        let mut b = spec_b.build(sb, pool);
                        let mut battle =
                            Battle::from_fixture(dex, seed, &teams[*t1], &teams[*t2]).unwrap();
                        battle.set_log_enabled(true);
                        let (p1, p2): (&mut dyn Agent, &mut dyn Agent) = if *a_is_p1 {
                            (&mut *a, &mut *b)
                        } else {
                            (&mut *b, &mut *a)
                        };
                        let (p1_score, turns, t) = play(dex, &mut battle, [p1, p2], max_turns);
                        let a_score = if *a_is_p1 { p1_score } else { 1.0 - p1_score };
                        let by_arm = if *a_is_p1 { t } else { [t[1], t[0]] };
                        out.push((i, a_score, turns, by_arm, *t1, *t2));
                    }
                    out
                })
            })
            .collect();
        for h in handles {
            rows.extend(h.join().unwrap());
        }
    });
    rows.sort_by_key(|r| r.0);

    if let Some(path) = flag(&args, "--jsonl") {
        let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        for (i, score, turns, t, t1, t2) in &rows {
            writeln!(
                f,
                "{}",
                json!({"game":i,"a_score":score,"turns":turns,"team_p1":t1,"team_p2":t2,
                "a":t[0].json(),"b":t[1].json()})
            )
            .unwrap();
        }
    }
    let pairs: Vec<f64> = rows
        .chunks_exact(2)
        .map(|p| (p[0].1 + p[1].1) / 2.0)
        .collect();
    let mean = pairs.iter().sum::<f64>() / pairs.len() as f64;
    let var = pairs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (pairs.len() as f64 - 1.0);
    let split = pairs.iter().filter(|&&p| p != 0.5).count();
    println!(
        "A={} vs B={}: score {:.3} +/- {:.3} over {} games, split pairs {split}/{}",
        args[0],
        args[1],
        mean,
        1.96 * (var / pairs.len() as f64).sqrt(),
        rows.len(),
        pairs.len()
    );
    let mut arm = [Tally::default(); 2];
    for r in &rows {
        arm[0].add(&r.3[0]);
        arm[1].add(&r.3[1]);
    }
    for (name, t) in ["A", "B"].iter().zip(arm) {
        println!("{name}: {}", t.json());
    }
    let sung: Vec<&(usize, f64, u16, [Tally; 2], usize, usize)> = rows
        .iter()
        .filter(|r| r.3[0].sing + r.3[1].sing > 0)
        .collect();
    let s = sung.iter().map(|r| r.1).sum::<f64>() / sung.len().max(1) as f64;
    println!(
        "games with a Perish Song chosen: {} (A score in them {:.3})",
        sung.len(),
        s
    );
}
