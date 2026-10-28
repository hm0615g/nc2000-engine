use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::Arc;

use nc2000_bot::compiled_preview;
use nc2000_bot::preview::MetaPool;
use nc2000_bot::smmcts::SelRule;
use nc2000_bot::{BeliefPrior, ProtocolAgent, RmConfig};
use nc2000_engine::battle::{Outcome, PokemonSet};
use nc2000_engine::dex::Dex;
use nc2000_engine::state::{
    Battle, Pokemon, RequestKind, Status, BOOST_NAMES,
};
use serde_json::{json, Value};

const PROTOCOL_VERSION: u32 = 1;

struct PreviewIdentity {
    config: String,
    pool: Value,
    prior: Value,
}

impl PreviewIdentity {
    fn new(cfg: &RmConfig, pool_json: &str) -> Result<Self, String> {
        let pool = serde_json::from_str(pool_json)
            .map_err(|e| format!("pool identity JSON: {e}"))?;

        Ok(Self {
            config: format!("{cfg:?}"),
            pool,
            prior: Value::Null,
        })
    }

    fn set_prior(&mut self, text: &str) -> Result<(), String> {
        self.prior = serde_json::from_str(text)
            .map_err(|e| format!("prior identity JSON: {e}"))?;
        Ok(())
    }

    fn info(&self, agent: &ProtocolAgent) -> Option<Value> {
        let search = agent.search()?;
        if !search.is_preview() {
            return None;
        }

        let battle = agent.battle()?;
        let observer = agent.observer()?;

        let team_key =
            compiled_preview::key(battle, observer.side())?;

        let public: Vec<_> = observer
            .mons()
            .iter()
            .map(|m| {
                json!([
                    m.species.0,
                    m.level,
                    format!("{:?}", m.gender),
                    m.preview_has_item,
                    Value::Null,
                    m.revealed_moves
                        .iter()
                        .map(|x| x.0)
                        .collect::<Vec<_>>(),
                    format!("{:?}", m.item),
                    m.appeared,
                ])
            })
            .collect();

        Some(json!({
            "format": "nc2000-preview-cache-info-v1",
            "build": compiled_preview::build_fingerprint(),
            "driver": "protocol-native",
            "config": self.config,
            "pool": self.pool,
            "prior": self.prior,
            "teamKey": team_key,
            "opponentPublic": public,
        }))
    }
}

struct Session {
    agent: ProtocolAgent,
    preview: PreviewIdentity,
    pinned: bool,
}

fn prior_report(
    prior: &BeliefPrior,
    applied: bool,
    refusals: &[String],
) -> Value {
    let mut warnings = refusals.to_vec();
    warnings.extend(prior.warnings().iter().cloned());

    json!({
        "applied": applied,
        "species": prior.len(),
        "meanMoveSum": prior.mean_move_sum(),
        "skipped": prior.skipped(),
        "warnings": warnings,
    })
}

fn request_str(r: Option<RequestKind>) -> &'static str {
    match r {
        Some(RequestKind::TeamPreview) => "teampreview",
        Some(RequestKind::Move) => "move",
        Some(RequestKind::Switch) => "switch",
        Some(RequestKind::Wait) | None => "",
    }
}

fn status_str(s: Status) -> &'static str {
    match s {
        Status::None => "",
        other => other.as_str(),
    }
}

fn poke_json(dex: &Dex, p: &Pokemon) -> Value {
    let mut boosts = serde_json::Map::new();

    for (&b, name) in p.boosts.iter().zip(BOOST_NAMES) {
        boosts.insert(
            name.to_string(),
            json!(b),
        );
    }

    let moves: Vec<Value> = p
        .move_slots
        .iter()
        .map(|s| {
            json!({
                "id": dex.moves.key(s.id),
                "name": dex.move_static(s.id).name,
                "pp": s.pp,
                "maxpp": s.maxpp,
                "disabled": s.disabled,
            })
        })
        .collect();

    let types: Vec<&str> = p
        .types
        .iter()
        .map(|t| dex.type_name(t))
        .collect();

    let volatiles: Vec<&str> = p
        .volatiles
        .iter()
        .map(|(id, _)| dex.conds_key(*id))
        .collect();

    json!({
        "species": dex.species.get(p.species).name,
        "name": p.name.as_str(),
        "level": p.level,
        "gender": p.gender.as_str(),
        "hp": p.hp.max(0),
        "maxhp": p.maxhp,
        "fainted": p.fainted,
        "status": status_str(p.status),
        "boosts": boosts,
        "moves": moves,
        "item": p.item.map(|it| dex.items.get(it).name.clone()),
        "types": types,
        "volatiles": volatiles,
        "trapped": p.trapped,
    })
}

