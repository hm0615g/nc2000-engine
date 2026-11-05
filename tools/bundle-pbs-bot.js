#!/usr/bin/env node
/**
 * Assemble a self-contained, cross-platform (Windows/macOS/Linux) portable package
 * for the Gym Leader's Castle Discord Bot under `dist/pbs-castle-bot/`.
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
      observe: 'node tools/pbs-discord-bot.js --observe',
      'dry-run': 'node tools/pbs-discord-bot.js --dry-run-pbs',
      selftest: 'node tools/pbs-discord-bot.js --selftest',
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
    replyMode: 'reply',
    postMatchThanksText: '対戦ありがとうございました！',
    iters: 81000,
    hostTimeoutSec: 240,
    roomWaitTimeoutSec: 90,
    pbsBaseUrl: 'http://psense.lib.net/PBS/_/{ROOM}/PS.cgi',
    observeOnly: false,
    dryRunPbs: false,
    logAllChannelEvents: true,
  };
  fs.writeFileSync(
    path.join(OUT_DIR, 'pbs-bot.config.example.json'),
    JSON.stringify(exampleCfg, null, 2) + '\n'
  );

  // Write Windows launchers (`start-bot.bat`, `observe-room-bot.bat`, `dry-run-room-bot.bat`)
  const makeBat = extraFlag => [
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
    `node tools\\pbs-discord-bot.js ${extraFlag}`.trim(),
    'pause',
  ].join('\r\n') + '\r\n';

  fs.writeFileSync(path.join(OUT_DIR, 'start-bot.bat'), makeBat(''));
  fs.writeFileSync(path.join(OUT_DIR, 'observe-room-bot.bat'), makeBat('--observe'));
  fs.writeFileSync(path.join(OUT_DIR, 'dry-run-room-bot.bat'), makeBat('--dry-run-pbs'));

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
2. Discord Developer Portal (https://discord.com/developers/applications) で Bot を作成し：
   - **Bot** タブ → **Privileged Gateway Intents** の **MESSAGE CONTENT INTENT** を **ON** にします（必須）。
   - **Bot** タブ → **Reset Token** から Bot Token を取得します。
   - **OAuth2 → URL Generator** で \`bot\` スコープ（権限：\`View Channels\`, \`Send Messages\`, \`Read Message History\`）を選び、城 Discord サーバーに招待します。
3. \`pbs-bot.config.example.json\` をコピーして **\`pbs-bot.config.json\`** を作成し、編集します：
   \`\`\`json
   {
     "discordToken": "ここに取得したDiscord Botトークンを貼る",
     "pbsName": "城で表示するトレーナー名",
     "pbsPass": "城のパスワード",
     "pbsMsg": "よろしくお願いします！",
     "channelIds": [],
     "roomBotIds": [],
     "replyTriggerText": "応戦します",
     "replyMode": "reply",
     "postMatchThanksText": "対戦ありがとうございました！",
     "iters": 81000,
     "hostTimeoutSec": 240,
     "roomWaitTimeoutSec": 90
   }
   \`\`\`

---

## 2. 城Discordの部屋割り当てBotの挙動確認・テスト方法

部屋割り当てBotのメッセージ形式や反応条件を確認できるよう、**3つの起動モード**と**自動イベントロガー (\`data/discord-events.jsonl\`)** を備えています。

### ステップ1: 観察モード (\`observe-room-bot.bat\` または \`./start-bot.sh --observe\`)
- **一切発言せず、城への入室も行いません。**
- チャンネルに流れる全メッセージ・Embed・編集イベント (\`MESSAGE_CREATE\`, \`MESSAGE_UPDATE\`, \`THREAD_CREATE\`) をコンソールと \`data/discord-events.jsonl\` に記録し、部屋番号パーサーがどの部屋（\`C\`〜\`O\`）を抽出したかをリアルタイム表示します。
- 他の人間同士の募集〜「応戦します」〜部屋割り当てBotの返信を1回観察するだけで、部屋Botの正確なフォーマットと \`author.id\` がすべて \`data/discord-events.jsonl\` に記録されます。

### ステップ2: 部屋割り当てドライラン (\`dry-run-room-bot.bat\` または \`./start-bot.sh --dry-run-pbs\`)
- \`@Bot\` でメンションされると**即座に \`応戦します\` と返信**し、部屋割り当てBotから返ってきた部屋番号（例：\`K部屋\`）を検出するところまでをテストします（実際の \`PS.cgi\` へは入室しません）。
- もし部屋割り当てBotが「Discordの返信機能 (\`reply\`)」ではなく「メンション付き (\`<@相手ID> 応戦します\`)」や「通常発言 (\`plain\`)」を要求する仕様だった場合は、\`pbs-bot.config.json\` の \`"replyMode"\` を \`"reply"\` / \`"mention"\` / \`"plain"\` のいずれかに変更するだけで対応できます。

### ステップ3: 本番常駐モード (\`start-bot.bat\` または \`./start-bot.sh\`)
- \`@Bot\` でメンションされると即座に \`応戦します\` と返信し、部屋割り当てBotが指定した \`C\`〜\`O\` 部屋へ即座に入室して対戦を開始します。

---

## 3. 強さ・パーティの2段階切り替え & 情報漏れ活用・適応

- **通常時（デフォルト）**: \`81,000\` iterations (\`c = 0.4\`) + **Nash均衡混合 (\`pool:nash\`)** + 定跡選出ブック (\`compiled-preview-v1.json\`)
- **手加減・別パ要求時（メンションに \`手加減\`, \`別パ\`, \`サブ\`, \`Tier A\`, \`弱め\` 等が含まれる場合）**: \`81,000\` iterations (\`c = 0.4\`) + **Tier A サブパーティ箱 (\`pool:tier-a\`、人間由来の代表8構築)**
- **城固有の情報漏れ活用 & 対戦相手適応**:
  - 控えに引っ込んだ相手ポケモンの厳密 \`HP / 最大HP\`・状態異常・判明した持ち物の永続保持
  - 厳密 \`最大HP\` からの個体値・めざパタイプ絞り込み (\`hp_type_mask\`) と、ダメージ時に回復しなかった相手への「たべのこし非所持 (\`noLeftovers\`)」推論
  - 対戦相手の型・選出履歴を \`data/pbs-opponent-db.json\` に自動蓄積して再戦時に即時適応し、全試合の要約を \`data/pbs-matches.jsonl\` に記録
`;
  fs.writeFileSync(path.join(OUT_DIR, 'README.md'), readmeContent);

  console.log(`[bundle-pbs-bot] Successfully assembled portable package at: ${OUT_DIR}`);
}

if (require.main === module) {
  main();
}
