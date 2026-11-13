use conformance::fixture::{corpus_files, repo_root, Fixture};
use conformance::load_dex;
use nc2000_engine::dex::Dex;
use std::sync::OnceLock;

pub fn dex() -> &'static Dex {
    static DEX: OnceLock<Dex> = OnceLock::new();
    DEX.get_or_init(load_dex)
}

pub fn puredata_full_fixtures() -> &'static [Fixture] {
    static FIXTURES: OnceLock<Vec<Fixture>> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        let root = repo_root().join("fixtures/corpus-v1");
        let mut out = Vec::new();
        for corpus in ["puredata", "full"] {
            for path in corpus_files(&root.join(corpus)) {
                out.push(Fixture::load(&path).unwrap());
            }
        }
        assert!(!out.is_empty());
        out
    })
}
