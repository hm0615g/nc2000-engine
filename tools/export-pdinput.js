#!/usr/bin/env node
/**
 * Generate a standalone HTML importer (`pdinput-import.html`) or JSON manifest
 * that encodes `data/team-pool-v2/team-pool.json` into `psense.lib.net/_/PDINPUT.cgi`
 * 26-field `PD` records and provides automated EUC-JP form submission and
 * 1-click Cookie installation.
 *
 * Usage:
 *   node tools/export-pdinput.js [output.html] [--verify-live]
 */
'use strict';

const fs = require('fs');
const path = require('path');

const REPO = path.join(__dirname, '..');
const pool = JSON.parse(fs.readFileSync(path.join(REPO, 'data/team-pool-v2/team-pool.json'), 'utf8'));
const dex = JSON.parse(fs.readFileSync(path.join(REPO, 'data/gen2stadium2.json'), 'utf8'));
const i18n = JSON.parse(fs.readFileSync(path.join(REPO, 'data/i18n-ja.json'), 'utf8'));

const PDINPUT_URL = 'http://psense.lib.net/_/PDINPUT.cgi';
const PDINPUT_MAX_BATCH_SIZE = 17;
const PDINPUT_COOKIE_LIMIT_BYTES = 8000;

const PDINPUT_ITEM_CODES = {
  '': 0,
  masterball: 2,
  ultraball: 3,
  brightpowder: 4,
  greatball: 5,
  pokeball: 6,
  moonstone: 9,
  firestone: 23,
  thunderstone: 24,
  waterstone: 25,
  luckypunch: 31,
  leafstone: 35,
  metalpowder: 36,
  quickclaw: 74,
  psncureberry: 75,
  softsand: 77,
  sharpbeak: 78,
  przcureberry: 79,
  burntberry: 80,
  iceberry: 81,
  poisonbarb: 82,
  kingsrock: 83,
  bitterberry: 84,
  mintberry: 85,
  silverpowder: 89,
  mysticwater: 96,
  twistedspoon: 97,
  blackbelt: 99,
  blackglasses: 103,
  pinkbow: 105,
  stick: 106,
  nevermeltice: 108,
  magnet: 109,
  miracleberry: 110,
  spelltag: 114,
  miracleseed: 118,
  thickclub: 119,
  focusband: 120,
  hardstone: 126,
  charcoal: 139,
  berryjuice: 140,
  scopelens: 141,
  metalcoat: 144,
  dragonfang: 145,
  leftovers: 147,
  mysteryberry: 151,
  dragonscale: 152,
  berserkgene: 153,
  heavyball: 158,
  levelball: 160,
  lureball: 161,
  fastball: 162,
  lightball: 164,
  friendball: 165,
  moonball: 166,
  loveball: 167,
  sunstone: 170,
  polkadotbow: 171,
  upgrade: 173,
  berry: 174,
  goldberry: 175,
};

function toId(s) {
  return String(s || '')
    .normalize('NFD')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '');
}

