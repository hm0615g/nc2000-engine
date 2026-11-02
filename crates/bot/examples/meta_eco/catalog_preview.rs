//! Prior-conditioned catalog preview bake. Preserve the actual trained sets.
//! Every output is keyed by own full set and opponent public roster only.
use super::{
    compute_raw_to_canon_action_map, flag, legal_triples_sets, num, preview_sig_sets,
    shell_pair_crn_seed, solve_debiased_bayesian_shell_preview_nash, transpose_preview_matrix,
    CompiledRoster6, PreviewSig,
};
use nc2000_bot::{compiled_preview, preview::MetaPool, Belief, Observer, SplitMix64};
use nc2000_engine::{
    battle::{PokemonSet, SearchChoice},
    dex::{toid, Dex},
    state::Battle,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn file_sha(p: &str) -> String {
    sha(&std::fs::read(p).unwrap())
}
fn write(p: &Path, v: &Value) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(v).unwrap()).unwrap();
    std::fs::rename(tmp, p).unwrap();
}
fn canonical(sets: &[PokemonSet]) -> Vec<PokemonSet> {
    let mut out = sets.to_vec();
    out.sort_by_key(|s| (std::cmp::Reverse(s.level), toid(&s.species)));
    out
}
fn material(dex: &Dex, sets: &[PokemonSet]) -> String {
    let b = Battle::from_fixture(dex, "1,2,3,4", sets, sets).unwrap();
    let key: Value = serde_json::from_str(&compiled_preview::key(&b, 0).unwrap()).unwrap();
    key[0].to_string()
}
fn source_inputs() -> BTreeMap<String, String> {
    let mut paths = vec![
        "Cargo.toml".into(),
        "Cargo.lock".into(),
        "data/search-profiles.json".into(),
        "data/gen2stadium2.json".into(),
        "data/learnsets-gen2.json".into(),
        "data/belief-pool-v3/belief-pool.json".into(),
        "data/team-pool-v2/team-pool.json".into(),
        "data/training-v3/rentals.json".into(),
        "data/training-v3/supplemental.json".into(),
        "crates/bot/Cargo.toml".into(),
        "crates/bot/build.rs".into(),
        "crates/engine/Cargo.toml".into(),
        "crates/engine/build.rs".into(),
        "crates/bot/examples/meta_eco.rs".into(),
        "crates/bot/examples/meta_eco/catalog_preview.rs".into(),
    ];
    fn collect(dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                collect(&p, out)
            } else if p.extension().is_some_and(|s| s == "rs") {
                out.push(p.to_str().unwrap().into());
            }
        }
    }
    for dir in [
        "crates/bot/src",
        "crates/engine/src",
        "crates/conformance/src",
    ] {
        collect(Path::new(dir), &mut paths);
    }
    paths
        .into_iter()
        .map(|p| {
            let digest = file_sha(&p);
            (p, digest)
        })
        .collect()
}

fn initial_state(
    dex: &Dex,
    template: &Battle,
    a: [usize; 3],
    b: [usize; 3],
    seed: u64,
) -> (
    nc2000_engine::compact::MatchTable,
    nc2000_engine::compact::CompactBattle,
) {
    let mut battle = template.clone();
    battle.reseed(seed);
    battle
        .apply_choices(
            dex,
            [
                Some(SearchChoice::Team(a.map(|i| i as u8 + 1))),
                Some(SearchChoice::Team(b.map(|i| i as u8 + 1))),
            ],
        )
        .unwrap();
    nc2000_engine::compact::CompactBattle::from_battle(&battle, dex)
}

