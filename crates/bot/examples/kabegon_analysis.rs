use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use conformance::load_dex;
use nc2000_bot::preview::load_meta_pool;
use nc2000_bot::rng::SplitMix64;
use nc2000_bot::smmcts::{RmAgent, RmConfig, SelRule};
use nc2000_bot::Agent;
use nc2000_engine::battle::{Outcome, PokemonSet, SearchChoice};
use nc2000_engine::dex::Dex;
use nc2000_engine::state::Battle;

fn make_kabegon_team() -> Vec<PokemonSet> {
    let json_text = r#"[
  {
    "name": "Snorlax",
    "species": "Snorlax",
    "item": "Leftovers",
    "ability": "No Ability",
    "moves": ["Double-Edge", "Curse", "Rest", "Sleep Talk"],
    "nature": "Serious",
    "evs": {"hp": 255, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255},
    "gender": "M",
    "ivs": {"hp": 31, "atk": 31, "def": 31, "spa": 31, "spd": 31, "spe": 31},
    "level": 55,
    "happiness": 255
  },
  {
    "name": "Marowak",
    "species": "Marowak",
    "item": "Thick Club",
    "ability": "No Ability",
    "moves": ["Bonemerang", "Rock Slide", "Hidden Power Bug", "Swords Dance"],
    "nature": "Serious",
    "evs": {"hp": 255, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255},
    "gender": "M",
    "ivs": {"hp": 31, "atk": 27, "def": 27, "spa": 31, "spd": 31, "spe": 31},
    "level": 50,
    "happiness": 255
  },
  {
    "name": "Starmie",
    "species": "Starmie",
    "item": "Miracle Berry",
    "ability": "No Ability",
    "moves": ["Hydro Pump", "Psychic", "Thunder Wave", "Recover"],
    "nature": "Serious",
    "evs": {"hp": 255, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255},
    "gender": "N",
    "ivs": {"hp": 31, "atk": 3, "def": 31, "spa": 31, "spd": 31, "spe": 31},
    "level": 50,
    "happiness": 255
  },
  {
    "name": "Blissey",
    "species": "Blissey",
    "item": "Quick Claw",
    "ability": "No Ability",
    "moves": ["Ice Beam", "Fire Blast", "Present", "Soft-Boiled"],
    "nature": "Serious",
    "evs": {"hp": 255, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255},
    "gender": "F",
    "ivs": {"hp": 31, "atk": 3, "def": 31, "spa": 31, "spd": 31, "spe": 31},
    "level": 50,
    "happiness": 255
  },
  {
    "name": "Heracross",
    "species": "Heracross",
    "item": "Scope Lens",
    "ability": "No Ability",
    "moves": ["Megahorn", "Earthquake", "Rest", "Sleep Talk"],
    "nature": "Serious",
    "evs": {"hp": 255, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255},
    "gender": "M",
    "ivs": {"hp": 31, "atk": 31, "def": 31, "spa": 31, "spd": 31, "spe": 31},
    "level": 50,
    "happiness": 255
  },
  {
    "name": "Moltres",
    "species": "Moltres",
    "item": "Charcoal",
    "ability": "No Ability",
    "moves": ["Fire Blast", "Sunny Day", "Rest", "Sleep Talk"],
    "nature": "Serious",
    "evs": {"hp": 255, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255},
    "gender": "N",
    "ivs": {"hp": 31, "atk": 3, "def": 31, "spa": 31, "spd": 31, "spe": 31},
    "level": 50,
    "happiness": 255
  }
]"#;
    serde_json::from_str(json_text).expect("Failed to parse Kabegon team JSON")
}

struct MatchupResult {
    opp_id: String,
    games_played: usize,
    kabegon_wins: usize,
    opp_wins: usize,
    ties: usize,
    kabegon_picks: HashMap<String, usize>,
    kabegon_leads: HashMap<String, usize>,
    opp_picks: HashMap<String, usize>,
    opp_leads: HashMap<String, usize>,
    avg_turns: f64,
    narrative_samples: Vec<String>,
}

