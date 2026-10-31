#!/usr/bin/env python3

import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "data/compiled-preview-v1.json"
OUTPUT = ROOT / "data/compiled-preview-fork-attestation-v1.json"

# These are the exact reviewed fork-local diagnostic changes.
# If either file changes again, attestation must stop until the new change
# has been reviewed explicitly.
APPROVED_FORK_INPUTS = {
    "crates/bot/src/belief.rs":
        "bbec1613b77c0d94cf91c92bcd03e30b2ccf9976b6ba8516fadbd9c7ff1e4140",
    "crates/bot/src/import.rs":
        "7bcc288105c3c81dd01e48be346a5647e449aab2736a322a56144b02ef2f81b7",
}


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def sha256_file(path):
    return sha256_bytes(path.read_bytes())


artifact_bytes = ARTIFACT.read_bytes()
artifact = json.loads(artifact_bytes)

if artifact.get("format") != "nc2000-compiled-preview-v1":
    raise SystemExit("unexpected compiled-preview format")

inputs = artifact.get("inputs")
policies = artifact.get("policies")

if not isinstance(inputs, dict):
    raise SystemExit("compiled-preview inputs missing")

if not isinstance(policies, dict):
    raise SystemExit("compiled-preview policies missing")

changed = {}

for rel, expected in inputs.items():
    path = ROOT / rel

    if not path.is_file():
        raise SystemExit(f"missing artifact input: {rel}")

    current = sha256_file(path)

    if current != expected:
        changed[rel] = {
            "artifact": expected,
            "current": current,
        }

actual_changed = set(changed)
approved_changed = set(APPROVED_FORK_INPUTS)

if actual_changed != approved_changed:
    print("EXPECTED_CHANGED_INPUTS:")
    for name in sorted(approved_changed):
        print(f"  {name}")

    print("ACTUAL_CHANGED_INPUTS:")
    for name in sorted(actual_changed):
        print(f"  {name}")

    raise SystemExit(
        "refusing attestation: source delta is not exactly the reviewed "
        "fork-local diagnostic delta"
    )

for rel, approved_digest in APPROVED_FORK_INPUTS.items():
    current_digest = changed[rel]["current"]

    if current_digest != approved_digest:
        raise SystemExit(
            f"refusing attestation: reviewed fork input changed: {rel}\n"
            f"approved={approved_digest}\n"
            f"current ={current_digest}"
        )

current_build = subprocess.check_output(
    [
        "cargo",
        "run",
        "--quiet",
        "--release",
        "-p",
        "nc2000-bot",
        "--example",
        "compile_preview",
        "--",
        "--identity",
    ],
    cwd=ROOT,
    text=True,
).strip()

policy_bytes = json.dumps(
    policies,
    separators=(",", ":"),
    ensure_ascii=False,
).encode("utf-8")

attestation = {
    "format": "nc2000-compiled-preview-fork-attestation-v1",
    "kind": "diagnostic-only-source-delta-v1",
    "artifactSha256": sha256_bytes(artifact_bytes),
    "artifactBuild": artifact["build"],
    "currentBuild": current_build,
    "policySha256": sha256_bytes(policy_bytes),
    "policyCount": len(policies),
    "changedInputs": changed,
    "note": (
        "Fork-local compatibility attestation. "
        "The upstream compiled-preview artifact is kept byte-for-byte "
        "unmodified. Only the exact reviewed belief/import diagnostic "
        "delta is permitted."
    ),
}

OUTPUT.write_text(
    json.dumps(attestation, indent=2, ensure_ascii=False) + "\n"
)

print(f"ARTIFACT_SHA256={attestation['artifactSha256']}")
print(f"ARTIFACT_BUILD={artifact['build']}")
print(f"CURRENT_BUILD={current_build}")
print(f"POLICY_COUNT={len(policies)}")
print(f"POLICY_SHA256={attestation['policySha256']}")

print("CHANGED_INPUTS:")
for name in sorted(changed):
    print(f"  {name}")

print("FORK_PREVIEW_REATTEST=PASS")
