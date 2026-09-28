//! Build a `nc2000-fork-v1` document from a recorded battle.
//!
//!   cargo run --release -p nc2000-bot --example fork_bundle -- \
//!     --log BATTLE.log --turn 11 --side 1 \
//!     --own-team BOT-TEAM.json --opponent-team OPPONENT-TEAM.json \
//!     --arm 'claimed=move earthquake' [--arm ...] \
//!     [--own-picks a,b,c] [--opponent-picks a,b,c] [--info blind|open] \
//!     [--label TEXT] [--out FORK.json]
//!
//! `--side` is the bot. The decision is the bot's action at the start of
//! `--turn`; that action is always included as the arm labelled `played`.
//! Both teams are canonicalized by the format validator and must agree with
//! every move and item the log reveals. When the opponent also acted that
//! turn, its own information set is reconstructed as `opponent_position`,
//! which `fork_counterfactual --foe protocol` needs.

use conformance::{fixture::repo_root, load_dex};
use nc2000_bot::{
    corpus::{load_battle, load_sources, reconstruct_context_with_cfg, HumanAction},
    fork::{find_choice, Arm, ForkSpec, Info, SCHEMA},
    position::PositionSpec,
    preview::load_meta_pool,
};
use nc2000_engine::{
    battle::{PokemonSet, SearchChoice},
    dex::{toid, Dex},
    state::Battle,
    validate::{canonicalize_team, Learnsets},
};
use std::path::PathBuf;

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("fork_bundle: {msg}");
    std::process::exit(2);
}

fn values(args: &[String], key: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, a)| *a == key)
        .map(|(i, _)| args.get(i + 1).cloned().unwrap_or_else(|| fail(format!("{key} needs a value"))))
        .collect()
}

fn canonical_team(dex: &Dex, ls: &Learnsets, path: &str) -> Vec<PokemonSet> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("{path}: {e}")));
    let verdict = canonicalize_team(dex, ls, &text);
    if verdict["ok"] != true {
        fail(format!("{path} is not a legal team: {}", verdict["errors"]));
    }
    serde_json::from_value(verdict["team"].clone()).unwrap_or_else(|e| fail(format!("{path}: {e}")))
}