fn run_matchup(
    dex: &Dex,
    opp_id: &str,
    opp_team: &[PokemonSet],
    kabegon_team: &[PokemonSet],
    num_pairs: usize,
) -> MatchupResult {
    let mut kabegon_wins = 0;
    let mut opp_wins = 0;
    let mut ties = 0;
    let mut total_turns = 0usize;

    let mut kabegon_picks: HashMap<String, usize> = HashMap::new();
    let mut kabegon_leads: HashMap<String, usize> = HashMap::new();
    let mut opp_picks: HashMap<String, usize> = HashMap::new();
    let mut opp_leads: HashMap<String, usize> = HashMap::new();
    let mut narrative_samples = Vec::new();

    let iters = 300;
    let base_seed = 20261106u64;

    for pair_idx in 0..num_pairs {
        let mut rng = SplitMix64::new(base_seed.wrapping_add(pair_idx as u64 * 10007));
        let bseed = rng.battle_seed();

        for swap in 0..2 {
            let kabegon_is_p1 = swap == 0;
            let (t1, t2) = if kabegon_is_p1 {
                (kabegon_team, opp_team)
            } else {
                (opp_team, kabegon_team)
            };

            let mut battle = Battle::from_fixture(dex, &bseed, t1, t2).unwrap();
            battle.set_log_enabled(true);

            let mut a1 = RmAgent::new(
                RmConfig {
                    iterations: iters,
                    rule: SelRule::Ucb,
                    ..Default::default()
                },
                rng.next(),
            );
            let mut a2 = RmAgent::new(
                RmConfig {
                    iterations: iters,
                    rule: SelRule::Ucb,
                    ..Default::default()
                },
                rng.next(),
            );

            let mut kabegon_chosen_slots = [0u8; 3];
            let mut opp_chosen_slots = [0u8; 3];

            let mut key_events = Vec::new();

            // Run turns
            while battle.outcome().is_none() && battle.turn <= 250 {
                let mut picks = [None, None];
                for s in 0..2 {
                    let cs = battle.legal_choices(dex, s);
                    if !cs.is_empty() {
                        let agent: &mut dyn Agent = if s == 0 { &mut a1 } else { &mut a2 };
                        let pick = agent.choose(&battle, dex, s, &cs);
                        if battle.turn == 0 {
                            if let SearchChoice::Team(slots) = pick {
                                if (s == 0) == kabegon_is_p1 {
                                    kabegon_chosen_slots = slots;
                                } else {
                                    opp_chosen_slots = slots;
                                }
                            }
                        }
                        picks[s] = Some(pick);
                    }
                }
                if picks == [None, None] {
                    break;
                }
                let turn_before = battle.turn;
                battle.apply_choices(dex, picks).unwrap();

                // Record summary lines if this game is sampled for narrative
                if pair_idx < 2 && swap == 0 && battle.turn > turn_before {
                    // Extract interesting recent logs
                    for line in &battle.log[battle.last_move_line.max(0) as usize..] {
                        if line.contains("|move|") || line.contains("|switch|") || line.contains("|-faint|") {
                            key_events.push(format!("  T{}: {}", turn_before, line));
                        }
                    }
                }
            }

            // Record picks
            let k_lead = &kabegon_team[kabegon_chosen_slots[0] as usize - 1].species;
            *kabegon_leads.entry(k_lead.clone()).or_insert(0) += 1;
            for &slot in &kabegon_chosen_slots {
                let sp = &kabegon_team[slot as usize - 1].species;
                *kabegon_picks.entry(sp.clone()).or_insert(0) += 1;
            }

            let o_lead = &opp_team[opp_chosen_slots[0] as usize - 1].species;
            *opp_leads.entry(o_lead.clone()).or_insert(0) += 1;
            for &slot in &opp_chosen_slots {
                let sp = &opp_team[slot as usize - 1].species;
                *opp_picks.entry(sp.clone()).or_insert(0) += 1;
            }

            total_turns += battle.turn as usize;
            match battle.outcome() {
                Some(Outcome::P1Win) => {
                    if kabegon_is_p1 {
                        kabegon_wins += 1;
                    } else {
                        opp_wins += 1;
                    }
                }
                Some(Outcome::P2Win) => {
                    if kabegon_is_p1 {
                        opp_wins += 1;
                    } else {
                        kabegon_wins += 1;
                    }
                }
                None | Some(Outcome::Tie) => {
                    ties += 1;
                }
            }

            if pair_idx < 2 && swap == 0 {
                let outcome_str = match battle.outcome() {
                    Some(Outcome::P1Win) => "Kabegon WIN",
                    Some(Outcome::P2Win) => "Opponent WIN",
                    _ => "TIE",
                };
                let sample_text = format!(
                    "Game Sample (Kabegon: [{}], Opponent: [{}]) -> {} (Turns: {})\n{}",
                    kabegon_chosen_slots.iter().map(|s| kabegon_team[*s as usize - 1].species.as_str()).collect::<Vec<_>>().join(", "),
                    opp_chosen_slots.iter().map(|s| opp_team[*s as usize - 1].species.as_str()).collect::<Vec<_>>().join(", "),
                    outcome_str,
                    battle.turn,
                    key_events.join("\n")
                );
                narrative_samples.push(sample_text);
            }
        }
    }

    let games_played = num_pairs * 2;
    MatchupResult {
        opp_id: opp_id.to_string(),
        games_played,
        kabegon_wins,
        opp_wins,
        ties,
        kabegon_picks,
        kabegon_leads,
        opp_picks,
        opp_leads,
        avg_turns: total_turns as f64 / games_played as f64,
        narrative_samples,
    }
}

