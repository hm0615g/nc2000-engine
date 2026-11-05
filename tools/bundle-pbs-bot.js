#!/usr/bin/env node
/**
 * Assemble a self-contained, cross-platform (Windows/macOS/Linux) portable package
 * for the Gym Leader's Castle Discord Bot under `dist/pbs-castle-bot/`.
 *
 * The resulting directory contains:
 * - Precompiled WASM engine (`crates/wasm/pkg-node/`) — no Rust or wasm-pack needed on the host PC
 * - Pure-JS EUC-JP PBS client (`tools/pbs-client.js`), Discord Gateway bot (`tools/pbs-discord-bot.js`),
 *   and PD0 cookie encoder (`tools/export-pdinput.js`) — no external `iconv` binary needed
 * - Required data tables (`data/kodoku-r3/conclusion-equilibrium-deme.json`, `data/team-pool-v2/team-pool.json`,
 *   `data/belief-pool-v3/belief-pool.json`, `data/compiled-preview-v1.json`, `data/gen2stadium2.json`,
 *   `data/i18n-ja.json`, `data/search-profiles.json`, `data/live-search.json`)
 * - `pbs-bot.config.example.json`, `package.json`, `start-bot.bat` (Windows), `start-bot.sh` (macOS/Linux),
 *   and `README.md`.
 *
 * Usage:
 *   node tools/bundle-pbs-bot.js [--out dist/pbs-castle-bot]
 */
'use strict';

const fs = require('fs');
const path = require('path');

const REPO = path.resolve(__dirname, '..');
const outArgIdx = process.argv.indexOf('--out');
const OUT_DIR = path.resolve(
  REPO,
  outArgIdx >= 0 && process.argv[outArgIdx + 1]
    ? process.argv[outArgIdx + 1]
    : 'dist/pbs-castle-bot'
);

const FILES_TO_COPY = [
  'tools/pbs-discord-bot.js',
  'tools/pbs-client.js',
  'tools/export-pdinput.js',
  'crates/wasm/pkg-node/nc2000_wasm.js',
  'crates/wasm/pkg-node/nc2000_wasm_bg.wasm',
  'crates/wasm/pkg-node/package.json',
  'data/search-profiles.json',
  'data/live-search.json',
  'data/gen2stadium2.json',
  'data/i18n-ja.json',
  'data/team-pool-v2/team-pool.json',
  'data/kodoku-r3/conclusion-equilibrium-deme.json',
  'data/belief-pool-v3/belief-pool.json',
  'data/compiled-preview-v1.json',
];

function copyRelFile(relPath) {
  const src = path.join(REPO, relPath);
  if (!fs.existsSync(src)) {
    throw new Error(`Missing required source file: ${src}`);
  }
  const dst = path.join(OUT_DIR, relPath);
  fs.mkdirSync(path.dirname(dst), { recursive: true });
  fs.copyFileSync(src, dst);
}