fn state_view_json(
    battle: &Battle,
    dex: &Dex,
) -> Value {
    let sides: Vec<Value> = (0..2)
        .map(|n| {
            let side = &battle.sides[n];

            let party: Vec<Value> = side
                .party
                .iter()
                .map(|&slot| {
                    poke_json(
                        dex,
                        &side.roster[slot as usize],
                    )
                })
                .collect();

            let active = side
                .active
                .and_then(|slot| {
                    side.party
                        .iter()
                        .position(|&x| x == slot)
                });

            let conditions: Vec<&str> = side
                .side_conditions
                .iter()
                .map(|(id, _)| dex.conds_key(*id))
                .collect();

            json!({
                "name": side.name,
                "active": active,
                "party": party,
                "pokemonLeft": side.pokemon_left,
                "sideConditions": conditions,
                "request": request_str(
                    side.request_state()
                ),
            })
        })
        .collect();

    let field = json!({
        "weather": battle
            .field
            .weather
            .map(|w| dex.conds_key(w)),
        "pseudoWeather": battle
            .field
            .pseudo_weather
            .iter()
            .map(|(id, _)| dex.conds_key(*id))
            .collect::<Vec<_>>(),
    });

    json!({
        "turn": battle.turn,
        "sides": sides,
        "field": field,
        "outcome": battle.outcome().map(|o| {
            match o {
                Outcome::P1Win => "p1",
                Outcome::P2Win => "p2",
                Outcome::Tie => "tie",
            }
        }),
    })
}

fn state_view(
    session: &Session,
    dex: &Dex,
) -> Result<Value, String> {
    let battle = session
        .agent
        .battle()
        .ok_or_else(
            || "stateView before onRequest".to_string()
        )?;

    Ok(state_view_json(battle, dex))
}

fn session_id(msg: &Value) -> Result<&str, String> {
    msg.get("battle")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(
            || "missing battle id".to_string()
        )
}

fn session_mut<'a>(
    sessions: &'a mut HashMap<String, Session>,
    msg: &Value,
) -> Result<&'a mut Session, String> {
    let id = session_id(msg)?;

    sessions
        .get_mut(id)
        .ok_or_else(
            || format!("unknown battle id: {id}")
        )
}

fn session_ref<'a>(
    sessions: &'a HashMap<String, Session>,
    msg: &Value,
) -> Result<&'a Session, String> {
    let id = session_id(msg)?;

    sessions
        .get(id)
        .ok_or_else(
            || format!("unknown battle id: {id}")
        )
}

fn parse_team(
    value: &Value,
    label: &str,
) -> Result<Vec<PokemonSet>, String> {
    serde_json::from_value(value.clone())
        .map_err(|e| format!("{label}: {e}"))
}

fn request_text(msg: &Value) -> Result<String, String> {
    let request = msg
        .get("request")
        .ok_or_else(
            || "missing request".to_string()
        )?;

    match request {
        Value::String(s) => Ok(s.clone()),
        other => serde_json::to_string(other)
            .map_err(|e| {
                format!("request serialization: {e}")
            }),
    }
}