function encodeMonForPdinput(s) {
  const spId = toId(s.species || s.name || '');
  const sp = dex.species[spId];
  if (!sp) throw new Error(`Unknown species: ${s.species || s.name}`);
  const jpSpecies = i18n.species[spId];
  if (!jpSpecies) throw new Error(`Missing JP species for ${spId}`);

  const level = typeof s.level === 'number' ? s.level : 50;
  const ivs = s.ivs || {};
  const atkDv = Math.max(0, Math.min(15, Math.floor((ivs.atk ?? 31) / 2)));
  const defDv = Math.max(0, Math.min(15, Math.floor((ivs.def ?? 31) / 2)));
  const speDv = Math.max(0, Math.min(15, Math.floor((ivs.spe ?? 31) / 2)));
  const spcDv = Math.max(0, Math.min(15, Math.floor((ivs.spa ?? 31) / 2)));
  const hpDv = ((atkDv & 1) << 3) | ((defDv & 1) << 2) | ((speDv & 1) << 1) | (spcDv & 1);

  const evs = s.evs || {};
  const doHp = Math.max(0, Math.min(63, Math.floor((evs.hp ?? 255) / 4)));
  const doAtk = Math.max(0, Math.min(63, Math.floor((evs.atk ?? 255) / 4)));
  const doDef = Math.max(0, Math.min(63, Math.floor((evs.def ?? 255) / 4)));
  const doSpe = Math.max(0, Math.min(63, Math.floor((evs.spe ?? 255) / 4)));
  const doSpc = Math.max(0, Math.min(63, Math.floor((evs.spa ?? 255) / 4)));

  const rawMoves = Array.isArray(s.moves) ? s.moves : [];
  const moveCodes = [];
  const movePps = [];
  const ppUps = [0, 0, 0, 0];
  const jpMoves = ['', '', '', ''];

  for (let i = 0; i < 4; i++) {
    const rawMove = rawMoves[i];
    if (rawMove) {
      let mId = toId(rawMove);
      if (mId.startsWith('hiddenpower')) mId = 'hiddenpower';
      const mv = dex.moves[mId];
      if (!mv) throw new Error(`Unknown move: ${rawMove}`);
      const jpMv = i18n.moves[mId];
      if (!jpMv) throw new Error(`Missing JP move for ${mId}`);
      const ppUp = 3;
      moveCodes.push(mv.num + 1);
      movePps.push(mv.pp + Math.floor(mv.pp / 5) * ppUp);
      ppUps[i] = ppUp;
      jpMoves[i] = jpMv;
    } else {
      moveCodes.push(0);
      movePps.push(0);
      ppUps[i] = 0;
      jpMoves[i] = '';
    }
  }

  const baseHp = sp.baseStats.hp;
  const maxHp = Math.floor(((baseHp + hpDv) * 2 + doHp) * level / 100) + level + 10;

  const itemId = toId(s.item || '');
  const itemCode = PDINPUT_ITEM_CODES[itemId];
  if (itemCode === undefined) throw new Error(`Unsupported item: ${s.item}`);
  const jpItem = itemId ? (i18n.items[itemId] || '') : '';

  const hex = n => n.toString(16).toUpperCase();
  const koHex = `${hex(atkDv)}${hex(defDv)}${hex(speDv)}${hex(spcDv)}`;

  const fields = [
    sp.num,
    level,
    hex(atkDv),
    hex(defDv),
    hex(speDv),
    hex(spcDv),
    doHp,
    doAtk,
    doDef,
    doSpe,
    doSpc,
    ...moveCodes,
    ...movePps,
    ...ppUps,
    maxHp,
    0,
    itemCode,
  ];

  return {
    record: fields.join('_'),
    speciesId: spId,
    jpSpecies,
    level,
    jpMoves,
    ppUps,
    jpItem,
    doValues: [doHp, doAtk, doDef, doSpe, doSpc],
    koHex,
    maxHp,
  };
}

function encodeTeamForPdinput(team) {
  const mons = team.sets.map(encodeMonForPdinput);
  return {
    id: team.id,
    tier: team.tier || '',
    cookieValue: '1_x_' + mons.map(m => m.record).join('_x_'),
    firstFivePd: mons.slice(0, 5).map(m => m.record).join('_x_'),
    sixthMon: mons[5],
    mons,
  };
}