fn evaluate_matrix(
    dex: &Dex,
    a: &CompiledRoster6,
    b: &CompiledRoster6,
    ns: u32,
    max_turns: u16,
    base_seed: u64,
) -> Vec<f64> {
    let mut ab = Battle::from_fixture(dex, "1,2,3,4", &a.sets, &b.sets).unwrap();
    ab.set_log_enabled(false);
    ab.log.clear();
    let mut ba = Battle::from_fixture(dex, "1,2,3,4", &b.sets, &a.sets).unwrap();
    ba.set_log_enabled(false);
    ba.log.clear();
    let forward = preview_sig_sets(&a.sets) <= preview_sig_sets(&b.sets);
    let mut matrix = vec![0.0; a.legal.len() * b.legal.len()];
    let playout = nc2000_bot::Playout::heavy();
    let rules = nc2000_bot::mcts::RolloutRules::default();
    for (ia, &ta) in a.legal.iter().enumerate() {
        for (ib, &tb) in b.legal.iter().enumerate() {
            let (x, y) = if forward { (ia, ib) } else { (ib, ia) };
            let mut value = 0.0;
            for s in 0..ns {
                let seed = base_seed
                    ^ u64::from(s).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    ^ ((x * 64 + y) as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                let (ra, rb) = if forward {
                    (seed ^ 0xA5A5_5A5A_F0F0_0F0F, seed ^ 0x5A5A_A5A5_0F0F_F0F0)
                } else {
                    (seed ^ 0x5A5A_A5A5_0F0F_F0F0, seed ^ 0xA5A5_5A5A_F0F0_0F0F)
                };
                let (table, mut state) = initial_state(dex, &ab, ta, tb, seed | 1);
                let va = nc2000_bot::mcts::compact_playout_value(
                    &mut state,
                    &table,
                    &playout,
                    max_turns,
                    &mut SplitMix64::new(ra),
                    rules,
                );
                let (table, mut state) = initial_state(dex, &ba, tb, ta, seed | 1);
                let vb = nc2000_bot::mcts::compact_playout_value(
                    &mut state,
                    &table,
                    &playout,
                    max_turns,
                    &mut SplitMix64::new(rb),
                    rules,
                );
                value += 0.5 * (va + 1.0 - vb);
            }
            matrix[ia * b.legal.len() + ib] = value / f64::from(ns);
        }
    }
    matrix
}
struct Type {
    ids: Vec<String>,
    weight: f64,
    identity: String,
    roster: CompiledRoster6,
    shell: usize,
}
struct Shell {
    sig: PreviewSig,
    types: Vec<usize>,
    weights: Vec<f64>,
    apex: bool,
}
struct Catalog {
    id: String,
    sets: Vec<PokemonSet>,
    ty: usize,
    shell: usize,
    apex: bool,
    canonical_to_raw: Vec<usize>,
    legal: Vec<[usize; 3]>,
}

fn quantize(p: &[f64], ev: &[f64], legal: &[[usize; 3]]) -> Vec<String> {
    assert_eq!(p.len(), legal.len());
    assert!(p.iter().all(|v| v.is_finite() && *v >= 0.0));
    assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    let mut counts: Vec<usize> = p.iter().map(|v| (v * 64.0).floor() as usize).collect();
    let used: usize = counts.iter().sum();
    assert!(used <= 64);
    let mut order: Vec<_> = (0..p.len()).collect();
    order.sort_by(|&a, &b| {
        (p[b] * 64.0 - counts[b] as f64)
            .total_cmp(&(p[a] * 64.0 - counts[a] as f64))
            .then(ev[b].total_cmp(&ev[a]))
            .then(a.cmp(&b))
    });
    for &i in order.iter().take(64 - used) {
        counts[i] += 1;
    }
    order.sort_by(|&a, &b| p[b].total_cmp(&p[a]).then(a.cmp(&b)));
    let mut out = vec![];
    for i in order {
        let t = legal[i];
        for _ in 0..counts[i] {
            out.push(format!("team {}, {}, {}", t[0] + 1, t[1] + 1, t[2] + 1));
        }
    }
    assert_eq!(out.len(), 64);
    out
}

// Retain the existing ordinary-party near-optimal diversity policy. APEX
// variants use the solved Bayesian policy without this additional mixture.
fn casual(p: &[f64], ev: &[f64], legal: &[[usize; 3]]) -> Vec<f64> {
    let value: f64 = p.iter().zip(ev).map(|(p, v)| p * v).sum();
    let mut near: Vec<f64> = ev
        .iter()
        .map(|v| {
            if value - v <= 0.045 {
                (-((value - v).max(0.0)) / 0.022).exp()
            } else {
                0.0
            }
        })
        .collect();
    let sum: f64 = near.iter().sum();
    if sum > 0.0 {
        for v in &mut near {
            *v /= sum
        }
    } else {
        near = p.to_vec()
    }
    let mut mix: Vec<f64> = p
        .iter()
        .zip(near)
        .map(|(p, n)| 0.78 * p + 0.22 * n)
        .collect();
    for slot in 0..6 {
        let eligible: Vec<usize> = legal
            .iter()
            .enumerate()
            .filter(|(_, t)| t.contains(&slot))
            .map(|(i, _)| i)
            .collect();
        let cov: f64 = eligible.iter().map(|&i| mix[i]).sum();
        if let Some(&best) = eligible
            .iter()
            .max_by(|&&a, &&b| ev[a].total_cmp(&ev[b]).then(b.cmp(&a)))
        {
            if value - ev[best] <= 0.055 && cov < 0.12 {
                mix[best] += 0.12 - cov;
            }
        }
    }
    let sum: f64 = mix.iter().sum();
    for v in &mut mix {
        *v /= sum;
    }
    mix
}

fn function_source<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source.find(&format!("fn {name}(")).expect(name);
    let open = start + source[start..].find('{').unwrap();
    let mut depth=0;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        if *byte==b'{' {depth+=1;} else if *byte==b'}' {depth-=1; if depth==0 {return &source[start..=open+offset];}}
    }
    panic!("unterminated function {name}")
}

