#!/usr/bin/env python3
"""Validate a complete fresh prior-preview bake before activating its tables."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess

ap = argparse.ArgumentParser(description=__doc__)
ap.add_argument('bake', type=Path)
ap.add_argument('--check-only', action='store_true')
args = ap.parse_args()
sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
artifact_path = args.bake / 'compiled-preview-v1.json'
matrix_path = args.bake / 'preview-nash-matrix.json'
artifact = json.loads(artifact_path.read_text())
matrix = json.loads(matrix_path.read_text())
manifest = json.loads((args.bake / 'manifest.json').read_text())
basis = json.loads((args.bake / 'basis.json').read_text())
catalog = json.loads(Path('data/team-pool-v2/team-pool.json').read_text())['teams']
assert artifact['format'] == 'nc2000-compiled-preview-v1'
assert 'status' not in artifact and 'retirement' not in artifact
assert artifact['computation']['initialization'] == 'native-engine-team-preview'
assert artifact['inputs'] == matrix['inputs'] == basis['inputs']
for path, expected in artifact['inputs'].items():
    assert sha(path) == expected, f'Stale bake input: {path}'
assert artifact['priorSha256'] == sha('data/belief-pool-v3/belief-pool.json')
assert artifact['dexSha256'] == sha('data/gen2stadium2.json')
assert artifact['c'] == json.loads(Path('data/search-profiles.json').read_text())['blind']['c']
assert manifest['artifactSha256'] == sha(artifact_path)
assert manifest['basisSha256'] == sha(args.bake / 'basis.json')
assert artifact['jobsSha256'] == matrix['basisSha256'] == manifest['basisSha256']
build = subprocess.check_output(['cargo', 'run', '--quiet', '--release', '-p', 'nc2000-bot',
                                '--example', 'compile_preview', '--', '--identity'], text=True).strip()
assert artifact['build'] == build, 'Stale bake engine identity'
own_keys, public_keys = set(), set()
for key, choices in artifact['policies'].items():
    own, foe = json.loads(key)
    assert len(own) == len(foe) == 6 and all(len(p) == 2 for p in foe)
    own_keys.add(json.dumps(own))
    public_keys.add(json.dumps(foe))
    assert len(choices) == 64
    for choice in choices:
        match = re.fullmatch(r'team (\d+),\s*(\d+),\s*(\d+)', choice)
        assert match, f'Invalid table choice: {choice}'
        slots = [int(v) - 1 for v in match.groups()]
        assert len(set(slots)) == 3 and all(0 <= v < 6 for v in slots)
        assert sum(own[v][1] for v in slots) <= 155
assert len(own_keys) == len(catalog)
assert len(artifact['policies']) == artifact['matchups'] == len(own_keys) * len(public_keys)
assert manifest['catalogParties'] == len(catalog) and manifest['publicRosters'] == len(public_keys)
assert len(matrix['previewPolicies']) == len(catalog)
assert all(len(v) == len(catalog) for v in matrix['previewPolicies'].values())
if not args.check_only:
    for source, target in [
        (artifact_path, Path('data/compiled-preview-v1.json')),
        (matrix_path, Path('data/team-pool-v2/preview-nash-matrix.json')),
        (args.bake / 'basis.json', Path('data/preview-training-v3/basis.json')),
        (args.bake / 'manifest.json', Path('data/preview-training-v3/bake-manifest.json')),
    ]:
        target.parent.mkdir(parents=True, exist_ok=True)
        temp = target.with_suffix('.tmp')
        shutil.copyfile(source, temp)
        temp.replace(target)
print(f"{'Verified' if args.check_only else 'Activated'} {artifact['matchups']} policies; "
      f"{len(own_keys)} own parties x {len(public_keys)} public rosters")
