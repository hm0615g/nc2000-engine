# Castle/Discord incident status

<!-- castle-continuation-policy:start -->
# Castle/Discord 対戦調査・停止条件・作業状況

> 重要: Castle bot の調査・修正前に必ずこの文書を読む。修正後はこの文書の進捗・根拠・テスト結果を更新する。
> 本文にない作業を「完了」と呼ばない。未検証は未検証と明記する。

## 1. ユーザーが決定した本番運用の優先順位（2026-11-21）

- 対戦が異常終了すること自体が本番環境では望ましくない。最優先目標は対戦の継続・完遂。
- 局面情報が不完全な場合も、可能な回復・代替局面・城の合法選択肢を利用して進行する。
- 異常検出・証拠保存と対戦停止を分離する。情報不一致の存在だけで停止しない。
- ただし城にない入力を捏造したり、受理状況が不明な送信を無条件に再送したりしない。
- 異常が再発した場合に追跡できるよう、原文、時刻、判断、実際の操作、代替措置を記録する。

### 「安全停止」と認めない8系統（修正対象）

| No. | 原因 | 継続の原則 |
| --- | --- | --- |
| 4 | 未知の城画面 | 最新画面の再取得・再判定・状態復元を試みる |
| 5 | 観測中に画面が変化 | 新しい画面を取り直し、処理を続ける |
| 6 | フォーム・ターン番号の不整合 | 城の現在のフォームを根拠に再同期する |
| 8 | LLOG解析・変換失敗 | 確定済み情報を保存し、未解釈部分を記録して続ける |
| 9 | 未対応・未検証の通常対戦イベント | 正式実装を進め、当面は可能な代替手段で継続する |
| 11 | 最大HP未観測・HP不一致 | 即瀕死などの確定情報を利用し、通常事象で停止しない |
| 13 | 技・PP・特殊行動の不整合 | 城が提示する合法な操作から代替行動を選ぶ |
| 14 | 相手候補・最大HP補完失敗 | 候補を拡大・制約緩和・代替探索を利用する |

残る 1、2、3、7、10、12、15、16 はユーザーによる停止妥当性の判定がまだ済んでいない。
No.11 immediate-faint fix is PARTIAL; all other approved categories and remaining No.11 cases are PENDING.無制限の再送、無限待機、証拠のない補完で置き換えない。

## 2. エラー調査の必須手順

1. 前回監査済みの試合IDを除外し、新規対戦のみを調べる。過去の既知現象を新規不具合として重複計上しない。
2. **初回の調査で一括収集する**: 対象全ターン・全判断、両者の選出/行動/HP/瀕死/状態/持ち物/PP/場の効果、城の入力画面、PDAT、LLOG、観測履歴、最終PositionSpec・WASM stateView、探索結果・合法手・送信値、失敗前後、終了画面。
3. 実際の画面上で何が起きたか、どの技・交代・特殊処理が発生したかを時系列で説明し、コード上の停止判定と対応させる。
4. 城表示の遅延、ログ変換の誤り、探索器の盤面誤認、探索判断への悪影響を区別する。警告件数を盤面不一致件数と混同しない。
5. 初回から根本原因まで分析する。追加データが不可欠なら、必要な証拠をまとめて一度に取得する。
6. 回帰テストには同じ生トレースを使い、修正前再現・修正後の継続・合法手・局面整合性・新旧正常系を確認する。

### ユーザーに渡す実行コードの必須条件

- **実行用スクリプト全文をチャット本文へ掲載すること。** ファイルのダウンロードだけで済ませない。
- **任意のディレクトリから、コマンド全体を一括コピペ実行できること。** 作業ディレクトリの移動やパスの書き換えを要求しない。
- 端末の日本語文字コード問題が発生したため、必要に応じてコマンド本文をASCIIのみで構成する。
- 変更前に対象範囲を検査し、構文・実行・再実行のテストを行う。未実施のテストをPASSと報告しない。