fn verify_partial_reuse(source: &Path, inputs: &BTreeMap<String,String>, revision: &str) -> Value {
    let previous: Value=serde_json::from_slice(&std::fs::read(source.join("compiled-preview-v1.json")).unwrap()).unwrap();
    let old: BTreeMap<String,String>=serde_json::from_value(previous["inputs"].clone()).unwrap();
    let allowed=["crates/bot/src/belief.rs","crates/bot/src/blind.rs","crates/bot/src/import.rs",
        "crates/bot/src/lib.rs","crates/bot/src/player.rs","crates/bot/src/player_state.rs","crates/bot/examples/meta_eco/catalog_preview.rs"];
    let mut changed=Vec::new();
    for (path,digest) in inputs {
        if old.get(path)==Some(digest) {continue;}
        assert!(allowed.contains(&path.as_str()), "rollout/Bayesian dependency changed: {path}; selective reuse refused");
        changed.push(path.clone());
    }
    for path in old.keys() {assert!(inputs.contains_key(path), "removed bake dependency: {path}");}
    let mut kernels=serde_json::Map::new();
    for (path,names) in [
        ("crates/bot/examples/meta_eco/catalog_preview.rs",vec!["initial_state","evaluate_matrix","canonical","material","casual","quantize"]),
        ("crates/bot/src/belief.rs",vec!["refilter","build_refs","consistent"]),
    ] {
        let baseline=std::process::Command::new("git").args(["show",&format!("{revision}:{path}")]).output().unwrap();
        assert!(baseline.status.success());
        assert_eq!(sha(&baseline.stdout),old[path],"baseline revision differs from previous bake");
        let baseline=String::from_utf8(baseline.stdout).unwrap();
        let current=std::fs::read_to_string(path).unwrap();
        for name in names {
            let before=function_source(&baseline,name);let after=function_source(&current,name);
            assert_eq!(before,after,"semantic preview dependency changed: {path}::{name}");
            kernels.insert(format!("{path}::{name}"),json!(sha(after.as_bytes())));
        }
    }
    json!({"sourceRevision":revision,"sourceArtifactSha256":file_sha(source.join("compiled-preview-v1.json").to_str().unwrap()),
        "sourceBuild":previous["build"],"changedInputs":changed,"unchangedKernelFunctions":kernels,
        "policy":"Recompute only Nash-involving matrices and solutions; reuse other frozen rollout results after closed dependency checks and public candidate/key validation."})
}