function renderStandaloneHtml(encodedTeams) {
  const jsonPayload = JSON.stringify(encodedTeams);
  return `<!doctype html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>NC2000 — psense.lib.net/_/PDINPUT.cgi パーティプール自動インポーター</title>
<style>
  body { background: #f6f3ec; color: #282b27; font: 15px/1.65 system-ui, sans-serif; max-width: 1020px; margin: auto; padding: 24px; }
  h1 { font-size: 1.5rem; margin-bottom: 0.4rem; }
  h2 { font-size: 1.15rem; margin-top: 0; }
  .card { background: #fff; border: 1px solid #d5d2c8; border-radius: 10px; padding: 16px 18px; margin-bottom: 16px; }
  .btn { cursor: pointer; padding: 8px 14px; border-radius: 6px; border: 1px solid #385c42; background: #fff; color: #282b27; font-weight: 600; font-size: 0.92rem; }
  .btn.primary { background: #385c42; color: #fff; }
  .btn.active { background: #385c42; color: #fff; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .row { display: flex; gap: 10px; flex-wrap: wrap; align-items: center; }
  .note { color: #555; font-size: 0.9rem; margin: 6px 0; }
  .status { margin-top: 10px; padding: 10px 12px; border-radius: 6px; background: #eaece2; font-weight: 600; }
  .team-item { border: 1px solid #dcd9cf; border-radius: 8px; padding: 10px 12px; margin-bottom: 8px; background: #faf9f5; }
  .mon-chip { display: inline-block; background: #eaece2; padding: 2px 8px; border-radius: 4px; font-size: 0.85rem; margin: 2px 4px 2px 0; }
  textarea { width: 100%; box-sizing: border-box; font-family: monospace; font-size: 0.8rem; padding: 8px; border-radius: 6px; border: 1px solid #ccc; }
</style>
</head>
<body>
<h1>psense.lib.net/_/PDINPUT.cgi パーティプール自動インポーター</h1>
<p class="note">標準パーティプール（<code>team-pool-v2</code> 全33構築）を <code>PDINPUT.cgi</code> の26フィールド <code>PD</code> Cookie 形式に変換し、ブラウザへ全自動登録します。<br>
※ Apache 1.3.39 の Cookie ヘッダ上限（8,190 bytes）により1度に保持できる上限は <strong>最大17パーティ（約7.7KB）</strong> です。前半（1〜17）または後半（18〜33）を選んで取り込んでください。</p>

<div class="card">
  <h2>バッチ選択（最大17構築）</h2>
  <div class="row" id="batch-buttons"></div>
  <p class="note" id="selection-summary"></p>
</div>

<div class="card">
  <h2>1. フォーム自動送信でブラウザに取り込む（推奨）</h2>
  <p class="note">サブウィンドウを開き、<code>PDINPUT.cgi</code> の 1-POST 保存エンドポイント（<code>ACTION=NEW&amp;ACTION2=SAVED&amp;PBAN=1..N</code>, EUC-JP）へ順番に自動送信して <code>PD0〜PD16</code> に書き込みます。</p>
  <div class="row">
    <button class="btn primary" id="start-btn">選択中のパーティを全自動取り込み</button>
    <button class="btn" id="stop-btn" style="display:none">停止</button>
    <label class="note">送信間隔(ms): <input type="number" id="step-ms" value="900" min="300" max="5000" step="100" style="width:70px"></label>
    <a class="btn" href="${PDINPUT_URL}" target="_blank" rel="noopener">PDINPUT.cgi を開く ↗</a>
  </div>
  <div class="status" id="run-status" style="display:none"></div>
</div>

<div class="card">
  <h2>2. ワンライナー / ブックマークレットで一括反映（1秒で完了）</h2>
  <p class="note"><code>PDINPUT.cgi</code> を開いているタブで実行すると、古いスロットをクリアして選択中の全パーティを瞬時に Cookie へ書き込みます。</p>
  <div class="row" style="margin-bottom:8px">
    <button class="btn" id="copy-btn">ワンライナー (javascript:...) をコピー</button>
    <a class="btn" id="bookmarklet-link" href="#">PDINPUT一括登録ブックマークレット</a>
  </div>
  <textarea id="script-box" rows="3" readonly></textarea>
</div>

<div class="card">
  <h2>パーティ一覧（全 ${encodedTeams.length} 構築）</h2>
  <div id="team-list"></div>
</div>

<script>
const TEAMS = ${jsonPayload};
const URL_TARGET = ${JSON.stringify(PDINPUT_URL)};
const MAX_BATCH = ${PDINPUT_MAX_BATCH_SIZE};
const LIMIT_BYTES = ${PDINPUT_COOKIE_LIMIT_BYTES};

let selectedIds = TEAMS.slice(0, MAX_BATCH).map(t => t.id);
let timer = null;
let targetWin = null;

function cookieBytes(list) {
  if (!list.length) return 0;
  return list.map((t, i) => 'PD' + i + '=' + t.cookieValue).join('; ').length;
}

function submitEucJp(fields, winName) {
  const f = document.createElement('form');
  f.method = 'POST';
  f.action = URL_TARGET;
  f.acceptCharset = 'EUC-JP';
  f.target = winName;
  f.style.display = 'none';
  for (const [k, v] of Object.entries(fields)) {
    const inp = document.createElement('input');
    inp.type = 'hidden';
    inp.name = k;
    inp.value = v;
    f.appendChild(inp);
  }
  document.body.appendChild(f);
  f.submit();
  document.body.removeChild(f);
}

function singlePostFields(team, pban) {
  const m6 = team.sixthMon;
  return {
    sim: '', ACTION: 'NEW', ACTION2: 'SAVED',
    PD: team.firstFivePd, PC: '5', PBAN: String(pban),
    LV: String(m6.level), SYU: m6.jpSpecies,
    WAZA1: m6.jpMoves[0], PPUP1: String(m6.ppUps[0]),
    WAZA2: m6.jpMoves[1], PPUP2: String(m6.ppUps[1]),
    WAZA3: m6.jpMoves[2], PPUP3: String(m6.ppUps[2]),
    WAZA4: m6.jpMoves[3], PPUP4: String(m6.ppUps[3]),
    ITEM: m6.jpItem,
    DO1: String(m6.doValues[0]), DO2: String(m6.doValues[1]),
    DO3: String(m6.doValues[2]), DO4: String(m6.doValues[3]),
    DO5: String(m6.doValues[4]), KO: m6.koHex,
    B1: '完了'
  };
}

function render() {
  const selected = selectedIds.map(id => TEAMS.find(t => t.id === id)).filter(Boolean);
  const bytes = cookieBytes(selected);
  document.getElementById('selection-summary').textContent =
    '選択中: ' + selected.length + ' / ' + MAX_BATCH + ' 構築 (Cookieヘッダ: ' + bytes.toLocaleString() + ' / ' + LIMIT_BYTES.toLocaleString() + ' bytes)';
  document.getElementById('start-btn').textContent = '選択中の ' + selected.length + ' パーティを全自動取り込み';
  document.getElementById('start-btn').disabled = selected.length === 0 || timer !== null;

  const entries = selected.map(t => t.cookieValue);
  const script = "javascript:(()=>{if(location.hostname!=='psense.lib.net'){alert('psense.lib.net/_/PDINPUT.cgi で実行してください');return;}const p=" + JSON.stringify(entries) + ";const exp='expires=Tue, 19 Jan 2038 03:14:07 GMT; path=/_/';for(let i=0;i<35;i++){document.cookie='PD'+i+'=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/_/';}p.forEach((v,i)=>{document.cookie='PD'+i+'='+v+'; '+exp;});location.href='http://psense.lib.net/_/PDINPUT.cgi';})()";
  document.getElementById('script-box').value = script;
  document.getElementById('bookmarklet-link').href = script;

  const listEl = document.getElementById('team-list');
  listEl.innerHTML = '';
  TEAMS.forEach((team, idx) => {
    const slotIdx = selectedIds.indexOf(team.id);
    const checked = slotIdx !== -1;
    const slotNum = checked ? slotIdx + 1 : (idx % MAX_BATCH) + 1;
    const div = document.createElement('div');
    div.className = 'team-item';
    div.innerHTML = '<div class="row" style="justify-content:space-between;margin-bottom:6px">' +
      '<label style="font-weight:600;cursor:pointer"><input type="checkbox" data-id="' + team.id + '" ' + (checked ? 'checked' : '') + ' ' + (!checked && selectedIds.length >= MAX_BATCH ? 'disabled' : '') + '> #' + (idx + 1) + ' ' + team.id + (checked ? ' → PD' + slotIdx + ' (No.' + slotNum + ')' : '') + '</label>' +
      '<div class="row"><button class="btn" data-write="' + team.id + '" data-slot="' + slotNum + '">No.' + slotNum + ' に上書き送信</button><button class="btn" data-recept="' + team.id + '">空き枠へ再現送信 (RECEPT)</button></div>' +
      '</div>' +
      '<div>' + team.mons.map(m => '<span class="mon-chip">' + m.jpSpecies + ' Lv' + m.level + ' (' + (m.jpItem || 'なし') + ') [' + m.jpMoves.filter(Boolean).join('/') + ']</span>').join('') + '</div>';
    listEl.appendChild(div);
  });
}

document.getElementById('batch-buttons').innerHTML = [0, 1].map(bIdx => {
  const start = bIdx * MAX_BATCH;
  const sub = TEAMS.slice(start, start + MAX_BATCH);
  return '<button class="btn" data-batch="' + bIdx + '">パート ' + (bIdx + 1) + ' (#' + (start + 1) + '〜#' + (start + sub.length) + ' · ' + sub.length + '構築 · ' + cookieBytes(sub).toLocaleString() + ' B)</button>';
}).join('');

document.getElementById('batch-buttons').addEventListener('click', e => {
  const b = e.target.closest('[data-batch]');
  if (!b) return;
  const idx = Number(b.dataset.batch);
  selectedIds = TEAMS.slice(idx * MAX_BATCH, (idx + 1) * MAX_BATCH).map(t => t.id);
  render();
});

document.getElementById('team-list').addEventListener('change', e => {
  const id = e.target.dataset.id;
  if (!id) return;
  if (selectedIds.includes(id)) selectedIds = selectedIds.filter(x => x !== id);
  else if (selectedIds.length < MAX_BATCH) selectedIds = [...selectedIds, id];
  render();
});

document.getElementById('team-list').addEventListener('click', e => {
  const wBtn = e.target.closest('[data-write]');
  if (wBtn) {
    const team = TEAMS.find(t => t.id === wBtn.dataset.write);
    const slot = Number(wBtn.dataset.slot);
    window.open('about:blank', 'nc2000_pdinput_target');
    submitEucJp(singlePostFields(team, slot), 'nc2000_pdinput_target');
    return;
  }
  const rBtn = e.target.closest('[data-recept]');
  if (rBtn) {
    const team = TEAMS.find(t => t.id === rBtn.dataset.recept);
    window.open('about:blank', 'nc2000_pdinput_target');
    submitEucJp({ ACTION: 'RECEPT', C: team.cookieValue }, 'nc2000_pdinput_target');
  }
});

document.getElementById('copy-btn').addEventListener('click', async () => {
  const box = document.getElementById('script-box');
  box.select();
  try { await navigator.clipboard.writeText(box.value); } catch {}
});

document.getElementById('start-btn').addEventListener('click', () => {
  const selected = selectedIds.map(id => TEAMS.find(t => t.id === id)).filter(Boolean);
  if (!selected.length) return;
  const winName = 'nc2000_pdinput_target';
  targetWin = window.open('about:blank', winName);
  const statusEl = document.getElementById('run-status');
  statusEl.style.display = 'block';
  if (!targetWin) {
    statusEl.textContent = 'ポップアップがブロックされました。許可して再実行してください。';
    return;
  }
  document.getElementById('stop-btn').style.display = 'inline-block';
  const stepMs = Math.max(300, Number(document.getElementById('step-ms').value) || 900);
  let i = 0;
  function step() {
    if (i >= selected.length) {
      timer = null;
      document.getElementById('stop-btn').style.display = 'none';
      statusEl.textContent = '完了！ ' + selected.length + ' パーティを PD0〜PD' + (selected.length - 1) + ' に登録しました。';
      try { targetWin.location.href = URL_TARGET; } catch { window.open(URL_TARGET, winName); }
      render();
      return;
    }
    const team = selected[i];
    statusEl.textContent = '取り込み中 (' + (i + 1) + ' / ' + selected.length + '): ' + team.id + ' を PD' + i + ' (No.' + (i + 1) + ') に書き込み中…';
    submitEucJp(singlePostFields(team, i + 1), winName);
    i++;
    timer = setTimeout(step, stepMs);
    render();
  }
  step();
});

document.getElementById('stop-btn').addEventListener('click', () => {
  if (timer) { clearTimeout(timer); timer = null; }
  document.getElementById('stop-btn').style.display = 'none';
  document.getElementById('run-status').textContent = '停止しました。';
  render();
});

render();
</script>
</body>
</html>`;
}