## 3. 既知事象と調査済みの範囲（2026-11-21時点）

- fork: `hm0615g/nc2000-engine` / `review-local-changes`。本家取込完了基準commit: `32409c9622668a3cae641f97f259e1c9949deb3d`。
- 前回監査済みの実戦22試合は除外。最後の既知監査済みID: `2026-11-16T22-36-18-702Z-K-g1`。
- 今回の新規6試合: `2026-11-20T15-34-27-819Z-K-g1`, `2026-11-20T15-41-38-718Z-K-g1`, `2026-11-20T15-51-55-270Z-K-g1`, `2026-11-20T15-59-46-555Z-K-g1`, `2026-11-20T16-29-14-394Z-K-g1`, `2026-11-20T16-36-44-915Z-K-g1`。全198判断。完了5、観測停止1。
- 5件の`PDAT_FOE_REMAINING_MISMATCH`: 4件はじばく/だいばくはつ、1件はドーブルのみちづれ。いずれも双方瀕死後の`CHANGE`画面でPDATだけ相手残数が1多く、LLOGと探索器の瀕死数は5/5一致、次の`INPUT`でPDATが一致。**既知の城表示更新遅延。新規不具合に計上しない。**
- `2026-11-20T16-29-14-394Z-K-g1`: 8ターンで相手バリヤードのバトンタッチ→ドーブル登場→まきびしでHP161→141→自分カビゴンのすてみタックルで141→0→ドーブル瀕死→バリヤード再登場。次の9ターン`BATTLE_INPUT`で停止。カビゴン275/292、バリヤード74/146。LLOGにドーブルHP0・瀕死があるが、ドーブルの最大HPを独立公開画面から確認できなかった。`tools/pbs-castle-observation-ledger.js`の`PUBLIC_MAXIMUM_HP_NOT_OBSERVED`が停止させた。
- 上記ドーブル事象はバトンタッチ限定ではない。通常交代や強制交代も含め、公開最大HPを未観測のまま倒れるケースを一般的に処理する。
- この6試合は本家取込前の実戦であり、本家取込後のadaptive実戦の監査は未実施。

## 4. 進捗（修正のたびに必ず更新）

| 作業 | 状態 |
| --- | --- |
| 本家取込 | 完了。上記commitを基準 |
| 新規6試合の異常原因分析 | 完了。既知表示遅延5件・最大HP未観測停止1件 |
| 停止条件16系統の一次分類 | 完了（個々の検査の網羅的な修正審査は未実施） |
| 8系統の停止不可・継続優先方針 | ユーザー決定済み |
| 最大HP未観測の即瀕死への修正 | PARTIAL FIX: source-linked HP0 + faint event; offline regression PASS; live match untested |
| その他7系統の継続処理実装 | 未実施 |
| 実トレースによる修正後回帰テスト | PASS: 3 synthetic cases + original 8 prior inputs and turn-9 scene, no Castle network |
| Castle source commit/push | VERIFIED: 132d389332a296c3cef42028527224abf82cb199; normal-path live 2/2 FINISHED after deploy; fallback live untested |
| 本家取込後adaptive実戦の評価 | 未実施 |

## 5. 作業履歴（追記すること）

- 2026-11-21: 新規6試合の詳細調査完了。5件は既知の相打ち後PDAT表示遅延。1件は最大HPを観測する前に瀕死となったドーブルの観測停止と特定。修正未着手。
- 2026-11-21: ユーザーが停止系統4,5,6,8,9,11,13,14を原則停止不可と決定。対戦継続優先・異常記録・代替行動実装を今後の方針とした。

### 次の担当者が作業終了時に記録すること

変更したソース、どの停止条件を改善したか、実戦証拠、再現方法、テストの実際のPASS/FAIL、既知の悪影響、未解決事項、commit SHA、push確認結果、次の調査対象試合の境界IDを、本書の進捗と履歴に追記する。
<!-- castle-continuation-policy:end -->

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