fn species_list(arg: Option<&String>) -> Vec<String> {
    arg.map(|s| s.split(',').map(|x| toid(x.trim())).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default()
}

/// The decision point of `side` at `turn` as that side saw it, with `team`
/// as its exact sets. `picks` completes a selection the log has not revealed.
fn view(
    dex: &Dex,
    root: &std::path::Path,
    pool: &nc2000_bot::preview::MetaPool,
    log: &nc2000_bot::corpus::CorpusBattle,
    side: usize,
    turn: u16,
    team: &[PokemonSet],
    picks: &[String],
    picks_flag: &str,
) -> Option<PositionSpec> {
    let d = log.decisions.iter().find(|d| d.side == side && d.turn == turn)?;
    let mut src = load_sources(dex, root);
    for set in team {
        let id = dex
            .species
            .id(&toid(&set.species))
            .unwrap_or_else(|| fail(format!("unknown species {}", set.species)));
        src.by_species.insert(id, vec![serde_json::to_value(set).unwrap()]);
    }
    let rec = reconstruct_context_with_cfg(dex, &src, pool.clone(), &log.lines, &log.evidence, d, 1, nc2000_bot::corpus::cfg())
        .unwrap_or_else(|| fail(format!("side {side} turn {turn}: reconstruction failed")));
    if let Some(i) = rec.provenance.iter().position(|p| *p != "cand-full") {
        fail(format!("side {side}: the log contradicts the supplied set for roster slot {i} ({})", rec.provenance[i]));
    }
    let mut spec = rec.agent.to_position_spec(dex).unwrap();
    for (i, set) in spec.own_sets.clone().iter().enumerate() {
        let truth = team
            .iter()
            .find(|t| toid(&t.species) == toid(&set.species))
            .unwrap_or_else(|| fail(format!("side {side}: {} missing from the team file", set.species)));
        spec.own_sets[i] = truth.clone();
    }
    let me = &mut spec.sides[side];
    if rec.imputed_pick || !picks.is_empty() {
        if picks.len() != 3 {
            fail(format!(
                "side {side} has not revealed all three picks by turn {turn}; pass {picks_flag} with all three species"
            ));
        }
        let slot_of = |species: &str| {
            me.mons
                .iter()
                .position(|m| m.species == species)
                .unwrap_or_else(|| fail(format!("pick {species} is not in side {side}'s team")))
        };
        let mut party: Vec<usize> = me.active.into_iter().collect();
        for (i, m) in me.mons.iter().enumerate() {
            if m.appeared && me.active != Some(i) {
                party.push(i);
            }
        }
        for slot in picks.iter().map(|p| slot_of(p)) {
            if !party.contains(&slot) {
                party.push(slot);
            }
        }
        if party.len() != 3 {
            fail(format!("side {side}: picks {picks:?} leave out a mon that has already appeared"));
        }
        me.party = party;
    }
    Some(spec)
}

fn played_choice(dex: &Dex, battle: &Battle, side: usize, legal: &[SearchChoice], action: &HumanAction) -> SearchChoice {
    match action {
        HumanAction::Move(key) => find_choice(dex, legal, &format!("move {key}")),
        HumanAction::Switch(species) => {
            let side = &battle.sides[side];
            let pos = side
                .party
                .iter()
                .position(|&slot| dex.species.key(side.roster[slot as usize].species) == species);
            pos.and_then(|p| find_choice(dex, legal, &format!("switch {}", p + 1)))
        }
    }
    .unwrap_or_else(|| fail(format!("the recorded action {action:?} is not legal in the reconstruction")))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |key: &str| values(&args, key).into_iter().last();
    let need = |key: &str| arg(key).unwrap_or_else(|| fail(format!("{key} is required")));
    let root = repo_root();
    let dex = load_dex();
    let ls = Learnsets::from_json(&std::fs::read_to_string(root.join("data/learnsets-gen2.json")).unwrap()).unwrap();
    let side: usize = need("--side").parse().unwrap_or_else(|_| fail("--side is 0 or 1"));
    let turn: u16 = need("--turn").parse().unwrap_or_else(|_| fail("--turn is a number"));
    if side > 1 {
        fail("--side is 0 or 1");
    }
    let info = match arg("--info").as_deref().unwrap_or("blind") {
        "blind" => Info::Blind,
        "open" => Info::Open,
        other => fail(format!("--info {other}: blind or open")),
    };
    let own_team = canonical_team(&dex, &ls, &need("--own-team"));
    let opponent_team_raw = canonical_team(&dex, &ls, &need("--opponent-team"));
    let own_picks = species_list(arg("--own-picks").as_ref());
    let opponent_picks = species_list(arg("--opponent-picks").as_ref());
    let log_path = PathBuf::from(need("--log"));
    let log = load_battle(&log_path);
    if log.lines.is_empty() {
        fail(format!("{} has no protocol lines", log_path.display()));
    }
    let pool = load_meta_pool(&root.join("data/meta-pool-v0/meta-pool.json"));

    let bot_decision = log
        .decisions
        .iter()
        .find(|d| d.side == side && d.turn == turn)
        .unwrap_or_else(|| fail(format!("side {side} makes no move or switch at the start of turn {turn}")));
    let position = view(&dex, &root, &pool, &log, side, turn, &own_team, &own_picks, "--own-picks").unwrap();
    let opponent = &position.sides[1 - side];
    let opponent_team: Vec<PokemonSet> = opponent
        .mons
        .iter()
        .map(|m| {
            opponent_team_raw
                .iter()
                .find(|s| toid(&s.species) == m.species)
                .cloned()
                .unwrap_or_else(|| fail(format!("the opponent previewed {} but the team file lacks it", m.species)))
        })
        .collect();
    let opponent_position =
        view(&dex, &root, &pool, &log, 1 - side, turn, &opponent_team, &opponent_picks, "--opponent-picks");
    if opponent_position.is_none() {
        eprintln!("fork_bundle: the opponent did not act at turn {turn}; no opponent_position (--foe protocol unavailable)");
    }

    let mut fork = ForkSpec {
        schema: SCHEMA.into(),
        label: arg("--label").unwrap_or_else(|| format!("{} turn {turn}", log_path.file_stem().unwrap().to_string_lossy())),
        info,
        position,
        opponent_team,
        opponent_picks,
        opponent_position,
        arms: Vec::new(),
    };
    let mut battle = fork.battle(&dex, 0).unwrap_or_else(|e| fail(e));
    let legal = battle.legal_choices(&dex, side);
    let played = played_choice(&dex, &battle, side, &legal, &bot_decision.action);
    fork.arms.push(Arm { input: played.to_input(&dex), label: "played".into() });
    for spec in values(&args, "--arm") {
        let (label, input) = match spec.split_once('=') {
            Some((l, i)) => (l.trim().to_string(), i.trim().to_string()),
            None => (String::new(), spec.trim().to_string()),
        };
        let choice = find_choice(&dex, &legal, &input).unwrap_or_else(|| {
            let names: Vec<String> = legal.iter().map(|c| c.to_input(&dex)).collect();
            fail(format!("--arm {input} is not legal; legal: {}", names.join(" / ")))
        });
        if choice == played {
            if !label.is_empty() {
                fork.arms[0].label = format!("played/{label}");
            }
            continue;
        }
        fork.arms.push(Arm { input: choice.to_input(&dex), label });
    }
    if fork.arms.len() < 2 {
        eprintln!("fork_bundle: only the played action is an arm; add --arm for the alternative");
    }
    fork.check(&dex).unwrap_or_else(|e| fail(e));
    let json = serde_json::to_string_pretty(&fork).unwrap();
    match arg("--out") {
        Some(path) => std::fs::write(&path, json + "\n").unwrap_or_else(|e| fail(format!("{path}: {e}"))),
        None => println!("{json}"),
    }
    let names: Vec<String> = legal.iter().map(|c| c.to_input(&dex)).collect();
    eprintln!("fork_bundle: arms {:?}; legal at the fork: {}", fork.arms.iter().map(|a| &a.input).collect::<Vec<_>>(), names.join(" / "));
}