fn handle(
    msg: &Value,
    sessions: &mut HashMap<String, Session>,
    dex: &Dex,
    pool_json: &str,
) -> Result<(Value, bool), String> {
    let seq = msg
        .get("seq")
        .cloned()
        .unwrap_or(Value::Null);

    let cmd = msg
        .get("cmd")
        .and_then(Value::as_str)
        .ok_or_else(
            || "missing cmd".to_string()
        )?;

    match cmd {
        "hello" => {
            Ok((
                json!({
                    "seq": seq,
                    "ok": true,
                    "protocol":
                        "nc2000-native-protocol-v1",
                    "version": PROTOCOL_VERSION,
                    "build":
                        compiled_preview::build_fingerprint(),
                }),
                false,
            ))
        }

        "probe-prior" => {
            let text = msg
                .get("prior")
                .and_then(Value::as_str)
                .ok_or_else(
                    || "missing prior".to_string()
                )?;

            let prior =
                BeliefPrior::from_json(text);

            let mut refusals = Vec::new();

            if prior.is_empty() {
                refusals.push(
                    "no usable species in the table; today's fallback \
                     imputation is unchanged"
                        .to_string(),
                );
            }

            let applied =
                refusals.is_empty();

            let report =
                prior_report(
                    &prior,
                    applied,
                    &refusals,
                );

            Ok((
                json!({
                    "seq": seq,
                    "ok": true,
                    "priorReport": report,
                }),
                false,
            ))
        }

        "init" => {
            let id = session_id(msg)?.to_string();

            let side = msg
                .get("side")
                .and_then(Value::as_u64)
                .ok_or_else(
                    || "missing/invalid side".to_string()
                )? as usize;

            if side > 1 {
                return Err(
                    "side must be 0 or 1".to_string()
                );
            }

            let seed = msg
                .get("seed")
                .and_then(Value::as_u64)
                .ok_or_else(
                    || "missing/invalid seed".to_string()
                )?;

            if seed > u32::MAX as u64 {
                return Err(
                    "seed must fit unsigned 32-bit"
                        .to_string()
                );
            }

            let c = msg
                .get("c")
                .and_then(Value::as_f64)
                .unwrap_or(0.4);

            if !c.is_finite() || c <= 0.0 {
                return Err(
                    "c must be finite and positive"
                        .to_string()
                );
            }

            let cfg = RmConfig {
                rule: SelRule::Ucb,
                c,
                hp_buckets: 16,
                ..RmConfig::default()
            };

            let pool: MetaPool =
                serde_json::from_str(pool_json)
                    .map_err(|e| {
                        format!(
                            "belief pool JSON: {e}"
                        )
                    })?;

            let mut preview =
                PreviewIdentity::new(
                    &cfg,
                    pool_json,
                )?;

            let mut agent =
                ProtocolAgent::new(
                    dex,
                    side,
                    pool,
                    cfg,
                    seed as u32 as u64,
                );

            let own = parse_team(
                msg.get("ownTeam")
                    .ok_or_else(
                        || "missing ownTeam".to_string()
                    )?,
                "ownTeam",
            )?;

            agent.set_own_team(own);

            let mode = msg
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("blind");

            let pinned = match mode {
                "blind" => false,

                "open" => {
                    let opponent = parse_team(
                        msg.get("opponentTeam")
                            .ok_or_else(|| {
                                "open mode requires \
                                 opponentTeam"
                                    .to_string()
                            })?,
                        "opponentTeam",
                    )?;

                    agent.pin_opponent(opponent);
                    true
                }

                other => {
                    return Err(format!(
                        "unsupported mode: {other}"
                    ));
                }
            };

            let mut prior_result = Value::Null;

            if let Some(text) = msg
                .get("beliefPrior")
                .and_then(Value::as_str)
            {
                if !text.is_empty() {
                    let prior =
                        BeliefPrior::from_json(text);

                    let mut refusals = Vec::new();

                    if pinned {
                        refusals.push(
                            "open-team-sheet mode \
                             (pinOpponent): the belief \
                             is pinned to the opponent's \
                             true sets, so a prior must \
                             never be consulted; table \
                             ignored"
                                .to_string(),
                        );
                    }

                    if prior.is_empty() {
                        refusals.push(
                            "no usable species in the \
                             table; today's fallback \
                             imputation is unchanged"
                                .to_string(),
                        );
                    }

                    let applied =
                        refusals.is_empty();

                    prior_result =
                        prior_report(
                            &prior,
                            applied,
                            &refusals,
                        );

                    if applied {
                        preview.set_prior(text)?;
                        agent.set_belief_prior(
                            Arc::new(prior),
                        );
                    }
                }
            }

            sessions.insert(
                id.clone(),
                Session {
                    agent,
                    preview,
                    pinned,
                },
            );

            Ok((
                json!({
                    "seq": seq,
                    "ok": true,
                    "battle": id,
                    "build":
                        compiled_preview::build_fingerprint(),
                    "c": c,
                    "hpBuckets": 16,
                    "priorReport": prior_result,
                }),
                false,
            ))
        }

        "feed" => {
            let session =
                session_mut(sessions, msg)?;

            let lines = msg
                .get("lines")
                .and_then(Value::as_array)
                .ok_or_else(
                    || "missing lines array".to_string()
                )?;

            for line in lines {
                let line = line
                    .as_str()
                    .ok_or_else(|| {
                        "lines must contain strings"
                            .to_string()
                    })?;

                session.agent.push_line(
                    dex,
                    line,
                );
            }

            Ok((
                json!({
                    "seq": seq,
                    "ok": true,
                    "fed": lines.len(),
                }),
                false,
            ))
        }

        "request" => {
            let text = request_text(msg)?;

            let session =
                session_mut(sessions, msg)?;

            let owes = session
                .agent
                .on_request(
                    dex,
                    &text,
                )?;

            let preview_info =
                if session.pinned {
                    None
                } else {
                    session
                        .preview
                        .info(&session.agent)
                };

            let baked = session
                .agent
                .baked_preview(dex);

            Ok((
                json!({
                    "seq": seq,
                    "ok": true,
                    "owes": owes,
                    "iterations":
                        session.agent.iterations(),
                    "bakedPreview": baked,
                    "previewInfo": preview_info,
                }),
                false,
            ))
        }

        "step" => {
            let n = msg
                .get("n")
                .and_then(Value::as_u64)
                .ok_or_else(
                    || "missing/invalid n".to_string()
                )?;

            if n > u32::MAX as u64 {
                return Err(
                    "n exceeds u32".to_string()
                );
            }

            let session =
                session_mut(sessions, msg)?;

            let iterations = session
                .agent
                .step(
                    dex,
                    n as u32,
                )?;

            Ok((
                json!({
                    "seq": seq,
                    "ok": true,
                    "iterations": iterations,
                }),
                false,
            ))
        }

        "best" => {
            let session =
                session_mut(sessions, msg)?;

            let choice =
                session.agent.best(dex);

            Ok((
                json!({
                    "seq": seq,
                    "ok": true,
                    "choice": choice,
                    "iterations":
                        session.agent.iterations(),
                }),
                false,
            ))
        }

        "inspect" => {
            let session =
                session_ref(sessions, msg)?;

            let state = state_view(
                session,
                dex,
            )?;

            let policy: Value =
                serde_json::from_str(
                    &session
                        .agent
                        .root_policy(dex),
                )
                .map_err(|e| {
                    format!(
                        "rootPolicy decode: {e}"
                    )
                })?;

            let belief: Value =
                serde_json::from_str(
                    &session
                        .agent
                        .belief_info(),
                )
                .map_err(|e| {
                    format!(
                        "beliefInfo decode: {e}"
                    )
                })?;

            let preview_info =
                if session.pinned {
                    None
                } else {
                    session
                        .preview
                        .info(&session.agent)
                };

            Ok((
                json!({
                    "seq": seq,
                    "ok": true,
                    "iterations":
                        session.agent.iterations(),
                    "rootPolicy": policy,
                    "beliefInfo": belief,
                    "stateView": state,
                    "previewInfo": preview_info,
                    "bakedPreview":
                        session.agent
                            .baked_preview(dex),
                    "metrics": {
                        "legalityDrift":
                            session.agent
                                .legality_drift,
                        "projections":
                            session.agent
                                .projections,
                    },
                }),
                false,
            ))
        }

        "cancel" => {
            /*
             * Cancellation is generation-based in
             * ps-client.js. A native step command is
             * intentionally bounded to one search slice.
             * Once that slice returns, JS simply stops
             * issuing further step commands for the stale
             * generation.
             */
            let _ =
                session_ref(sessions, msg)?;

            Ok((
                json!({
                    "seq": seq,
                    "ok": true,
                    "cancelled": true,
                }),
                false,
            ))
        }

        "close" => {
            let id =
                session_id(msg)?.to_string();

            let existed =
                sessions.remove(&id).is_some();

            Ok((
                json!({
                    "seq": seq,
                    "ok": true,
                    "battle": id,
                    "existed": existed,
                }),
                false,
            ))
        }

        "shutdown" => {
            sessions.clear();

            Ok((
                json!({
                    "seq": seq,
                    "ok": true,
                    "shutdown": true,
                }),
                true,
            ))
        }

        other => Err(format!(
            "unknown command: {other}"
        )),
    }
}

