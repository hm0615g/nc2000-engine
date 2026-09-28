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
    fork::{play_out, ForkSpec, Info, ProtocolSeat, Seat},
    preview::{load_meta_pool, MetaPool},
    rng::SplitMix64,
    smmcts::{RmAgent, RmConfig, SelRule},
};
use nc2000_engine::{battle::Outcome, dex::Dex};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("fork_counterfactual: {msg}");
    std::process::exit(2);
}

#[derive(Clone, Copy, PartialEq)]
enum Policy {
    Protocol,
    Skuct,
}

impl Policy {
    fn parse(s: &str) -> Policy {
        match s {
            "protocol" => Policy::Protocol,
            "skuct" => Policy::Skuct,
            _ => fail(format!("unknown policy {s}: protocol or skuct")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Policy::Protocol => "protocol",
            Policy::Skuct => "skuct",
        }
    }
}

struct Run {
    fork: ForkSpec,
    pool: MetaPool,
    bot: Policy,
    foe: Policy,
    iters: u32,
    foe_iters: u32,
    c: f64,
    arms: Vec<usize>,
    seed: u64,
    max_steps: u32,
}

impl Run {
    fn seat(&self, dex: &Dex, policy: Policy, bot: bool, iters: u32, seed: u64) -> Result<Seat, String> {
        Ok(match policy {
            Policy::Skuct => Seat::Engine(Box::new(RmAgent::new(
                RmConfig { iterations: iters, rule: SelRule::Ucb, ..RmConfig::default() },
                seed,
            ))),
            Policy::Protocol => {
                let cfg = RmConfig { rule: SelRule::Ucb, c: self.c, hp_buckets: 16, ..RmConfig::default() };
                let agent = if bot {
                    self.fork.bot_agent(dex, self.pool.clone(), cfg, seed)?
                } else {
                    self.fork.opponent_agent(dex, self.pool.clone(), cfg, seed)?
                };
                Seat::Protocol(ProtocolSeat::new(agent, iters))
            }
        })
    }

    fn trial(&self, dex: &Dex, trial: usize, seeds: [u64; 3]) -> Result<Vec<Value>, String> {
        let [battle_seed, bot_seed, foe_seed] = seeds;
        let bot_side = self.fork.bot_side();
        let mut rows = Vec::new();
        for &i in &self.arms {
            let start = std::time::Instant::now();
            let mut battle = self.fork.battle(dex, battle_seed)?;
            let arm = self.fork.arm_choices(dex, &mut battle)?[i];
            let bot = self.seat(dex, self.bot, true, self.iters, bot_seed)?;
            let foe = self.seat(dex, self.foe, false, self.foe_iters, foe_seed)?;
            let mut seats = if bot_side == 0 { [bot, foe] } else { [foe, bot] };
            let result = play_out(&mut battle, dex, bot_side, arm, &mut seats, self.max_steps)
                .map_err(|e| format!("trial {trial} arm {}: {e}", self.fork.arms[i].input))?;
            let (score, outcome) = match (result.outcome, bot_side) {
                (None, _) => (None, "cap"),
                (Some(Outcome::Tie), _) => (Some(0.5), "tie"),
                (Some(Outcome::P1Win), 0) | (Some(Outcome::P2Win), 1) => (Some(1.0), "win"),
                _ => (Some(0.0), "loss"),
            };
            let (drift, projections) = match &seats[bot_side] {
                Seat::Protocol(seat) => (seat.agent.legality_drift, seat.agent.projections),
                Seat::Engine(_) => (0, 0),
            };
            rows.push(json!({
                "fork": self.fork.label, "turn": self.fork.position.turn,
                "trial": trial, "seed": self.seed, "battle_seed": battle_seed,
                "action": self.fork.arms[i].input, "arm_label": self.fork.arms[i].label,
                "reply": "search", "tail": self.bot.name(),
                "policy": format!("fork/{}/{}-vs-{}", info_name(self.fork.info), self.bot.name(), self.foe.name()),
                "iters": self.iters, "foe_iters": self.foe_iters,
                "score": score, "outcome": outcome, "final_turn": battle.turn, "steps": result.steps,
                "legality_drift": drift, "projections": projections,
                "elapsed_ms": start.elapsed().as_millis() as u64,
            }));
        }
        Ok(rows)
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
    let bot = Policy::parse(&arg("--bot").unwrap_or_else(|| "protocol".into()));
    let default_foe = if fork.opponent_position.is_some() { "protocol" } else { "skuct" };
    let foe = Policy::parse(&arg("--foe").unwrap_or_else(|| default_foe.into()));
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
    let run = Run {
        pool: load_meta_pool(&root.join("data/meta-pool-v0/meta-pool.json")),
        fork,
        bot,
        foe,
        iters,
        foe_iters,
        c,
        arms,
        seed,
        max_steps: num("--max-steps", 3300) as u32,
    };
    let mut rng = SplitMix64::new(seed);
    for _ in 0..first * 3 {
        rng.next();
    }
    let schedule: Vec<[u64; 3]> = (0..trials).map(|_| [rng.next(), rng.next(), rng.next()]).collect();
    eprintln!(
        "fork_counterfactual: {} — {} arms x {trials} trials, bot {} {iters} it, foe {} {foe_iters} it, c {c}, {threads} threads",
        run.fork.label, run.arms.len(), bot.name(), foe.name()
    );

    let next = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel::<(usize, Result<Vec<Value>, String>)>();
    let mut tally: BTreeMap<String, [u32; 4]> = BTreeMap::new();
    std::thread::scope(|scope| {
        for _ in 0..threads.min(trials) {
            let tx = tx.clone();
            let (run, dex, schedule, next) = (&run, &dex, &schedule, &next);
            scope.spawn(move || loop {
                let k = next.fetch_add(1, Ordering::Relaxed);
                if k >= schedule.len() {
                    break;
                }
                if tx.send((k, run.trial(dex, first + k, schedule[k]))).is_err() {
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