fn main() {
    let t0 = Instant::now();
    let dex = load_dex();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let pool_path = root.join("data/team-pool-v2/team-pool.json");
    let pool = load_meta_pool(&pool_path);

    let kabegon = make_kabegon_team();

    let target_opponents = [
        ("apex-c08-surgical-142", "Nash v14 (55カビゴン + 50パルシェン + 50ゲンガー)"),
        ("sample-06", "55カビゴン/ライコウ + 50エアームド + 50スイクン (受けループ)"),
        ("nb-sample-13-0", "55カビゴン + 50ハガネール + 50ゲンガー + 50パルシェン"),
    ];

    println!("==================================================================");
    println!("カベゴン (Kabegon) vs 55カビゴン+カビゴン受け 構築群の対戦実験");
    println!("==================================================================");

    for (opp_id, desc) in target_opponents {
        let opp_team = pool
            .teams
            .iter()
            .find(|t| t.id == opp_id)
            .unwrap_or_else(|| panic!("team {opp_id} not found"));

        println!("\n>>> 対戦相手: {} ({})", opp_id, desc);
        let res = run_matchup(&dex, opp_id, &opp_team.sets, &kabegon, 15); // 15 pairs = 30 games per matchup

        println!("対戦数: {} 試合", res.games_played);
        println!(
            "戦績: カベゴン {}勝 - {}敗 - {}分 (勝率: {:.1}%) | 平均ターン: {:.1}",
            res.kabegon_wins,
            res.opp_wins,
            res.ties,
            res.kabegon_wins as f64 / res.games_played as f64 * 100.0,
            res.avg_turns
        );

        println!("-- カベゴン選出率 (3枠合計/{}枠) --", res.games_played * 3);
        let mut kp: Vec<_> = res.kabegon_picks.into_iter().collect();
        kp.sort_by(|a, b| b.1.cmp(&a.1));
        for (sp, count) in kp {
            let lead_count = res.kabegon_leads.get(&sp).copied().unwrap_or(0);
            println!(
                "  {:12}: 選出 {}回 ({:.1}%), 初手 {}回 ({:.1}%)",
                sp,
                count,
                count as f64 / res.games_played as f64 * 100.0,
                lead_count,
                lead_count as f64 / res.games_played as f64 * 100.0
            );
        }

        println!("-- 相手側選出率 (3枠合計/{}枠) --", res.games_played * 3);
        let mut op: Vec<_> = res.opp_picks.into_iter().collect();
        op.sort_by(|a, b| b.1.cmp(&a.1));
        for (sp, count) in op {
            let lead_count = res.opp_leads.get(&sp).copied().unwrap_or(0);
            println!(
                "  {:12}: 選出 {}回 ({:.1}%), 初手 {}回 ({:.1}%)",
                sp,
                count,
                count as f64 / res.games_played as f64 * 100.0,
                lead_count,
                lead_count as f64 / res.games_played as f64 * 100.0
            );
        }

        if !res.narrative_samples.is_empty() {
            println!("\n[代表的な試合ログサンプル (Turn抜粋)]");
            for (idx, sample) in res.narrative_samples.iter().enumerate() {
                println!("--- サンプル {} ---", idx + 1);
                let lines: Vec<&str> = sample.lines().collect();
                for line in lines.iter().take(25) {
                    println!("{}", line);
                }
                if lines.len() > 25 {
                    println!("  ... (中略 {} 行) ...", lines.len() - 25);
                }
            }
        }
    }

    println!("\n総実行時間: {:.2}s", t0.elapsed().as_secs_f64());
}
