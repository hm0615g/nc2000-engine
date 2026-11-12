use conformance::fixture::repo_root;
use nc2000_bot::preview::{load_meta_pool, MetaPool};
use nc2000_engine::dex::Dex;
use nc2000_engine::validate::Learnsets;
use serde_json::Value;
use std::sync::{Arc, OnceLock};

pub fn dex() -> &'static Dex {
    static DEX: OnceLock<Dex> = OnceLock::new();
    DEX.get_or_init(conformance::load_dex)
}

pub fn learnsets() -> &'static Learnsets {
    static LEARNSETS: OnceLock<Learnsets> = OnceLock::new();
    LEARNSETS.get_or_init(|| {
        let text = std::fs::read_to_string(repo_root().join("data/learnsets-gen2.json")).unwrap();
        Learnsets::from_json(&text).unwrap()
    })
}

pub fn meta_pool_v0_arc() -> &'static Arc<MetaPool> {
    static POOL: OnceLock<Arc<MetaPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        Arc::new(load_meta_pool(
            &repo_root().join("data/meta-pool-v0/meta-pool.json"),
        ))
    })
}

pub fn meta_pool_v0() -> &'static MetaPool {
    meta_pool_v0_arc().as_ref()
}

pub fn belief_pool_v3() -> &'static MetaPool {
    static POOL: OnceLock<MetaPool> = OnceLock::new();
    POOL.get_or_init(|| load_meta_pool(&repo_root().join("data/belief-pool-v3/belief-pool.json")))
}

pub fn team_pool_v2() -> &'static MetaPool {
    static POOL: OnceLock<MetaPool> = OnceLock::new();
    POOL.get_or_init(|| load_meta_pool(&repo_root().join("data/team-pool-v2/team-pool.json")))
}

pub fn corpus_json_values() -> &'static [(String, Value)] {
    static CORPUS: OnceLock<Vec<(String, Value)>> = OnceLock::new();
    CORPUS.get_or_init(|| {
        let mut out = Vec::new();
        for pool_dir in ["full", "puredata"] {
            let dir = repo_root().join("fixtures/corpus-v1").join(pool_dir);
            let mut files: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().map(|x| x == "json").unwrap_or(false)
                        && !p.to_string_lossy().contains("DIVERGED")
                })
                .collect();
            files.sort();
            for f in files {
                let name = f.file_name().unwrap().to_string_lossy().into_owned();
                let val: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
                out.push((name, val));
            }
        }
        out
    })
}