<!-- castle-zero-hp-faint-progress-v1 -->
## Castle continuation fix progress (2026-11-21)
Only tested work is marked complete.
One fixed subcase does not complete an entire category.
| Stop category | Status |
| --- | --- |
| 4 | PENDING |
| 5 | PENDING |
| 6 | PENDING |
| 8 | PENDING |
| 9 | PENDING |
| 11 | PARTIAL: immediate faint with source-linked HP0 verified; other HP issues PENDING |
| 13 | PENDING |
| 14 | PENDING |

Changed files:
- tools/pbs-castle-observation-ledger.js
- tools/pbs-castle-observation.js
- tools/pbs-castle-faint-no-maxhp.test.js

Evidence: 2026-11-20T16-29-14-394Z-K-g1.
Tests: synthetic cases and original stopped-game replay through turn 9.
Castle network: not used. Live battle: not tested.
Core/Web: unchanged. Code commit/push: VERIFIED on fork (132d389332a296c3cef42028527224abf82cb199). Live battle: NOT TESTED.
Update every category after each confirmed fix.

<!-- castle-continuation-8-progress-v1 -->
## Eight-category production continuation status
The runtime continuation handler covers failures before an attempted POST.
It uses the current Castle decision form, records every recovery, and
retries reading when no actionable form is available.
A submitted-but-unconfirmed POST remains a separate category 16 concern.
These statuses supersede any earlier generic statement that all 8 are unimplemented.

| Category | Status | Remaining work |
| --- | --- | --- |
| 4 | CONTINUATION_HANDLER_INSTALLED | Live unknown-page recovery not verified |
| 5 | CONTINUATION_HANDLER_INSTALLED | Live changed-page recapture not verified |
| 6 | CONTINUATION_HANDLER_INSTALLED | Invalid-form re-read not verified live |
| 8 | CONTINUATION_HANDLER_INSTALLED | LLOG parser repair pending |
| 9 | CONTINUATION_HANDLER_INSTALLED | Event-specific parser improvements pending |
| 11 | CONTINUATION_HANDLER_INSTALLED | HP model repair pending; immediate-faint offline fix retained |
| 13 | CONTINUATION_HANDLER_INSTALLED | PP and special-action semantic repair pending |
| 14 | CONTINUATION_HANDLER_INSTALLED | Opponent model repair pending |

Tests: node --check client/helper/test; helper continuation test; original
stopped scene form replay; previously installed immediate-faint test when present.
No Castle HTTP requests in these tests; no live match verification yet.
Code commit/push: VERIFIED on fork (132d389332a296c3cef42028527224abf82cb199). Live battle: NOT TESTED.
Do not mark a category FULLY_FIXED until its regression and real-game criteria pass.

### Change history
2026-11-21: installed general continuation fallback for categories
4,5,6,8,9,11,13,14. Source checks remain diagnostic; exceptions before
POST are converted to Castle-form fallback or continued polling.
No source-side errors are silently suppressed: trace events retain the reason.
When the implementation is verified and deployed, change each row to FIXED
individually and remove it from the pending-work inventory.

<!-- castle-continuation-push-verified-v1 -->
## Verified fork deployment of continuation code
Code commit: 132d389332a296c3cef42028527224abf82cb199
Fork: hm0615g/nc2000-engine, branch review-local-changes.
Code commit verified by comparing local HEAD with fork ls-remote.
Tests: continuation 10/10 PASS; immediate-faint 4/4 PASS.
Live Castle battle: NOT TESTED.
Eight categories: continuation handler installed, NOT fully resolved.
Pending: live coverage, event-specific semantic repairs, further regression.
The documentation follow-up commit and remote SHA are verified by the runner.

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

<!-- castle-postdeploy-normal-play-v1 -->
## Post-deployment Castle live verification (2026-11-20 UTC)

