#!/usr/bin/env node
/**
 * Assemble a self-contained, cross-platform (Windows/macOS/Linux) portable package
 * for the Gym Leader's Castle Discord Bot under `dist/pbs-castle-bot/`, and build
 * the browser-accessible delivery directory `dist/pbs-delivery/` containing
 * `pbs-castle-bot.zip`, `HANDOFF.md`, and `index.html`.
 *
 * Usage:
 *   node tools/bundle-pbs-bot.js [--out dist/pbs-castle-bot]
 */
'use strict';

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const REPO = path.resolve(__dirname, '..');
const outArgIdx = process.argv.indexOf('--out');
const OUT_DIR = path.resolve(
  REPO,
  outArgIdx >= 0 && process.argv[outArgIdx + 1]
    ? process.argv[outArgIdx + 1]
    : 'dist/pbs-castle-bot'
);
const DELIVERY_DIR = path.resolve(REPO, 'dist/pbs-delivery');

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
  'tools/diagnose-position.js',
  'engine-manifest.json',
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

  // Copy docs/PBS-CASTLE-BOT-HANDOFF.md -> HANDOFF.md inside the bundle
  const handoffSrc = path.join(REPO, 'docs/PBS-CASTLE-BOT-HANDOFF.md');
  if (fs.existsSync(handoffSrc)) {
    fs.copyFileSync(handoffSrc, path.join(OUT_DIR, 'HANDOFF.md'));
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
    pbsPass: 'nc2pass',
    pbsMsg: 'よろしくお願いします',
    channelIds: [],
    roomBotIds: [],
    replyTriggerText: '応戦します',
    replyMode: 'reply',
    roomEnterGreetText: 'よろしくお願いします',
    postMatchThanksText: 'ありがとうございました',
    iters: 81000,
    hostTimeoutSec: 240,
    roomWaitTimeoutSec: 90,
    autoPickRoomAfterSec: 15,
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
  const readmeContent = `# ジムリーダーの城 (\`#2000-一撃なし\`) 常駐 Discord Bot — 運用ハンドオフ

このパッケージは Windows / macOS / Linux のどの環境でも **Node.js (v18以上)** さえ入っていれば単体で動作する自己完結フォルダです（Rust や \`iconv\` 等の外部バイナリは一切不要です）。
AIコーディングアシスタント（Claude / Cursor / ChatGPT 等）にこのままフォルダを読ませれば、詳細な技術仕様書 **\`HANDOFF.md\`** を参照して設定・運用・微調整をすべて自動で行えます。

---

## 1. セットアップ手順（最短3ステップ）

1. **Node.js (v18以上、推奨 v20/v22 LTS)** をインストールします（https://nodejs.org/）。
2. **Discord Developer Portal** (https://discord.com/developers/applications) で Bot を用意し：
   - **Bot** タブ → **Privileged Gateway Intents** の **MESSAGE CONTENT INTENT** を **ON** にします（必須）。
   - **Bot** タブ → **Reset Token** から Bot Token をコピーします。
   - **OAuth2 → URL Generator** で \`bot\` スコープ（権限：\`View Channels\`, \`Send Messages\`, \`Read Message History\`）を選び、城 Discord サーバーに招待します。
3. \`pbs-bot.config.example.json\` をコピーして **\`pbs-bot.config.json\`** を作り、\`discordToken\`・\`pbsName\`・\`pbsPass\`（半角8文字以内）を設定して起動します：
   - **Windows**: \`start-bot.bat\` をダブルクリック（初回のみ自動で \`npm install\` が走ります）
   - **Mac / Linux**: \`./start-bot.sh\` を実行

---

## 2. 城Discord (\`#2000-一撃なし\`) での動き

### ボット専用チャンネル & 部屋 K
- **対戦受付チャンネル**: \`989231786288611388\`（城Discord専用チャンネル、スレッド対応）
- **対戦部屋**: **\`第K\`** (\`http://psense.lib.net/PBS/_/K/PS.cgi\`)

### 対戦成立〜終了の流れ
1. 待機中は城サーバー（\`psense.lib.net\`）へ一切アクセスしません。
2. 承認チャンネルで誰かが \`@Bot 対戦希望します\`（または \`@Bot 手加減で対戦希望します\`）とメンションすると、即座にリプライで **\`応戦します\`** と返信します。
3. Bot は \`--mode host\` かつ \`--entry-status-ipc\` で **\`第K\`** 部屋へホスト入室を試行します。
   - **入室確定時**: PBS上で入室が確認された瞬間に **\`よろしくお願いします\`** と部屋URL（\`http://psense.lib.net/PBS/_/K/PS.cgi?ACTION=TITLE\`）を投稿し、相手のP2エントリーを待機します。
   - **部屋使用中 / 進行中**: 既に第K部屋で対戦が行われているか待機者がいる場合、即座に **\`現在部屋が利用できません\`** と返信して待機状態に戻ります。
4. 試合が終わるとチャンネルに **\`ありがとうございました\`** と投稿します。

---

## 3. 2段階の強さ・パーティ設定 ＆ データ蓄積

- **デフォルト (\`--tier default\`)**: \`81,000\` iterations + **Nash均衡混合 (\`pool:nash\`、APEX4)** + 定跡選出ブック (\`compiled-preview-v1.json\`)
- **手加減・別パ要求時 (\`--tier mercy\`)**: メンションに \`手加減\` / \`てかげん\` / \`別パ\` / \`サブ\` / \`Tier A\` / \`弱め\` 等が含まれる場合、\`81,000\` iterations + **Tier A サブパ箱 (\`pool:tier-a\`、人間由来の代表8構築)**
- **データ蓄積（Prior拡充用）**:
  - 対戦相手の型・選出・最大HP・持ち物履歴は \`data/pbs-opponent-db.json\` に自動保存され、連戦・再戦時に即座に相手適応Beliefとして使われます。
  - 全試合の要約ログは \`data/pbs-matches.jsonl\` に追記されます。定期的にこの2ファイルを回収してください。
`;
  fs.writeFileSync(path.join(OUT_DIR, 'README.md'), readmeContent);

  // Build delivery directory (`dist/pbs-delivery/`) with `pbs-castle-bot.zip`, `HANDOFF.md`, and `index.html`
  fs.mkdirSync(DELIVERY_DIR, { recursive: true });
  const zipPath = path.join(DELIVERY_DIR, 'pbs-castle-bot.zip');
  if (fs.existsSync(zipPath)) fs.unlinkSync(zipPath);
  execFileSync('zip', ['-r', '-q', zipPath, path.basename(OUT_DIR)], {
    cwd: path.dirname(OUT_DIR),
  });
  if (fs.existsSync(handoffSrc)) {
    fs.copyFileSync(handoffSrc, path.join(DELIVERY_DIR, 'HANDOFF.md'));
  }

  const zipStat = fs.statSync(zipPath);
  const zipSizeMb = (zipStat.size / (1024 * 1024)).toFixed(2);

  const indexHtml = `<!doctype html>
<html lang="ja">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>ジムリーダーの城 Discord Bot — 運用ハンドオフ & パッケージ配布</title>
  <style>
    :root {
      --bg: #0f141c;
      --panel: #171f2c;
      --border: #2a364b;
      --text: #e6edf6;
      --muted: #9fb0c7;
      --accent: #4ea1ff;
      --accent-hover: #73b6ff;
      --code-bg: #0b0f16;
      --ok: #3fb950;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      padding: 24px 16px 64px;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Hiragino Sans", "Noto Sans JP", sans-serif;
      background: var(--bg);
      color: var(--text);
      line-height: 1.65;
    }
    .container {
      max-width: 880px;
      margin: 0 auto;
    }
    header {
      background: var(--panel);
      border: 1px solid var(--border);
      border-radius: 12px;
      padding: 24px;
      margin-bottom: 24px;
    }
    h1 {
      margin: 0 0 8px;
      font-size: 1.5rem;
    }
    h2 {
      margin-top: 0;
      font-size: 1.2rem;
      border-bottom: 1px solid var(--border);
      padding-bottom: 8px;
    }
    .sub {
      color: var(--muted);
      margin: 0 0 18px;
      font-size: 0.95rem;
    }
    .btn-row {
      display: flex;
      flex-wrap: wrap;
      gap: 12px;
      align-items: center;
    }
    .btn {
      display: inline-flex;
      align-items: center;
      gap: 8px;
      background: var(--accent);
      color: #08101c;
      font-weight: 700;
      text-decoration: none;
      padding: 12px 20px;
      border-radius: 8px;
      border: none;
      cursor: pointer;
      font-size: 0.98rem;
    }
    .btn:hover { background: var(--accent-hover); }
    .btn-secondary {
      background: #233044;
      color: var(--text);
      border: 1px solid var(--border);
    }
    .btn-secondary:hover { background: #2d3e58; }
    section {
      background: var(--panel);
      border: 1px solid var(--border);
      border-radius: 12px;
      padding: 20px 24px;
      margin-bottom: 20px;
    }
    pre {
      background: var(--code-bg);
      border: 1px solid var(--border);
      border-radius: 8px;
      padding: 14px;
      overflow-x: auto;
      font-size: 0.88rem;
      line-height: 1.5;
      margin: 10px 0;
    }
    code {
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 0.9em;
    }
    table {
      width: 100%;
      border-collapse: collapse;
      margin: 12px 0;
      font-size: 0.93rem;
    }
    th, td {
      border: 1px solid var(--border);
      padding: 8px 12px;
      text-align: left;
    }
    th { background: #1d2738; }
    .tag {
      display: inline-block;
      padding: 2px 8px;
      border-radius: 99px;
      font-size: 0.8rem;
      background: #1f334d;
      color: #8cc4ff;
      font-weight: 600;
    }
    .copy-header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-top: 8px;
    }
    .copy-btn {
      background: #233044;
      color: var(--text);
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 4px 12px;
      cursor: pointer;
      font-size: 0.82rem;
    }
    .copy-btn.copied {
      border-color: var(--ok);
      color: var(--ok);
    }
  </style>
</head>
<body>
  <div class="container">
    <header>
      <h1>ジムリーダーの城 (<code>#2000-一撃なし</code>) 常駐 Discord Bot</h1>
      <p class="sub">Windows / macOS / Linux 対応ポータブルパッケージ（ビルド済みWASM・定跡ブック・EUC-JPクライアント同梱 / Node.js v18+ のみで動作）</p>
      <div class="btn-row">
        <a class="btn" href="pbs-castle-bot.zip" download="pbs-castle-bot.zip">
          ⬇ pbs-castle-bot.zip をダウンロード (${zipSizeMb} MB)
        </a>
        <a class="btn btn-secondary" href="HANDOFF.md" target="_blank" rel="noopener">
          📄 英語技術ハンドオフ (HANDOFF.md)
        </a>
      </div>
    </header>

    <section>
      <h2>1. 友人PCのAIアシスタント用 オンボーディングプロンプト</h2>
      <p>ご友人が ZIP を展開したフォルダで Claude Code / Cursor / Codex 等を開く際、以下のプロンプトをそのまま貼り付けるだけでセットアップ〜起動〜動作確認までAIが案内・実行できます：</p>
      <div class="copy-header">
        <span class="tag">AI貼り付け用プロンプト</span>
        <button class="copy-btn" onclick="copyText('ai-prompt', this)">コピー</button>
      </div>
      <pre id="ai-prompt">このフォルダ (pbs-castle-bot) はジムリーダーの城 (psense.lib.net/PBS/_/C..O/PS.cgi) の #2000-一撃なし チャンネル用常駐Discord Botです。
まず README.md と HANDOFF.md を読んで全体構成を把握してください。
その上で：
1. npm install --omit=dev と node tools/pbs-discord-bot.js --selftest を実行して環境とセルフテストを確認する
2. pbs-bot.config.example.json を pbs-bot.config.json にコピーし、私が渡す Discord Bot Token (discordToken) と城のトレーナー名 (pbsName) / パスワード (pbsPass、半角8文字以内) を設定する
3. node tools/pbs-discord-bot.js を起動して常駐を開始する
手順を案内・実行してください。</pre>
    </section>

    <section>
      <h2>2. セットアップ手順（人間向け最短3ステップ）</h2>
      <ol>
        <li><strong>Node.js (v18以上、推奨 v20/v22 LTS)</strong> をインストールします（<a href="https://nodejs.org/" style="color:var(--accent)">https://nodejs.org/</a>）。</li>
        <li><strong>Discord Developer Portal</strong> (<a href="https://discord.com/developers/applications" style="color:var(--accent)">https://discord.com/developers/applications</a>) で Bot を作成（または選択）し：
          <ul>
            <li><strong>Bot</strong> タブ → <strong>Privileged Gateway Intents</strong> の <strong>MESSAGE CONTENT INTENT</strong> を <strong>ON</strong> にします（必須）。</li>
            <li><strong>Bot</strong> タブ → <strong>Reset Token</strong> から Bot Token を取得します。</li>
            <li><strong>OAuth2 → URL Generator</strong> で <code>bot</code> スコープ（権限：<code>View Channels</code>, <code>Send Messages</code>, <code>Read Message History</code>）を選び、城 Discord サーバーに招待します。</li>
          </ul>
        </li>
        <li><code>pbs-bot.config.example.json</code> をコピーして <strong><code>pbs-bot.config.json</code></strong> を作成し、<code>discordToken</code>・<code>pbsName</code>・<code>pbsPass</code>（半角8文字以内）を記入して起動します：
          <ul>
            <li><strong>Windows</strong>: <code>start-bot.bat</code> をダブルクリック</li>
            <li><strong>Mac / Linux</strong>: <code>./start-bot.sh</code> を実行</li>
          </ul>
        </li>
      </ol>
      <div class="copy-header">
        <span class="tag">pbs-bot.config.json 設定例</span>
        <button class="copy-btn" onclick="copyText('cfg-json', this)">コピー</button>
      </div>
      <pre id="cfg-json">${JSON.stringify(exampleCfg, null, 2)}</pre>
    </section>

    <section>
      <h2>3. 確定済み動作仕様（城Bot専用チャンネル・K部屋仕様）</h2>
      <table>
        <thead>
          <tr>
            <th>設定項目</th>
            <th>値</th>
            <th>詳細</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td>受付チャンネル</td>
            <td><code>989231786288611388</code></td>
            <td>城Discordボット専用チャンネル（スレッド対応）</td>
          </tr>
          <tr>
            <td>対戦部屋</td>
            <td><strong>第K (K)</strong></td>
            <td><code>http://psense.lib.net/PBS/_/K/PS.cgi</code></td>
          </tr>
        </tbody>
      </table>
      <ul>
        <li><strong>呼び出し〜入室</strong>: <code>@Bot 対戦希望します</code> とメンションされると即座にリプライで <strong><code>応戦します</code></strong> と返し、<code>--mode host</code> かつ <code>--entry-status-ipc</code> で第K部屋に入室します。PBSへの入室が確認された瞬間に <strong><code>よろしくお願いします</code></strong> と部屋URL（<code>?ACTION=TITLE</code>）をDiscordに投稿します。</li>
        <li><strong>部屋使用中・進行中保護</strong>: 第K部屋が既に他者の対戦中や待機中の場合、待たされることなく即座に <strong><code>現在部屋が利用できません</code></strong> と返信します。</li>
        <li><strong>2段階パーティ切り替え</strong>:
          <ul>
            <li><strong>通常時 (<code>default</code>)</strong>: <code>81,000</code> iterations + <strong>Nash均衡混合 (<code>pool:nash</code>、APEX4)</strong> + 定跡選出ブック</li>
            <li><strong>手加減要求時 (<code>mercy</code>)</strong>: メンションに <code>手加減</code> / <code>別パ</code> / <code>サブ</code> / <code>Tier A</code> / <code>弱め</code> 等が含まれる場合、<code>81,000</code> iterations + <strong>Tier A サブパ箱 (<code>pool:tier-a</code>、人間由来の代表8構築)</strong></li>
          </ul>
        </li>
        <li><strong>城固有の情報漏れ活用 & オンライン適応</strong>: 控えに引っ込んだ相手ポケモンの厳密 <code>HP / 最大HP</code>・状態異常・持ち物の永続追跡、<code>最大HP</code> からのめざパタイプ＆個体値絞り込み、たべのこし非所持 (<code>noLeftovers</code>) 自動推論、および <code>data/pbs-opponent-db.json</code> による再戦時の即時型適応を行います。</li>
        <li><strong>ログ回収</strong>: <code>data/pbs-opponent-db.json</code>（相手型DB）と <code>data/pbs-matches.jsonl</code>（全試合ログ）に自動蓄積されます。</li>
      </ul>
    </section>
  </div>
  <script>
    function copyText(id, btn) {
      const text = document.getElementById(id).innerText;
      navigator.clipboard.writeText(text).then(() => {
        const orig = btn.innerText;
        btn.innerText = 'コピー完了 ✓';
        btn.classList.add('copied');
        setTimeout(() => {
          btn.innerText = orig;
          btn.classList.remove('copied');
        }, 1800);
      });
    }
  </script>
</body>
</html>
`;
  fs.writeFileSync(path.join(DELIVERY_DIR, 'index.html'), indexHtml);

  console.log(`[bundle-pbs-bot] Successfully assembled portable package at: ${OUT_DIR}`);
  console.log(`[bundle-pbs-bot] Successfully built delivery archive at: ${zipPath} (${zipSizeMb} MB)`);
}

if (require.main === module) {
  main();
}
