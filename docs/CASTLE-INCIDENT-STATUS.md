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
上記8系統の具体的な復旧方法は未実装。無制限の再送、無限待機、証拠のない補完で置き換えない。

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
| 最大HP未観測の即瀕死への修正 | 未実施 |
| その他7系統の継続処理実装 | 未実施 |
| 実トレースによる修正後回帰テスト | 未実施 |
| 対戦コードの修正commit/push | 未実施 |
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
