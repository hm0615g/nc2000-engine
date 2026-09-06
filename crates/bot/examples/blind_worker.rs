use std::io::{BufRead, Write};
use std::time::Instant;

use conformance::fixture::repo_root;
use nc2000_bot::player::PlayerFrame;
use nc2000_bot::preview::load_meta_pool;
use nc2000_bot::{ProtocolAgent, RmConfig, SplitMix64};
use nc2000_engine::battle::PokemonSet;
use nc2000_engine::dex::Dex;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    New {
        side: usize,
        team: Vec<PokemonSet>,
        seed: u64,
    },
    Choose {
        frame: PlayerFrame,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--features" | "--describe" => index += 1,
            "--iters" | "--pool" | "--dex" | "--model" | "--temperature" => {
                if index + 1 == args.len() {
                    return Err(format!("missing value for {}", args[index]).into());
                }
                index += 2;
            }
            name => return Err(format!("unknown option {name}").into()),
        }
    }
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let iterations: u32 = flag("--iters")
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(30_000);
    if iterations == 0 {
        return Err("--iters must be positive".into());
    }
    let pool_path = flag("--pool")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| repo_root().join("data/meta-pool-v0/meta-pool.json"));
    let dex_path = flag("--dex")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| repo_root().join("data/gen2stadium2.json"));
    let dex = Dex::from_json(&std::fs::read_to_string(dex_path)?)?;
    let pool = load_meta_pool(&pool_path);
    if args.iter().any(|a| a == "--describe") {
        println!(
            "{}",
            serde_json::to_string(&nc2000_bot::learning::Vocabulary::from_dex(&dex))?
        );
        return Ok(());
    }
    let features = args.iter().any(|a| a == "--features");
    let temperature: f64 = flag("--temperature")
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(0.0);
    if !temperature.is_finite() || temperature < 0.0 {
        return Err("temperature must be finite and nonnegative".into());
    }
    let model = flag("--model")
        .map(|path| {
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            nc2000_bot::learned::PolicyValue::from_json(&text, &dex)
        })
        .transpose()?;
    let mut agent: Option<ProtocolAgent> = None;
    let mut policy_rng = SplitMix64::new(0);
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    for line in std::io::stdin().lock().lines() {
        let command: Command = serde_json::from_str(&line?)?;
        let response = match command {
            Command::New { side, team, seed } => {
                if side > 1 {
                    return Err("side must be 0 or 1".into());
                }
                let mut next =
                    ProtocolAgent::new(&dex, side, pool.clone(), RmConfig::default(), seed);
                next.set_own_team(team);
                agent = Some(next);
                policy_rng = SplitMix64::new(seed ^ 0xC6BC279692B5C323);
                json!({"ready": true, "iterations": iterations})
            }
            Command::Choose { frame } => {
                let started = Instant::now();
                let agent = agent.as_mut().ok_or("choose before new")?;
                for line in &frame.lines {
                    agent.push_line(&dex, line);
                }
                if !agent.on_request(&dex, &frame.request.to_string())? {
                    return Err("choose on wait request".into());
                }
                let encoded = if features || model.is_some() {
                    Some(nc2000_bot::learning::observation(agent, &dex)?)
                } else {
                    None
                };
                let prediction = model
                    .as_ref()
                    .map(|model| model.predict(encoded.as_ref().unwrap()))
                    .transpose()?;
                let mut log_prob = None;
                let action = if let Some((logits, _)) = &prediction {
                    let encoded = encoded.as_ref().unwrap();
                    let eligible: Vec<usize> = logits
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| {
                            encoded.actions[*i].eligible
                                && frame.legal_actions.contains(&encoded.actions[*i].input)
                        })
                        .map(|(i, _)| i)
                        .collect();
                    let best = eligible
                        .iter()
                        .copied()
                        .max_by(|&i, &j| logits[i].total_cmp(&logits[j]))
                        .ok_or("model has no legal action")?;
                    let best = if temperature > 0.0 {
                        let peak = logits[best] as f64;
                        let weights: Vec<f64> = eligible
                            .iter()
                            .map(|&i| ((logits[i] as f64 - peak) / temperature).exp())
                            .collect();
                        let total: f64 = weights.iter().sum();
                        let mut draw = policy_rng.next_f64() * total;
                        let mut selected = weights.len() - 1;
                        for (i, &weight) in weights.iter().enumerate() {
                            draw -= weight;
                            if draw < 0.0 {
                                selected = i;
                                break;
                            }
                        }
                        log_prob = Some((weights[selected] / total).ln());
                        eligible[selected]
                    } else {
                        best
                    };
                    encoded.actions[best].input.clone()
                } else {
                    agent.step(&dex, iterations)?;
                    agent.best(&dex).ok_or("no selected action")?
                };
                if !frame.legal_actions.contains(&action) {
                    return Err(format!("selected illegal action {action}").into());
                }
                let elapsed_ns = started.elapsed().as_nanos() as u64;
                let mut response = json!({
                    "action": action,
                    "elapsed_ns": elapsed_ns,
                    "iterations": agent.iterations(),
                    "legality_drift": agent.legality_drift,
                    "projections": agent.projections,
                });
                if features {
                    response["observation"] = serde_json::to_value(encoded)?;
                }
                if let Some((logits, value)) = prediction {
                    response["logits"] = serde_json::to_value(logits)?;
                    response["value"] = json!(value);
                    response["log_prob"] = json!(log_prob);
                    response["temperature"] = json!(temperature);
                }
                response
            }
        };
        serde_json::to_writer(&mut output, &response)?;
        writeln!(output)?;
        output.flush()?;
    }
    Ok(())
}