pub(super) fn run(args: &[String]) {
    let started = Instant::now();
    let dex = Dex::from_json(&std::fs::read_to_string("data/gen2stadium2.json").unwrap()).unwrap();
    let catalog_json: Value =
        serde_json::from_slice(&std::fs::read("data/team-pool-v2/team-pool.json").unwrap())
            .unwrap();
    let pool: MetaPool =
        serde_json::from_slice(&std::fs::read("data/belief-pool-v3/belief-pool.json").unwrap())
            .unwrap();
    let ns_apex: u32 = num(args, "--ns-apex", 3);
    let ns_other: u32 = num(args, "--ns-other", 2);
    let max_turns: u16 = num(args, "--max-turns", 250);
    let seed: u64 = num(args, "--seed", 2026110301);
    let sweeps: usize = num(args, "--sweeps", 600);
    let threads: usize = num(
        args,
        "--threads",
        std::thread::available_parallelism().unwrap().get(),
    );
    assert!(ns_apex > 0 && ns_other > 0 && threads > 0 && sweeps > 0);
    let out =
        PathBuf::from(flag(args, "--out-dir").unwrap_or_else(|| "tmp/prior-preview-v1".into()));
    std::fs::create_dir_all(&out).unwrap();
    let reuse=flag(args,"--nash-only-reuse").map(PathBuf::from);
    let revision=flag(args,"--reuse-revision").unwrap_or_else(||"97a133f6".into());
    let reuse_proof=reuse.as_ref().map(|p|verify_partial_reuse(p,&source_inputs(),&revision));
    let old_artifact: Option<Value>=reuse.as_ref().map(|p|serde_json::from_slice(&std::fs::read(p.join("compiled-preview-v1.json")).unwrap()).unwrap());
    let old_matrix: Option<Value>=reuse.as_ref().map(|p|serde_json::from_slice(&std::fs::read(p.join("preview-nash-matrix.json")).unwrap()).unwrap());
    let old_manifest: Option<Value>=reuse.as_ref().map(|p|serde_json::from_slice(&std::fs::read(p.join("manifest.json")).unwrap()).unwrap());
    let cat: Vec<(String, Vec<PokemonSet>, bool)> = catalog_json["teams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["id"].as_str().unwrap().into(),
                serde_json::from_value(t["sets"].clone()).unwrap(),
                t["nash"] == true,
            )
        })
        .collect();
    let sigs: BTreeSet<_> = cat.iter().map(|(_, s, _)| preview_sig_sets(s)).collect();
    let apex_sigs: BTreeSet<_> = cat
        .iter()
        .filter(|(_, _, a)| *a)
        .map(|(_, s, _)| preview_sig_sets(s))
        .collect();
    let mut shells: Vec<Shell> = sigs
        .iter()
        .map(|s| Shell {
            sig: s.clone(),
            types: vec![],
            weights: vec![],
            apex: apex_sigs.contains(s),
        })
        .collect();
    let shell_map: BTreeMap<_, _> = shells
        .iter()
        .enumerate()
        .map(|(i, s)| (s.sig.clone(), i))
        .collect();
    let mut types: Vec<Type> = vec![];
    let mut identities: BTreeMap<String, usize> = BTreeMap::new();
    for t in &pool.teams {
        let sig = preview_sig_sets(&t.sets);
        let Some(&shell) = shell_map.get(&sig) else {
            continue;
        };
        for s in &t.sets {
            for m in &s.moves {
                assert!(
                    !["beatup", "metronome", "mimic", "mirrormove", "transform"]
                        .contains(&toid(m).as_str()),
                    "ordered-bench action space required: {}",
                    t.id
                );
            }
        }
        let sets = canonical(&t.sets);
        let identity = material(&dex, &sets);
        let w = t.weight.unwrap_or(1.0);
        assert!(w.is_finite() && w > 0.0);
        if let Some(&i) = identities.get(&identity) {
            types[i].ids.push(t.id.clone());
            types[i].weight += w;
        } else {
            let i = types.len();
            identities.insert(identity.clone(), i);
            shells[shell].types.push(i);
            types.push(Type {
                ids: vec![t.id.clone()],
                weight: w,
                identity,
                roster: CompiledRoster6::compile(&dex, &sets),
                shell,
            });
        }
    }
    for s in &mut shells {
        let sum: f64 = s.types.iter().map(|&i| types[i].weight).sum();
        assert!(sum > 0.0);
        s.weights = s.types.iter().map(|&i| types[i].weight / sum).collect();
    }
    let catalog: Vec<Catalog> = cat
        .into_iter()
        .map(|(id, sets, apex)| {
            let canonical_sets = canonical(&sets);
            let ty = *identities
                .get(&material(&dex, &canonical_sets))
                .expect("catalog full set absent from prior");
            let raw: Vec<Value> = sets
                .iter()
                .map(|s| serde_json::to_value(s).unwrap())
                .collect();
            Catalog {
                id,
                canonical_to_raw: compute_raw_to_canon_action_map(&raw, &canonical_sets),
                legal: legal_triples_sets(&sets),
                shell: types[ty].shell,
                sets,
                ty,
                apex,
            }
        })
        .collect();
    // Cross-check source membership against the actual runtime preview filter.
    for c in &catalog {
        let b = Battle::from_fixture(&dex, "1,2,3,4", &c.sets, &c.sets).unwrap();
        let obs = Observer::new(&b, 0);
        let belief = Belief::new(&dex, &pool, &obs);
        let runtime: BTreeSet<_> = belief
            .alive()
            .iter()
            .map(|&i| belief.candidate_id(i).to_owned())
            .collect();
        let planned: BTreeSet<_> = shells[c.shell]
            .types
            .iter()
            .flat_map(|&i| types[i].ids.clone())
            .collect();
        assert_eq!(runtime, planned, "prior mismatch: {}", c.id);
    }
    let inputs = source_inputs();
    let basis = json!({"format":"prior-preview-basis-v1","inputs":inputs,"seed":seed,"ns_apex":ns_apex,"ns_other":ns_other,"max_turns":max_turns,"sweeps":sweeps});
    if let Some(source)=&reuse {
        let before:Value=serde_json::from_slice(&std::fs::read(source.join("basis.json")).unwrap()).unwrap();
        for key in ["seed","ns_apex","ns_other","max_turns","sweeps"] {assert_eq!(before[key],basis[key],"changed partial-bake condition {key}");}
        assert_eq!(old_artifact.as_ref().unwrap()["jobsSha256"],old_manifest.as_ref().unwrap()["basisSha256"]);
        assert_eq!(old_manifest.as_ref().unwrap()["artifactSha256"],file_sha(source.join("compiled-preview-v1.json").to_str().unwrap()));
    }
    let basis_hash = sha(&serde_json::to_vec(&basis).unwrap());
    let cache = out.join(&basis_hash);
    std::fs::create_dir_all(&cache).unwrap();
    write(&out.join("basis.json"), &basis);
    let mut jobs = vec![];
    for i in 0..types.len() {
        for j in i..types.len() {
            jobs.push((
                i,
                j,
                if shells[types[i].shell].apex || shells[types[j].shell].apex {
                    ns_apex
                } else {
                    ns_other
                },
            ));
        }
    }
    let limit: usize = num(args, "--pair-limit", jobs.len());
    let run_jobs = &jobs[..jobs.len().min(limit)];
    let cursor = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let computed = AtomicUsize::new(0);
    eprintln!("[prior-preview] {} catalog parties, {} public rosters, {} prior entries / {} distinct types, {} matrices",catalog.len(),shells.len(),types.iter().map(|t|t.ids.len()).sum::<usize>(),types.len(),jobs.len());
    let matrices = std::thread::scope(|scope| {
        let handles:Vec<_>=(0..threads).map(|_|scope.spawn(||{let mut results=vec![];
            loop{let k=cursor.fetch_add(1,Ordering::Relaxed);if k>=run_jobs.len(){break}let (i,j,ns)=run_jobs[k];
                let path=cache.join(format!("pair-{i:03}-{j:03}.json"));
                let expected=json!({"basis":basis_hash,"i":i,"j":j,"ns":ns,"own":sha(types[i].identity.as_bytes()),"foe":sha(types[j].identity.as_bytes())});
                let eligible=shells[types[i].shell].apex || shells[types[j].shell].apex;
                let mat=if let Some(source)=reuse.as_ref().filter(|_|!eligible) {
                    let old_basis=old_manifest.as_ref().unwrap()["basisSha256"].as_str().unwrap();
                    let old_path=source.join(old_basis).join(format!("pair-{i:03}-{j:03}.json"));
                    let row:Value=serde_json::from_slice(&std::fs::read(&old_path).unwrap()).unwrap();
                    let mut old_expected=expected.clone();old_expected["basis"]=json!(old_basis);
                    assert_eq!(row["job"],old_expected,"wrong reused matrix {}",old_path.display());
                    write(&path,&json!({"job":expected,"matrix":row["matrix"],"reusedFromSha256":file_sha(old_path.to_str().unwrap())}));
                    serde_json::from_value(row["matrix"].clone()).unwrap()
                } else if path.is_file(){let row:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();assert_eq!(row["job"],expected);serde_json::from_value(row["matrix"].clone()).unwrap()}
                else{let pair_seed=shell_pair_crn_seed(seed,&shells[types[i].shell].sig,&shells[types[j].shell].sig,1);
                    let mut mat=evaluate_matrix(&dex,&types[i].roster,&types[j].roster,ns,max_turns,pair_seed);
                    if i==j{let n=types[i].roster.legal.len();for a in 0..n {mat[a*n+a]=0.5;for b in a+1..n{let v=0.5*(mat[a*n+b]+1.0-mat[b*n+a]);mat[a*n+b]=v;mat[b*n+a]=1.0-v;}}}
                    write(&path,&json!({"job":expected,"matrix":mat}));computed.fetch_add(1,Ordering::Relaxed);mat};
                let n=done.fetch_add(1,Ordering::Relaxed)+1;
                if n%8==0||n==run_jobs.len(){eprintln!("[prior-preview] {n}/{} matrices, {} new, {:.1}s",run_jobs.len(),computed.load(Ordering::Relaxed),started.elapsed().as_secs_f64());}
                results.push(((i,j),mat,ns));
            }results})).collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    if run_jobs.len() < jobs.len() {
        return;
    }
    let mut mats: BTreeMap<(usize, usize), (Vec<f64>, u32)> = BTreeMap::new();
    for ((i, j), mat, ns) in matrices {
        let rev = transpose_preview_matrix(
            &mat,
            types[i].roster.legal.len(),
            types[j].roster.legal.len(),
        );
        mats.insert((i, j), (mat, ns));
        mats.insert((j, i), (rev, ns));
    }
    let mut solutions: BTreeMap<(usize, usize), Vec<f64>> = BTreeMap::new();
    let mut values: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for u in 0..shells.len() {
        for v in u..shells.len() {
            let a = &shells[u];
            let b = &shells[v];
            if reuse.is_some() && !a.apex && !b.apex { continue; }

            let ar: Vec<_> = a.types.iter().map(|&i| &types[i].roster).collect();
            let br: Vec<_> = b.types.iter().map(|&i| &types[i].roster).collect();
            let pairs: Vec<_> = a
                .types
                .iter()
                .flat_map(|&i| b.types.iter().map(move |&j| (i, j)))
                .collect();
            let mm: Vec<&[f64]> = pairs.iter().map(|p| mats[p].0.as_slice()).collect();
            let ns: Vec<_> = pairs.iter().map(|p| mats[p].1).collect();
            let (_, av, bv, p, q) = solve_debiased_bayesian_shell_preview_nash(
                &ar,
                &br,
                &mm,
                &ns,
                ar[0].legal.len(),
                br[0].legal.len(),
                &a.weights,
                &b.weights,
                sweeps,
            );
            for (k, &i) in a.types.iter().enumerate() {
                solutions.insert((i, v), p[k].clone());
                values.insert((i, v), av[k]);
            }
            if u != v {
                for (k, &j) in b.types.iter().enumerate() {
                    solutions.insert((j, u), q[k].clone());
                    values.insert((j, u), bv[k]);
                }
            }
        }
    }
    eprintln!(
        "[prior-preview] Bayesian solve complete, {:.1}s",
        started.elapsed().as_secs_f64()
    );
    let mut policies = serde_json::Map::new();
    let mut preview_policies = serde_json::Map::new();
    for c in &catalog {
        let mut vs = serde_json::Map::new();
        for d in &catalog {
            let s = &shells[d.shell];
            if reuse.is_some() && !c.apex && !s.apex {
                let row=&old_matrix.as_ref().unwrap()["previewPolicies"][&c.id][&d.id];
                let planned:Vec<String>=s.types.iter().flat_map(|&i|types[i].ids.clone()).collect();
                assert_eq!(row["priorEntries"],json!(planned),"changed public candidate membership");
                let b=Battle::from_fixture(&dex,"1,2,3,4",&c.sets,&d.sets).unwrap();
                let key=compiled_preview::key(&b,0).unwrap();
                let old=&old_artifact.as_ref().unwrap()["policies"][&key];assert!(old.is_array(),"changed own/public preview key");
                policies.insert(key,old.clone());vs.insert(d.id.clone(),row.clone());
                continue;
            }
            let ka = types[c.ty].roster.legal.len();
            let mut expected = vec![0.0; ka];
            for (k, &j) in s.types.iter().enumerate() {
                let opponent = &solutions[&(j, c.shell)];
                let kb = opponent.len();
                let mat = &mats[&(c.ty, j)].0;
                for ia in 0..ka {
                    expected[ia] += s.weights[k]
                        * (0..kb)
                            .map(|ib| mat[ia * kb + ib] * opponent[ib])
                            .sum::<f64>();
                }
            }
            let can = &solutions[&(c.ty, d.shell)];
            let mut raw = vec![0.0; ka];
            let mut ev = vec![0.0; ka];
            for (ia, &ir) in c.canonical_to_raw.iter().enumerate() {
                raw[ir] = can[ia];
                ev[ir] = expected[ia];
            }
            let final_policy = if c.apex {
                raw.clone()
            } else {
                casual(&raw, &ev, &c.legal)
            };
            let choices = quantize(&final_policy, &ev, &c.legal);
            let b = Battle::from_fixture(&dex, "1,2,3,4", &c.sets, &d.sets).unwrap();
            let key = compiled_preview::key(&b, 0).unwrap();
            if let Some(old) = policies.insert(key, json!(choices)) {
                assert_eq!(old, json!(choices), "hidden opponent set affected policy");
            }
            vs.insert(d.id.clone(),json!({"priorTypeCount":s.types.len(),"priorEntries":s.types.iter().flat_map(|&i|types[i].ids.clone()).collect::<Vec<_>>(),"policy":final_policy,"bayesianPolicy":raw,"expectedPayoffByAction":ev,"quantizedChoices":choices,"rolloutGameValue":values[&(c.ty,d.shell)]}));
        }
        preview_policies.insert(c.id.clone(), Value::Object(vs));
    }
    assert_eq!(policies.len(), catalog.len() * shells.len());
    let method="Native engine team-preview initialization followed by compact rollout full legal matrices; prior-conditioned Bayesian RM+ with subteam pooling and finite-sample shrinkage. Exact trained sets retained; approximation, not a full-game equilibrium certificate.";
    let regenerated=jobs.iter().filter(|&&(i,j,_)|reuse.is_none() || shells[types[i].shell].apex || shells[types[j].shell].apex).count();
    let partial= reuse_proof.map(|proof|json!({"proof":proof,"regeneratedMatrices":regenerated,
        "reusedMatrices":jobs.len()-regenerated,"nashTeams":catalog.iter().filter(|c|c.apex).map(|c|c.id.clone()).collect::<Vec<_>>()}));
    let artifact = json!({"format":"nc2000-compiled-preview-v1","build":compiled_preview::build_fingerprint(),"c":0.4,
        "priorSha256":file_sha("data/belief-pool-v3/belief-pool.json"),"dexSha256":file_sha("data/gen2stadium2.json"),
        "matchups":policies.len(),"iterations":0,"method":method,"inputs":inputs,"jobsSha256":basis_hash,"partialRebake":partial,
        "computation":{"initialization":"native-engine-team-preview","seed":seed,"rolloutSeedsApex":ns_apex,"rolloutSeedsOther":ns_other,"maxTurns":max_turns,"solverSweeps":sweeps,"matrices":jobs.len(),"priorEntries":types.iter().map(|t|t.ids.len()).sum::<usize>(),"priorTypes":types.len()},"policies":policies});
    let matrix = json!({"version":"training-v3-prior-preview-v1","method":method,"numTeams":catalog.len(),"numShells":shells.len(),
        "inputs":inputs,"basisSha256":basis_hash,"teamIds":catalog.iter().map(|c|c.id.clone()).collect::<Vec<_>>(),"previewPolicies":preview_policies});
    write(&out.join("compiled-preview-v1.json"), &artifact);
    write(&out.join("preview-nash-matrix.json"), &matrix);
    write(
        &out.join("manifest.json"),
        &json!({"basisSha256":basis_hash,"matrices":jobs.len(),"catalogParties":catalog.len(),"publicRosters":shells.len(),"priorTypes":types.len(),"partialRebake":partial,"elapsedSeconds":started.elapsed().as_secs_f64(),"artifactSha256":file_sha(out.join("compiled-preview-v1.json").to_str().unwrap())}),
    );
    eprintln!(
        "[prior-preview] wrote {} complete policies to {} in {:.1}s",
        policies.len(),
        out.display(),
        started.elapsed().as_secs_f64()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantization_is_normalized_and_deterministic() {
        let legal = vec![[0, 1, 2], [1, 0, 2], [2, 0, 1]];
        let p = [0.3, 0.25, 0.45];
        let ev = [0.5; 3];
        let a = quantize(&p, &ev, &legal);
        assert_eq!(a.len(), 64);
        assert_eq!(a, quantize(&p, &ev, &legal));
        assert_eq!(a.iter().filter(|s| *s == "team 1, 2, 3").count(), 19);
    }

    #[test]
    fn canonicalization_preserves_actual_trained_stats() {
        let dex = Dex::from_json(include_str!("../../../../data/gen2stadium2.json")).unwrap();
        let pool: MetaPool =
            serde_json::from_str(include_str!("../../../../data/team-pool-v2/team-pool.json"))
                .unwrap();
        for t in pool.teams {
            let original = Battle::from_fixture(&dex, "1,2,3,4", &t.sets, &t.sets).unwrap();
            let sets = canonical(&t.sets);
            let sorted = Battle::from_fixture(&dex, "1,2,3,4", &sets, &sets).unwrap();
            for p in &original.sides[0].roster {
                let q = sorted.sides[0]
                    .roster
                    .iter()
                    .find(|q| q.species == p.species)
                    .unwrap();
                assert_eq!(p.set_ivs, q.set_ivs);
                assert_eq!(p.set_evs, q.set_evs);
                assert_eq!(p.gender, q.gender);
                assert_eq!(p.happiness, q.happiness);
                assert_eq!(p.base_stored_stats, q.base_stored_stats);
                assert_eq!(p.base_maxhp, q.base_maxhp);
                assert_eq!(p.base_hp_type, q.base_hp_type);
                assert_eq!(p.base_hp_power, q.base_hp_power);
            }
        }
    }

    #[test]
    fn physical_pooling_does_not_merge_happiness_or_move_order() {
        let pool: MetaPool =
            serde_json::from_str(include_str!("../../../../data/team-pool-v2/team-pool.json"))
                .unwrap();
        let p = &pool.teams[0].sets[0];
        let mut q = p.clone();
        q.happiness = Some(255 - p.happiness.unwrap_or(255));
        assert_ne!(
            super::super::mon_set_fingerprint(p),
            super::super::mon_set_fingerprint(&q)
        );
        q = p.clone();
        q.moves.swap(0, 1);
        assert_ne!(
            super::super::mon_set_fingerprint(p),
            super::super::mon_set_fingerprint(&q)
        );
    }

    #[test]
    fn bayesian_policy_responds_to_hidden_type_prior_weights() {
        let first = [1.0, 0.0];
        let second = [0.0, 1.0];
        let mats: Vec<&[f64]> = vec![&first, &second];
        let (_, _, _, p, _) =
            super::super::solve_bayesian_shell_preview_nash(&mats, 2, 1, &[1.0], &[0.9, 0.1], 600);
        let (_, _, _, q, _) =
            super::super::solve_bayesian_shell_preview_nash(&mats, 2, 1, &[1.0], &[0.1, 0.9], 600);
        assert!(p[0][0] > 0.99);
        assert!(q[0][1] > 0.99);
    }

    #[test]
    fn berserk_gene_is_not_preconsumed_on_the_bench() {
        let dex = Dex::from_json(include_str!("../../../../data/gen2stadium2.json")).unwrap();
        let pool: MetaPool =
            serde_json::from_str(include_str!("../../../../data/team-pool-v2/team-pool.json"))
                .unwrap();
        let sets = &pool
            .teams
            .iter()
            .find(|t| t.id == "sample-07")
            .unwrap()
            .sets;
        let mut template = Battle::from_fixture(&dex, "1,2,3,4", sets, sets).unwrap();
        template.set_log_enabled(false);
        let (_, bench) = initial_state(&dex, &template, [0, 1, 2], [0, 1, 2], 17);
        assert_ne!(bench.mons[1].item, 0);
        assert_eq!(bench.mons[1].boosts[0], 0);
        assert_eq!(bench.mons[1].vol_flags, 0);
        let (_, lead) = initial_state(&dex, &template, [1, 0, 2], [0, 1, 2], 17);
        assert_eq!(lead.mons[0].item, 0);
        assert_eq!(lead.mons[0].boosts[0], 2);
        assert_ne!(lead.mons[0].vol_flags, 0);
    }
}