- Deployed at: 2026-11-20T20:58:03.000Z.
- Branch: hm0615g/nc2000-engine, review-local-changes; verified deploy HEAD: 2ec1d48e6a1be536b7e7646038f9a374112ced62.
- Evidence: operator-provided read-only scan of data/pbs-traces and pbs-work (69,566 entries; no scan limit; 80 JSONL trace files; two new matches after deployment). Raw trace files were not independently re-read for this document update.

| Match ID | Outcome | Decisions | Observation errors | Fallback attempts | Fallback confirmed | Stops | Invalid lines | Recovered unknown pages | Wait-for-form |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2026-11-20T21-05-20-206Z-K-g1 | FINISHED / WIN | 14 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| 2026-11-20T21-13-05-615Z-K-g1 | FINISHED / LOSE | 17 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

- Normal-path live verification: PASS, 2/2 matches finished; 31 decisions, no observation errors and no stops.
- Recovery behavior is NOT verified by these live matches: zero fallback attempts. Do NOT mark stop categories 4, 5, 6, 8, 9, 11, 13, or 14 fully fixed.
- Previously recorded offline regression: continuation 10/10 PASS; immediate-faint 4/4 PASS. These tests were not re-run in this documentation update.
- New-match audit boundary: after 2026-11-20T21-13-05-615Z-K-g1 (started 2026-11-20T21:13:05.616Z). Do not count either recorded match as new in subsequent audits.
- Next: deterministic no-network pre-POST observation-failure integration test, then verify live recovery, current-form legal choice and exactly one guarded submission without suppressing evidence.
- This documentation change makes no Castle requests and changes no bot source.

<!-- castle-upstream-sync-2026-11-24-v1 -->
## Upstream master integration (2026-11-24)

- Upstream: puniu3/nc2000-engine master at ba9a251323bd664d3381a59d45ee935b0086f3e7; common ancestor 0325ea9a2b8ebd4d5a49a3b7357e4ca483fae0c0.
- Fork before merge: hm0615g/nc2000-engine review-local-changes at 5d973c4bd4156fb67a2af164e548b02d9630e4bc.
- PS: preserved fork-only runtime, team directory, and Web-adaptive policy; adopted upstream default catalog team-pool-v3 and removed disabled compiled-preview table code. No PS network battle executed.
- Castle: adopted upstream catalog team-pool-v3 and flagship mixture as the new default; legacy pool:nash stays explicitly selectable. Team Preview always uses live search; the observation and preview paths explicitly select castle-free for L_TURN=0, castle-tournament for 80, and custom Castle ruleset for other limits. No Castle HTTP request executed.
- Conserved fork AGENTS.md safety rules alongside upstream contributor rules; core, WASM, engine and Web match upstream code.
- Tests: Node syntax for PS/Castle/observation PASS; PS help startup without external Showdown PASS; Castle help startup PASS; v3 catalog/flagship and Castle ruleset selection checks PASS; Castle continuation/faint unit tests PASS; cargo test -p nc2000-engine -p nc2000-bot PASS; PS native worker release build PASS; node WASM rebuild PASS; castle-ruleset Node test PASS.
- Not covered: real PS runtime initialization with an installed Pokemon Showdown build, full live Castle/PS match, specific fault recovery under upstream WASM, and obtaining the hosts actual L_TURN when joining as P2. The upstream castle-bot-design.md still marks general default ruleset changes deferred. Do not label these FULLY_FIXED.
- Previous successful Castle match boundary remains after 2026-11-20T21-13-05-615Z-K-g1.

<!-- castle-upstream-alignment-stage2-2026-11-24 -->
## Upstream-alignment repair (2026-11-24)

