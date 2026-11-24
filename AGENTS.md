# Agent & Contributor Guide (`nc2000-engine`)

## 1. Fast Build & Test Workflows

- **Rust workspace (fast dev/test profile in `.cargo/config.toml`)**:
  - `cargo t` (alias for `cargo test --workspace --lib --tests`, skipping 0-test doc-test discovery)
  - `cargo test -p nc2000-engine` / `cargo test -p conformance` / `cargo test -p nc2000-bot`
  - CI contract binaries preserved: `crates/engine/tests/sleep_clause.rs`, `crates/conformance/tests/kifu.rs`, `crates/conformance/tests/replay.rs`, `crates/conformance/tests/conformance_suite.rs`, and `crates/bot/tests/bot_suite.rs`.
- **WASM & Web**:
  - `crates/wasm/build.sh nodejs` + `for s in smoke determinism parity kifu fork solver endgame destiny-bond priority-heal residual-wait own-recovery-wait preview-cache; do node "crates/wasm/tests-node/$s.js"; done`
  - `crates/wasm/build.sh && cd web && npm run build && npm run test:worker && npm run test:evaluate-worker && npm run test:policy && npm run test:e2e`

## 2. Epoch & Cache Versioning

- **2-axis epoch tracking**:
  - `ENGINE_EPOCH = 1` in `crates/engine/src/replay.rs` governs compact replay (`NC2-…`) and kifu structural compatibility; bump only on breaking mechanics or replay format changes.
  - `POLICY_EPOCH = "policy-v1"` (`CASTLE_POLICY_EPOCH = "castle-policy-v1"`) in `crates/bot/src/compiled_preview.rs` governs server-side preview cache identity (`crates/wasm/src/preview_cache.rs`, `tools/preview-cache.js`) and evaluator build versioning (`web/vite.config.ts`). Browser Team Preview runs live search directly (no pre-baked preview table).
- `tools/bundle-core-sdk.js` writes `dist/nc2000-core-sdk/engine-manifest.json` (untracked).

## 3. Research & Strength Roadmap

- Playing-strength architecture, measured-null dead ends, and open `81k` frontiers: see [`docs/BOT-STRENGTH-HANDOFF.md`](docs/BOT-STRENGTH-HANDOFF.md).
- Do NOT consult gitignored local scratch/archive directories (`findings/`, `tmp/`, `.git/sota/`) for current architecture or open bottlenecks; verify claims directly against [`docs/BOT-STRENGTH-HANDOFF.md`](docs/BOT-STRENGTH-HANDOFF.md) and live code under `crates/`.

## Castle and Discord bot operation
Before any Castle bot investigation or change, read docs/CASTLE-INCIDENT-STATUS.md. After each fix, update status and tests there. Provide all executable scripts inline as a single copy-and-paste command runnable from any directory.

<!-- ascii-copy-paste-rule-v1 -->
## Mandatory ASCII-only copy-and-paste commands
- Every executable command provided to the user must be a COMPLETE, single copy-and-paste block, runnable from any working directory.
- The entire command, including heredoc content, code comments, and string literals, must use ASCII characters only.
- Never put raw Japanese or other non-ASCII text directly in pasted Python, Node.js, or shell source. Terminal encoding has repeatedly corrupted such text and caused Python SyntaxError.
- For non-ASCII file content, pass ASCII-safe encoded bytes (for example base64), then decode with explicit UTF-8 inside the script.
- Check the complete command for non-ASCII characters before presenting it; test syntax, expected output, and repeated execution when feasible.
- After every fix, update the incident status, exact tests and results, and commit/push status. Do not mark untested work as completed.

<!-- castle-fork-push-guard-v1 -->
## Fork-only push verification
- The writable repository is hm0615g/nc2000-engine on review-local-changes.
- The origin remote may refer to puniu3/nc2000-engine (upstream, read-only).
- Never assume origin is the fork and never run git push origin without verifying its URL.
- Fetch the fork first, require fast-forward ancestry, and push to the explicit fork URL without --force.
- Confirm the fork branch SHA after pushing; record the actual result in the incident status document.
- Provide the full executable command inline, ASCII-only, runnable from any directory.

<!-- no-backups-in-git-worktrees-v1 -->
## No backups in Git worktrees
- Never create .bak, .orig, .save, temporary recovery copies, or other backup files anywhere inside a Git worktree, including ignored and untracked directories.
- Do not create backups without explicit user permission. Agree on the destination before writing any backup.
- Never copy credentials or configuration secrets to a backup without explicit permission.
- Do not leave disposable files that appear as untracked entries in git status --short.
- Incident 2026-11-21: pbs-bot.config.json.bak was created inside this worktree. User requested deletion and this permanent rule.

<!-- castle-workspace-write-policy-v1 -->
## Castle diagnostic file placement
- Use only <repo>/pbs-work for temporary Castle audits, logs, replay artifacts, and diagnostic archives; verify Git ignores this directory.
- Never write to /tmp or any path outside nc2000-engine without explicit user approval.
- Do not create configuration backups or .bak/.orig files automatically, including in pbs-work.
- Git-only local ignore rules may live in .git/info/exclude; do not assume .gitignore contains the necessary rule.
- Do not change upstream-maintained bot core, WASM, engine, or Web as part of Castle diagnostics.
