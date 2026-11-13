use nc2000_engine::dex::Dex;
use std::sync::OnceLock;

pub fn dex() -> &'static Dex {
    static DEX: OnceLock<Dex> = OnceLock::new();
    DEX.get_or_init(|| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/gen2stadium2.json");
        Dex::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
    })
}