if (require.main === module) {
  const encodedTeams = pool.teams.map(encodeTeamForPdinput);
  const args = process.argv.slice(2);
  const outArg = args.find(a => !a.startsWith('--'));
  if (outArg) {
    const outPath = path.resolve(outArg);
    fs.mkdirSync(path.dirname(outPath), { recursive: true });
    fs.writeFileSync(outPath, renderStandaloneHtml(encodedTeams), 'utf8');
    console.log(`Wrote standalone PDINPUT importer (${encodedTeams.length} teams) to ${outPath}`);
  } else {
    console.log(`Encoded ${encodedTeams.length} teams cleanly.`);
    const p1 = encodedTeams.slice(0, PDINPUT_MAX_BATCH_SIZE);
    const p2 = encodedTeams.slice(PDINPUT_MAX_BATCH_SIZE);
    const b1 = p1.map((t, i) => `PD${i}=${t.cookieValue}`).join('; ').length;
    const b2 = p2.map((t, i) => `PD${i}=${t.cookieValue}`).join('; ').length;
    console.log(`Batch 1: ${p1.length} teams (${b1} bytes)`);
    console.log(`Batch 2: ${p2.length} teams (${b2} bytes)`);
  }
}

module.exports = {
  PDINPUT_ITEM_CODES,
  encodeMonForPdinput,
  encodeTeamForPdinput,
  renderStandaloneHtml,
};