fn parse_args() -> Result<(String, String), String> {
    let args: Vec<String> =
        std::env::args().collect();

    let mut dex_path =
        "data/gen2stadium2.json".to_string();

    let mut pool_path =
        "data/belief-pool-v3/belief-pool.json"
            .to_string();

    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "--dex" => {
                i += 1;
                dex_path = args
                    .get(i)
                    .ok_or_else(|| {
                        "--dex requires a path"
                            .to_string()
                    })?
                    .clone();
            }

            "--pool" => {
                i += 1;
                pool_path = args
                    .get(i)
                    .ok_or_else(|| {
                        "--pool requires a path"
                            .to_string()
                    })?
                    .clone();
            }

            other => {
                return Err(format!(
                    "unknown argument: {other}"
                ));
            }
        }

        i += 1;
    }

    Ok((dex_path, pool_path))
}

fn run() -> Result<(), String> {
    let (dex_path, pool_path) =
        parse_args()?;

    let dex_json =
        std::fs::read_to_string(&dex_path)
            .map_err(|e| {
                format!(
                    "read dex {dex_path}: {e}"
                )
            })?;

    let dex = Dex::from_json(&dex_json)
        .map_err(|e| {
            format!("parse dex: {e:?}")
        })?;

    let pool_json =
        std::fs::read_to_string(&pool_path)
            .map_err(|e| {
                format!(
                    "read pool {pool_path}: {e}"
                )
            })?;

    /*
     * Validate once at process startup.
     * Individual sessions deserialize their own MetaPool
     * so ProtocolAgent owns it independently.
     */
    let _: MetaPool =
        serde_json::from_str(&pool_json)
            .map_err(|e| {
                format!("parse pool: {e}")
            })?;

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut sessions =
        HashMap::<String, Session>::new();

    for line in stdin.lock().lines() {
        let line = line
            .map_err(|e| {
                format!("stdin: {e}")
            })?;

        if line.trim().is_empty() {
            continue;
        }

        let parsed: Result<Value, _> =
            serde_json::from_str(&line);

        let (response, stop) = match parsed {
            Ok(msg) => {
                let seq = msg
                    .get("seq")
                    .cloned()
                    .unwrap_or(Value::Null);

                match handle(
                    &msg,
                    &mut sessions,
                    &dex,
                    &pool_json,
                ) {
                    Ok(result) => result,

                    Err(error) => (
                        json!({
                            "seq": seq,
                            "ok": false,
                            "error": error,
                        }),
                        false,
                    ),
                }
            }

            Err(e) => (
                json!({
                    "seq": Value::Null,
                    "ok": false,
                    "error":
                        format!("invalid JSON: {e}"),
                }),
                false,
            ),
        };

        serde_json::to_writer(
            &mut stdout,
            &response,
        )
        .map_err(|e| {
            format!("stdout JSON: {e}")
        })?;

        stdout
            .write_all(b"\n")
            .map_err(|e| {
                format!("stdout newline: {e}")
            })?;

        stdout
            .flush()
            .map_err(|e| {
                format!("stdout flush: {e}")
            })?;

        if stop {
            break;
        }
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!(
            "native_protocol_worker: {e}"
        );
        std::process::exit(1);
    }
}