- Baseline: fork 803de6ec3d8151a42c0d3ee74be39f61562ff855; upstream ba9a251323bd664d3381a59d45ee935b0086f3e7.
- Evidence: 2026-11-24T02-30-58-075Z-K-g1 (8 decisions) and 2026-11-24T02-38-09-131Z-K-g1 (12 decisions).
- Stage 1: upstream off-pool belief fallback permitted only after public HP and ruleset checks.
- Stage 1 operator tests: replay 20/20 PASS, upstream fallback 18 decisions, Discord selftest PASS, Castle regression 14/14 PASS, WASM Castle ruleset PASS.
- Stage 2: single battle decision pipeline retained; source parity, PP, HP and legal-action guards retained.
- Stage 2: automatic post-match audit changed to opt-in; raw traces retained for offline audit.
- Stage 2: bundle manifest updated for Castle modules and flagship v3; fixed iters omitted from example config; handoff updated.
- Stage 2: two initial decisions tested using reduced-iteration WASM adaptive search, not production full-budget search.
- Historical duplicate patch scripts retained pending separate deletion review.
- Remaining: real post-deployment battle and full board parity certification.
- Git commit/push: NOT PERFORMED by this script.

<!-- castle-upstream-alignment-operator-verification-2026-11-24 -->
## Upstream-alignment operator test results (2026-11-24)

- Baseline HEAD: 803de6ec3d8151a42c0d3ee74be39f61562ff855.
- Stage 1: PATCH_READY_FOR_REVIEW; mock gate PASS; node syntax PASS.
- Stage 1: real Castle trace replay PASS (8 + 12 = 20 decisions); upstream off-pool fallback used in 18 decisions.
- Stage 1: Discord selftest PASS; Castle regressions 14/14 PASS; WASM Castle ruleset PASS.
- Stage 2: STAGE2_READY_FOR_REVIEW; bundle manifest PASS (30 files); Discord selftest PASS.
- Stage 2: adaptive WASM search smoke PASS for both postmerge matches, 128 iterations each, upstream fallback used in both.
- Stage 2: Castle regressions PASS; WASM Castle ruleset PASS; Git diff check PASS.
- The two 128-iteration probes do not certify full-budget production adaptive search or full-turn parity.
- Existing RECOGNIZED_SOURCE_WITHOUT_PROTOCOL events remain investigation items; they were not a blocker in the replay.
- The raw trace collection remains enabled; post-match parity audit is opt-in.
- Live postrepair Castle battle and full-board parity are NOT TESTED.

<!-- castle-upstream-alignment-fork-deploy-2026-11-24 -->
## Verified fork source deployment (2026-11-24)

- Fork: hm0615g/nc2000-engine, review-local-changes.
- Source commit: 9e6cc2d610bc2253f7f9b2c8b3c9bb4e70a30707.
- Verified the fork remote branch SHA equals the source commit after an explicit non-force push.
- Tested: stage-1 real trace 20/20 PASS with 18 off-pool fallbacks; stage-2 two 128-iteration adaptive probes PASS; Discord selftest, Castle regressions, WASM Castle ruleset PASS.
- Not certified: live Castle postrepair match, complete per-turn board parity, and full-budget 243000 iteration battle performance.
- Old duplicate diagnostic scripts remain in the repository; removal deferred until independent dependency review.

<!-- castle-live-auditor-parity-fix-2026-11-24 -->
## Post-deployment Castle trace auditor correction (2026-11-24)

- Source: two raw traces starting 2026-11-24T04-16-23-779Z-K-g1 and 2026-11-24T04-22-20-293Z-K-g1.
- Both finished (LOSE); all 26 decisions used the normal observed path: 24 adaptive searches, 2 forced single choices.
- All 26 submissions confirmed once each; zero observation exceptions or emergency fallbacks.
- Both encounters used UPSTREAM_OFF_POOL_FALLBACK while maintaining public-HP and legal-root checks.
- Previously 24 ACTION_MAPPING_FALLBACK errors were false positives: auditor omitted observed:adaptive-search.
- Three PDAT_FOE_REMAINING_MISMATCH entries were verified delayed Castle CHANGE counts: game1 turn11 Explosion; game2 turn4 Explosion and turn7 Self-Destruct.
- Next independent INPUT PDAT corrected all three; both LLOG and final PositionSpec already agreed on fainting.
- Auditor now downgrades only precisely confirmed CHANGE-page delays to CASTLE_CHANGE_FOE_REMAINING_DELAY_CONFIRMED warnings.
- Negative checks retain ERROR for truly inconsistent mapping and for uncorrected foe count.
- Operator regression on both actual traces: errorCount=0 in raw and reparse modes; Discord selftest PASS; continuation/faint regression PASS.
- Position full certification remains INCOMPLETE; converter branches and hidden opponent state are not fully certified.
- Worktree change only: no commit/push, no Castle request, no source deletion.