function main() {
  fs.mkdirSync(OUT_DIR, { recursive: true });

  for (const rel of FILES_TO_COPY) {
    copyRelFile(rel);
  }

  // Write minimal root package.json (only `ws` is needed; Node >= 18 provides global `fetch`)
  const pkgJson = {
    name: 'nc2000-pbs-castle-bot',
    version: '1.0.0',
    private: true,
    description: 'Gym Leader\'s Castle (PS.cgi) Resident Discord Bot powered by NC2000 WASM MCTS',
    scripts: {
      start: 'node tools/pbs-discord-bot.js',
      'play-room': 'node tools/pbs-client.js',
    },
    dependencies: {
      ws: '^8.19.0',
    },
    engines: {
      node: '>=18.0.0',
    },
  };
  fs.writeFileSync(
    path.join(OUT_DIR, 'package.json'),
    JSON.stringify(pkgJson, null, 2) + '\n'
  );

  // Write pbs-bot.config.example.json
  const exampleCfg = {
    discordToken: 'YOUR_DISCORD_BOT_TOKEN_HERE',
    pbsName: 'NC2000',
    pbsPass: 'your_castle_password',
    pbsMsg: 'よろしくお願いします！',
    channelIds: [],
    roomBotIds: [],
    replyTriggerText: '応戦します',
    postMatchThanksText: '対戦ありがとうございました！',
    iters: 81000,
    hostTimeoutSec: 240,
    roomWaitTimeoutSec: 90,
    pbsBaseUrl: 'http://psense.lib.net/PBS/_/{ROOM}/PS.cgi',
  };
  fs.writeFileSync(
    path.join(OUT_DIR, 'pbs-bot.config.example.json'),
    JSON.stringify(exampleCfg, null, 2) + '\n'
  );

  // Write Windows launcher (`start-bot.bat`)
  const batContent = [
    '@echo off',
    'cd /d "%~dp0"',
    'if not exist "node_modules\\ws" (',
    '  echo Installing dependencies...',
    '  call npm install --omit=dev',
    ')',
    'if not exist "pbs-bot.config.json" (',
    '  echo [ERROR] pbs-bot.config.json not found!',
    '  echo Copy pbs-bot.config.example.json to pbs-bot.config.json and set your discordToken.',
    '  pause',
    '  exit /b 1',
    ')',
    'echo Starting Gym Leader\'s Castle Discord Bot...',
    'node tools\\pbs-discord-bot.js',
    'pause',
  ].join('\r\n') + '\r\n';
  fs.writeFileSync(path.join(OUT_DIR, 'start-bot.bat'), batContent);

  // Write macOS/Linux launcher (`start-bot.sh`)
  const shContent = [
    '#!/usr/bin/env bash',
    'set -euo pipefail',
    'cd "$(dirname "$0")"',
    'if [ ! -d "node_modules/ws" ]; then',
    '  echo "Installing dependencies..."',
    '  npm install --omit=dev',
    'fi',
    'if [ ! -f "pbs-bot.config.json" ]; then',
    '  echo "[ERROR] pbs-bot.config.json not found! Copy pbs-bot.config.example.json to pbs-bot.config.json first."',
    '  exit 1',
    'fi',
    'exec node tools/pbs-discord-bot.js "$@"',
  ].join('\n') + '\n';
  const shPath = path.join(OUT_DIR, 'start-bot.sh');
  fs.writeFileSync(shPath, shContent);
  fs.chmodSync(shPath, 0o755);

  // Write README.md for the friend's PC
  const readmeContent = `# ジムリーダーの城 (PS.cgi) 常駐 Discord Bot パッケージ

このフォルダは Windows / macOS / Linux のどの環境でも **Node.js (v18以上)** さえ入っていればそのまま動く自己完結パッケージです（Rust や \`iconv\` 等の外部ツールは不要です）。

---

## 1. 初回セットアップ手順（常駐させるPCにて）

1. **Node.js (v18以上、推奨 v20/v22 LTS)** をインストールします（https://nodejs.org/）。
2. Discord Developer Portal (https://discord.com/developers/applications) で Bot を作成（または既存 Bot を選択）し：
   - **Bot** タブ → **Privileged Gateway Intents** の **MESSAGE CONTENT INTENT** を **ON** にします（メンション本文の「手加減」指定や部屋割り当てメッセージを読むために必須です）。
   - **Bot** タブ → **Reset Token** から Bot Token を取得します。
   - **OAuth2 → URL Generator** で \`bot\` スコープ（権限：\`View Channels\`, \`Send Messages\`, \`Read Message History\`）を選び、城 Discord サーバーに招待します。
3. このフォルダ内の \`pbs-bot.config.example.json\` をコピーして **\`pbs-bot.config.json\`** を作成し、中身を編集します：
   \`\`\`json
   {
     "discordToken": "ここに取得したDiscord Botトークンを貼る",
     "pbsName": "城で表示するトレーナー名",
     "pbsPass": "城のパスワード",
     "pbsMsg": "よろしくお願いします！",
     "channelIds": [],
     "roomBotIds": [],
     "replyTriggerText": "応戦します",
     "postMatchThanksText": "対戦ありがとうございました！",
     "iters": 81000,
     "hostTimeoutSec": 240,
     "roomWaitTimeoutSec": 90
   }
   \`\`\`
   - \`channelIds\`: 空配列 \`[]\` のままなら全チャンネルで \`@メンション\` に反応します。特定チャンネルのみに限定したい場合は \`["123456789012345678"]\` のようにチャンネルIDを入れます。
   - \`roomBotIds\`: 空配列 \`[]\` のままで城の部屋割り当てBotの返信を自動検出します。

4. **起動方法**:
   - **Windows**: \`start-bot.bat\` をダブルクリック（初回のみ自動で \`npm install\` が走ります）
   - **Mac / Linux**: \`./start-bot.sh\` を実行

---

## 2. 動作仕様

### A. 部屋割り当てフロー（城Discord対応）
1. 待機中は城サーバー（\`psense.lib.net\`）へ一切アクセスしません。
2. Discord で誰かが \`@Bot 対戦お願いします\` とメンションすると、人間らしい間（約1.5〜2.8秒）を置いて **\`応戦します\`** とリプライします。
3. そのリプライに反応して **城Discordの常駐部屋割り当てBot** が部屋（例：\`K部屋\` や \`http://psense.lib.net/PBS/_/K/PS.cgi\`）を通知すると、即座にその部屋へ入室して1戦プレイします。
   - ※メンション時点で \`@Bot K部屋お願いします\` のように直接部屋が書かれていた場合は、\`応戦します\` と返信すると同時にその部屋へ直行します。

### B. 強さ・パーティの2段階切り替え（プレイヤー・スキュモーフィズム）
- **通常時（デフォルト）**:
  - 探索: **81,000 iterations** (\`c = 0.4\`) + 定跡選出ブック (\`compiled-preview-v1.json\`)
  - パーティ: **Nash均衡混合 (\`pool:nash\`)** — 同一6体シェル（カビゴン55/パルシェン50/ゲンガー50/ヘラクロス50/ガラガラ50/スターミー50）上のAPEX4均衡混合（40:30:15:15）
- **手加減・別パ要求時（メンションに \`手加減\`, \`てかげん\`, \`別パ\`, \`サブ\`, \`Tier A\`, \`弱め\` 等が含まれる場合）**:
  - 探索: **81,000 iterations** (\`c = 0.4\`)
  - パーティ: **Tier A サブパーティ箱 (\`pool:tier-a\`)** — 1人の人間プレイヤーがサブパとして持っていそうな代表的な8構築（サンダー・ライコウ・バンギラス・スイクン・ナッシー・カイリキー・ポリゴン2・フシギバナ軸など）からランダム選択

### C. 城固有の情報漏れ活用 & 対戦相手適応・データ収集
- **城固有の厳密HP・持ち物漏れ活用**:
  - 交代で引っ込んだ相手控えポケモンの厳密な \`現在HP / 最大HP\` と状態異常を記憶し続けます。
  - 観測された厳密な \`最大HP\` から個体値（めざめるパワーのタイプ候補マスク・HP個体値）を逆算し、またターン終了時にダメージを受けた相手が回復しなかった場合は「たべのこし非所持 (\`noLeftovers\`)」を即座に推論して探索のBelief分布を更新します。
- **即時適応 & Prior拡充用ログ保存**:
  - 対戦相手の6体構成・選出・判明した技・持ち物・最大HPを \`data/pbs-opponent-db.json\` に自動蓄積し、再戦時には相手の型を反映した適応Beliefで探索します。
  - 全試合の要約ログは \`data/pbs-matches.jsonl\` に追記されます（このファイルを回収することでオフラインのPrior拡充にそのまま使えます）。
`;
  fs.writeFileSync(path.join(OUT_DIR, 'README.md'), readmeContent);

  console.log(`[bundle-pbs-bot] Successfully assembled portable package at: ${OUT_DIR}`);
}

if (require.main === module) {
  main();
}
