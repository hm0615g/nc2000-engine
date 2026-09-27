#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
run_dir=data/party-vs-nash-20260927
cargo build --release -p nc2000-bot --example prior_exploit -j 4
target/release/examples/prior_exploit --arm pool:data/belief-pool-v1/belief-pool.json \
    --y "file:$run_dir/party.json" --validate-only > "$run_dir/native-validation.json"
python3 - <<'PY'
import hashlib, json, pathlib, subprocess
root = pathlib.Path('.')
out = root / 'data/party-vs-nash-20260927'
files = sorted(root.glob('crates/*/src/**/*.rs')) + [
    root / 'Cargo.lock', root / 'Cargo.toml', root / 'crates/bot/examples/prior_exploit.rs',
    out / 'party.json', root / 'data/meta-nash-v1/pool-artifact.json',
    root / 'data/belief-pool-v1/belief-pool.json', root / 'data/gen2stadium2.json',
    root / 'data/learnsets-gen2.json', root / 'target/release/examples/prior_exploit',
]
fingerprints = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}
manifest = out / 'manifest.json'
if manifest.exists():
    assert json.loads(manifest.read_text())['sha256'] == fingerprints, 'Inputs or binary changed; use a new output directory'
else:
    manifest.write_text(json.dumps({
        'source_commit': subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
        'source_diff': subprocess.check_output(['git','diff','--','crates'],text=True),
        'sha256': fingerprints,
        'information': 'both blind; same shipped belief pool; no baked tables or pondering',
        'party_change': 'Misdreavus: Miracle Berry -> Mint Berry, user approved',
        'runs': [{'label':'main-i3000','games':1024,'iters':3000,'seed':2026092701},
                 {'label':'confirm-i10000','games':256,'iters':10000,'seed':2026092702}],
    },indent=2)+'\n')
PY
for row in 'main-i3000 1024 3000 2026092701' 'confirm-i10000 256 10000 2026092702'; do
    read -r label games iters seed <<< "$row"
    target/release/examples/prior_exploit \
        --arm pool:data/belief-pool-v1/belief-pool.json \
        --y "file:$run_dir/party.json" --y-blind --threads 4 \
        --games "$games" --iters "$iters" --seed "$seed" \
        --out "$run_dir/results" --label "$label" 2>&1 | tee "$run_dir/$label.log"
done
python3 "$run_dir/summarize.py" "$run_dir/results/main-i3000.jsonl" \
    "$run_dir/results/confirm-i10000.jsonl" > "$run_dir/summary.json"