<!-- castle-live-auditor-fork-verified-2026-11-24 -->
## Verified fork deployment: Castle live trace auditor (2026-11-24)

- Fork: hm0615g/nc2000-engine, branch review-local-changes.
- Source commit: cc2c1abc53740e6b415f419fc6b9114682829eab.
- Source branch SHA independently checked after non-force push to the explicit fork URL.
- Live matches: 2026-11-24T04-16-23-779Z-K-g1 (LOSE, 15 decisions) and 2026-11-24T04-22-20-293Z-K-g1 (LOSE, 11 decisions).
- Both matches: 26/26 normal observed decisions, 24 adaptive searches, 2 single legal choices, 26 confirmed submissions, zero observation exceptions or emergency fallbacks.
- Reparsed audit after fix: 0 errors, 55 warnings total; 3 confirmed Castle CHANGE-page foe-count delays and 52 legacy-reparse/limited-certification warnings.
- Negative action-mapping regression: PASS; prior operator run also verified unresolved count remains an error.
- Discord selftest, Castle continuation and faint regression: PASS.
- Auditor-only patch: no live battle performed after this code change.
- Complete game-state certification remains PENDING: converter branches, hidden opponent state, and field effects are not all independently certified.
- No upstream core, WASM, or Web changes. Old duplicate diagnostic tools were not deleted.

<!-- castle-discord-default-catalog-2026-11-26 -->
## Discord default team change (2026-11-26)

- For normal Discord challenges, pbs-discord-bot.js now forwards --team pool:random to pbs-client.js.
- pool:random draws according to drawWeight from team-pool-v3/team-pool.json (33 team rows, 32 species shells, total mass 1).
- Explicit --team pool:flagship restores the prior two-variant mixture. PBS_TEAM and config team override also supported.
- Mercy tier defaults to pool:tier-a unless an explicit override is configured.
- Direct standalone pbs-client.js default remains pool:flagship. Core, WASM, Web and opponent belief are unchanged.
- Offline tests in operator worktree: node syntax PASS, v3 catalog checks PASS, Discord selftest with default/flagship/mercy mock-spawn PASS, Castle continuation/faint regressions PASS, WASM Castle ruleset PASS.
- No live Castle network requests were made; no live match test performed.
- Git commit and push NOT PERFORMED by this patch script.

<!-- castle-discord-catalog-fork-verified-2026-11-26 -->
## Verified Discord default v3 catalog deployment (2026-11-26)

- Fork: hm0615g/nc2000-engine, branch review-local-changes.
- Source commit: a8e05c5286af47ec3d0eff9418f81c960bf915b3.
- Source push verified by exact remote SHA comparison.
- Default Discord tier now passes --team pool:random, weighted across 33 rows / 32 species shells in data/team-pool-v3/team-pool.json.
- Explicit --team pool:flagship preserves the prior two-variant mixture. PBS_TEAM and config team override are supported.
- Mercy tier continues to use pool:tier-a unless explicitly overridden.
- Standalone pbs-client.js default remains unchanged. Core, WASM, and Web are unchanged.
- Operator tests: Node syntax PASS, catalog weights PASS, Discord selftest PASS, Castle regression PASS, WASM Castle ruleset PASS.
- Pre-push verification repeated in the source commit script. No live Castle match tested after this change.
