//! Bot-vs-bot continuations of a `nc2000-fork-v1` position, one row per
//! (trial, arm) on stdout in the `tools/summarize-counterfactual.py` schema.
//!
//!   cargo run --release -p nc2000-bot --example fork_counterfactual -- \
//!     --fork FORK.json [--trials 256] [--seed 1] [--start-trial 0] \
//!     [--bot protocol|skuct] [--foe protocol|skuct] [--iters N] [--foe-iters N] \
//!     [--arm all|INPUT] [--threads N] > rows.jsonl
//!   python3 tools/summarize-counterfactual.py rows.jsonl
//!
//! Every arm of a trial shares the trial's battle seed and both agents'
//! seeds, so arms are paired: they differ only by the bot's first action and
//! what follows from it. `protocol` is the ladder agent under the fork's
//! information policy, continuing from its own recorded information set
//! (the opponent's needs `opponent_position`); `skuct` reads the full battle.
//! Iterations default to the fork's profile in `data/search-profiles.json`.

use conformance::{fixture::repo_root, load_dex};
use nc2000_bot::{
    fork::{Arena, ForkSpec, Info, Policy},
    preview::load_meta_pool,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("fork_counterfactual: {msg}");
    std::process::exit(2);
}

fn policy(s: &str) -> Policy {
    match s {
        "protocol" => Policy::Protocol,
        "skuct" => Policy::Skuct,
        _ => fail(format!("unknown policy {s}: protocol or skuct")),
    }
}

fn info_name(info: Info) -> &'static str {
    match info {
        Info::Blind => "blind",
        Info::Open => "open",
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |key: &str| -> Option<String> {
        args.iter().position(|s| s == key).map(|i| args.get(i + 1).cloned().unwrap_or_else(|| fail(format!("{key} needs a value"))))
    };
    let num = |key: &str, default: u64| -> u64 {
        arg(key).map(|v| v.parse().unwrap_or_else(|_| fail(format!("{key} {v}: not a number")))).unwrap_or(default)
    };
    let root = repo_root();
    let dex = load_dex();
    let path = arg("--fork").unwrap_or_else(|| fail("--fork FORK.json is required"));
    let fork = ForkSpec::parse(&std::fs::read_to_string(&path).unwrap_or_else(|e| fail(format!("{path}: {e}"))))
        .unwrap_or_else(|e| fail(e));
    fork.check(&dex).unwrap_or_else(|e| fail(e));
    let profiles: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("data/search-profiles.json")).unwrap()).unwrap();
    let profile = &profiles[info_name(fork.info)];
    let bot = policy(&arg("--bot").unwrap_or_else(|| "protocol".into()));
    let default_foe = if fork.opponent_position.is_some() { "protocol" } else { "skuct" };
    let foe = policy(&arg("--foe").unwrap_or_else(|| default_foe.into()));
    let iters = num("--iters", profile["iterations"].as_u64().unwrap()) as u32;
    let foe_iters = num("--foe-iters", iters as u64) as u32;
    let c = arg("--c").map(|v| v.parse().unwrap_or_else(|_| fail("--c: not a number"))).unwrap_or_else(|| profile["c"].as_f64().unwrap());
    let trials = num("--trials", 256) as usize;
    let first = num("--start-trial", 0) as usize;
    let seed = num("--seed", 1);
    let threads = num("--threads", std::thread::available_parallelism().map_or(1, |n| n.get()) as u64).max(1) as usize;
    let only = arg("--arm").unwrap_or_else(|| "all".into());
    let arms: Vec<usize> = (0..fork.arms.len())
        .filter(|&i| only == "all" || fork.arms[i].input == only || fork.arms[i].label == only)
        .collect();
    if arms.is_empty() {
        fail(format!("--arm {only} matches no arm; arms: {:?}", fork.arms.iter().map(|a| &a.input).collect::<Vec<_>>()));
    }
    if trials == 0 || iters == 0 || foe_iters == 0 {
        fail("--trials, --iters and --foe-iters must be positive");
    }
    let arena = Arena { bot, foe, iters, foe_iters, c, seed, max_steps: num("--max-steps", 3300) as u32 };
    let pool = load_meta_pool(&root.join("data/meta-pool-v0/meta-pool.json"));
    eprintln!(
        "fork_counterfactual: {} — {} arms x {trials} trials, bot {} {iters} it, foe {} {foe_iters} it, c {c}, {threads} threads",
        fork.label, arms.len(), bot.name(), foe.name()
    );
    let trial = |k: usize| -> Result<Vec<Value>, String> {
        arms.iter()
            .map(|&arm| {
                let start = std::time::Instant::now();
                let mut row = arena.play(&dex, &fork, &pool, k, arm)?;
                row["elapsed_ms"] = (start.elapsed().as_millis() as u64).into();
                Ok(row)
            })
            .collect()
    };

    let next = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel::<(usize, Result<Vec<Value>, String>)>();
    let mut tally: BTreeMap<String, [u32; 4]> = BTreeMap::new();
    std::thread::scope(|scope| {
        for _ in 0..threads.min(trials) {
            let tx = tx.clone();
            let (trial, next) = (&trial, &next);
            scope.spawn(move || loop {
                let k = next.fetch_add(1, Ordering::Relaxed);
                if k >= trials {
                    break;
                }
                if tx.send((k, trial(first + k))).is_err() {
                    break;
                }
            });
        }
        drop(tx);
        let mut out = std::io::BufWriter::new(std::io::stdout().lock());
        let mut pending = BTreeMap::new();
        let mut emitted = 0;
        for (k, result) in rx {
            let rows = result.unwrap_or_else(|e| fail(e));
            pending.insert(k, rows);
            while let Some(rows) = pending.remove(&emitted) {
                for row in rows {
                    let t = tally.entry(row["action"].as_str().unwrap().to_string()).or_default();
                    match row["outcome"].as_str().unwrap() {
                        "win" => t[0] += 1,
                        "loss" => t[1] += 1,
                        "tie" => t[2] += 1,
                        _ => t[3] += 1,
                    }
                    writeln!(out, "{row}").unwrap();
                }
                out.flush().unwrap();
                emitted += 1;
            }
        }
    });
    for (action, [w, l, t, cap]) in &tally {
        let n = w + l + t + cap;
        eprintln!(
            "  {action:<28} {w:>5}W {l:>5}L {t:>3}T {cap:>3}cap  win rate {:.3}",
            (*w as f64 + 0.5 * *t as f64) / n.max(1) as f64
        );
    }
}
