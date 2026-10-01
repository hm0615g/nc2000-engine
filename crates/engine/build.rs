use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn hash_sources(path: &Path, hash: &mut Sha256) {
    let mut entries: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for entry in entries {
        if entry.is_dir() {
            hash_sources(&entry, hash);
        } else if entry.extension().is_some_and(|e| e == "rs") {
            println!("cargo:rerun-if-changed={}", entry.display());
            hash.update(entry.to_string_lossy().as_bytes());
            hash.update(fs::read(&entry).unwrap());
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let mut hash = Sha256::new();
    hash_sources(Path::new("src"), &mut hash);
    let digest = hash.finalize();
    let output = format!(
        "pub const ENGINE_FINGERPRINT: [u8; 16] = {:?};",
        &digest[..16]
    );
    fs::write(
        Path::new(&std::env::var("OUT_DIR").unwrap()).join("replay_version.rs"),
        output,
    )
    .unwrap();
}
