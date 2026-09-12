use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use nc2000_bot::player::PlayerFrame;
use nc2000_bot::preview::load_meta_pool;
use nc2000_bot::{ProtocolAgent, RmConfig};
use nc2000_engine::dex::Dex;
use serde_json::{json, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if !(3..=5).contains(&args.len()) {
        return Err("usage: profile_blind RECORDED_GAMES OUTPUT_SVG [ITERS] [DECISIONS]".into());
    }
    let iterations: u32 = args
        .get(3)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(30_000);
    let limit: usize = args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(32);
    if iterations == 0 || limit == 0 {
        return Err("iterations and decisions must be positive".into());
    }
    let mut lines = BufReader::new(std::fs::File::open(&args[1])?).lines();
    let manifest: Value = serde_json::from_str(&lines.next().ok_or("missing manifest")??)?;
    let pool = load_meta_pool(&PathBuf::from(
        manifest["config"]["pool"].as_str().ok_or("missing pool")?,
    ));
    let dex_path = manifest["config"]["dex"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| conformance::fixture::repo_root().join("data/gen2stadium2.json"));
    let dex = Dex::from_json(&std::fs::read_to_string(dex_path)?)?;
    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(997)
        .build()?;
    let mut decisions = 0;
    for line in lines {
        let game: Value = serde_json::from_str(&line?)?;
        let swap = game["swap"].as_u64().ok_or("missing swap")? as usize;
        let mut agents: Vec<_> = (0..2)
            .map(|side| {
                let mut agent = ProtocolAgent::new(
                    &dex,
                    side,
                    pool.clone(),
                    RmConfig::default(),
                    game["agent_seeds"][side ^ swap].as_u64().unwrap(),
                );
                agent.set_own_team(
                    pool.teams[game["team_ids"][side].as_u64().unwrap() as usize]
                        .sets
                        .clone(),
                );
                agent
            })
            .collect();
        for entry in game["frames"].as_array().ok_or("missing frames")? {
            let side = entry["side"].as_u64().ok_or("missing side")? as usize;
            let frame: PlayerFrame = serde_json::from_value(entry["frame"].clone())?;
            let agent = &mut agents[side];
            for line in frame.lines {
                agent.push_line(&dex, &line);
            }
            if !agent.on_request(&dex, &frame.request.to_string())? {
                return Err("unexpected wait request".into());
            }
            agent.step(&dex, iterations)?;
            decisions += 1;
            if decisions >= limit {
                break;
            }
        }
        if decisions >= limit {
            break;
        }
    }
    let report = guard.report().build()?;
    report.flamegraph(std::fs::File::create(&args[2])?)?;
    let mut samples: HashMap<String, isize> = HashMap::new();
    for (frames, count) in &report.data {
        if let Some(top) = frames.frames.first().and_then(|f| f.first()) {
            *samples.entry(top.name()).or_default() += *count;
        }
    }
    let mut samples: Vec<_> = samples.into_iter().collect();
    samples.sort_by_key(|(_, count)| -*count);
    let total: isize = samples.iter().map(|(_, count)| count).sum();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "decisions": decisions, "iterations": iterations, "samples": total,
            "self_time": samples.iter().take(40).map(|(symbol, count)| json!({
                "symbol": symbol, "fraction": *count as f64 / total.max(1) as f64,
            })).collect::<Vec<_>>(),
        }))?
    );
    Ok(())
}
