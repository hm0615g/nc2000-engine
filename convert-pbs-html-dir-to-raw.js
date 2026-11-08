#!/usr/bin/env node
'use strict';

/*
  convert-pbs-html-dir-to-raw.js

  Convert the current PokemonBattle simulator saved HTML logs into Pokemon Showdown
  battle protocol raw.log files for the NC2000 spectator pipeline.

  Design rules:
    - Do not silently guess unknown Japanese battle messages.
    - Write one raw.log per completed HTML battle that has a WIN marker.
    - Skip incomplete/no-win logs by default.
    - Keep HP state per side + species and use it when switching back in.
    - Emit audit files for every input.
    - Read raw_log.dict when provided and use it as a required reference artifact;
      the protocol forms implemented below follow that dictionary's examples.
*/

const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const {TextDecoder} = require('util');
const vm = require('vm');

const VERSION = '2026-09-19-v32';
const TIER = '[Gen 2] Nintendo Cup 2000 No OHKO Stadium2 Strict';
const BATTLE_PREFIX = 'battle-gen2nintendocup2000noohkostadium2strict-pbs';
const SYNTHETIC_TIMESTAMP_BASE = Math.floor(Date.UTC(2000, 0, 1, 0, 0, 0) / 1000);
const SYNTHETIC_TIMESTAMP_SPAN_SECONDS = 365 * 24 * 60 * 60;

function usage() {
  console.log(`Usage:
  node convert-pbs-html-dir-to-raw.js --input-dir ./html --out-dir ./out --dict ./raw_log.dict

Options:
  --input-dir DIR          Directory containing saved PokemonBattle HTML files. Required.
  --out-dir DIR            Output directory. Required.
  --dict FILE              raw_log.dict reference file. Strongly recommended; required unless --allow-no-dict.
  --ja-dict FILE           Optional local-ja-dict.js containing pokemon/moves/items Japanese names.
  --recursive              Search input-dir recursively.
  --include-incomplete     Also output raw.log for logs without a WIN marker. No |win| is synthesized.
  --strict                 Treat warnings as failures and do not output raw for warned battles.
  --battle-prefix PREFIX   Override battle id prefix. Default: ${BATTLE_PREFIX}
  --missing-timestamp-policy fail|skip|synthetic
                           How to handle PBS HTML logs without a timestamp in URL/name. Default: synthetic.
  --help                   Show this help.

Output:
  OUT/raw/*.raw.log
  OUT/audit/summary.json
  OUT/audit/warnings.txt
  OUT/audit/failed.txt
  OUT/audit/skipped.txt
  OUT/audit/per-battle/*.audit.txt
`);
}

function parseArgs(argv) {
  const args = {
    recursive: false,
    includeIncomplete: false,
    strict: false,
    allowNoDict: false,
    battlePrefix: BATTLE_PREFIX,
    missingTimestampPolicy: 'synthetic',
  };
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--help' || a === '-h') { args.help = true; continue; }
    if (a === '--recursive') { args.recursive = true; continue; }
    if (a === '--include-incomplete') { args.includeIncomplete = true; continue; }
    if (a === '--strict') { args.strict = true; continue; }
    if (a === '--allow-no-dict') { args.allowNoDict = true; continue; }
    if (a === '--input-dir') { args.inputDir = argv[++i]; continue; }
    if (a === '--out-dir') { args.outDir = argv[++i]; continue; }
    if (a === '--dict') { args.dict = argv[++i]; continue; }
    if (a === '--ja-dict') { args.jaDict = argv[++i]; continue; }
    if (a === '--battle-prefix') { args.battlePrefix = argv[++i]; continue; }
    if (a === '--missing-timestamp-policy') {
      args.missingTimestampPolicy = argv[++i];
      if (!['fail', 'skip', 'synthetic'].includes(args.missingTimestampPolicy)) {
        throw new Error('Invalid --missing-timestamp-policy: ' + args.missingTimestampPolicy);
      }
      continue;
    }
    throw new Error('Unknown option: ' + a);
  }
  return args;
}

function ensureDir(dir) { fs.mkdirSync(dir, {recursive: true}); }
function readFileBytes(p) { return fs.readFileSync(p); }

function decodeHtmlFile(file) {
  const buf = readFileBytes(file);
  let ascii = buf.toString('latin1');
  let charset = 'utf-8';
  const m = ascii.match(/charset\s*=\s*([A-Za-z0-9_-]+)/i);
  if (m) charset = m[1].toLowerCase();
  // The saved files declare EUC-JP. Node's TextDecoder supports euc-jp in this runtime.
  const candidates = charset.includes('euc') ? ['euc-jp', 'utf-8'] : ['utf-8', 'euc-jp'];
  let best = null;
  for (const enc of candidates) {
    try {
      const text = new TextDecoder(enc, {fatal: false}).decode(buf);
      const score = mojibakeScore(text);
      if (!best || score < best.score) best = {text, encoding: enc, score};
    } catch (e) {}
  }
  if (!best) best = {text: buf.toString('utf8'), encoding: 'utf-8', score: 999};
  return best;
}

function mojibakeScore(s) {
  let score = 0;
  score += (s.match(/�/g) || []).length * 10;
  score += (s.match(/[仄丐卞凶毛楞請勻]/g) || []).length;
  score -= (s.match(/[ぁ-んァ-ン一-龯]/g) || []).length * 0.01;
  return score;
}

function htmlDecode(s) {
  return String(s || '')
    .replace(/&nbsp;/g, ' ')
    .replace(/&amp;/g, '&')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&#(\d+);/g, (_, n) => String.fromCharCode(Number(n)))
    .replace(/&#x([0-9a-f]+);/ig, (_, n) => String.fromCharCode(parseInt(n, 16)));
}

function stripTagsToText(html) {
  return htmlDecode(String(html || '')
    .replace(/<!--([\s\S]*?)-->/g, '')
    .replace(/<br\s*\/?\s*>/gi, '\n')
    .replace(/<[^>]+>/g, '')
    .replace(/\r/g, '')
    .replace(/[ \t]+/g, ' ')
    .replace(/\n[ \t]+/g, '\n')
    .replace(/[ \t]+\n/g, '\n')
    .trim());
}

function normalizeJa(s) {
  return String(s || '')
    .replace(/[−－―‐]/g, '−')
    .replace(/〜/g, '〜')
    .replace(/　/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}
function normalizeNameKey(s) {
  return normalizeJa(s).replace(/ /g, '').replace(/−/g, 'ー');
}

const SPECIES = {
  'サンダー': 'Zapdos', 'サンダ−': 'Zapdos',
  'バンギラス': 'Tyranitar', 'ガラガラ': 'Marowak', 'カビゴン': 'Snorlax',
  'ナッシー': 'Exeggutor', 'ナッシ−': 'Exeggutor', 'ゲンガー': 'Gengar', 'ゲンガ−': 'Gengar',
  'サンドパン': 'Sandslash', 'ピクシー': 'Clefable', 'ピクシ−': 'Clefable',
  'キュウコン': 'Ninetales', 'フーディン': 'Alakazam', 'フ−ディン': 'Alakazam',
  'パルシェン': 'Cloyster', 'フシギバナ': 'Venusaur', 'エレブー': 'Electabuzz', 'エレブ−': 'Electabuzz',
  'ポリゴン２': 'Porygon2', 'ポリゴン2': 'Porygon2', 'エアームド': 'Skarmory', 'エア−ムド': 'Skarmory',
  'サイドン': 'Rhydon', 'ヘラクロス': 'Heracross', 'ミニリュウ': 'Dratini', 'ハクリュー': 'Dragonair', 'ハクリュ−': 'Dragonair',
  'ライコウ': 'Raikou', 'ムウマ': 'Misdreavus', 'スターミー': 'Starmie', 'スタ−ミ−': 'Starmie',
  'ブラッキー': 'Umbreon', 'ブラッキ−': 'Umbreon', 'サンダース': 'Jolteon', 'サンダ−ス': 'Jolteon',
  'ドンファン': 'Donphan', 'ヘルガー': 'Houndoom', 'ハッサム': 'Scizor', 'カイリキー': 'Machamp', 'カイリュー': 'Dragonite', 'カイリュ−': 'Dragonite',
  'ライチュウ': 'Raichu', 'ハガネール': 'Steelix', 'ハガネ−ル': 'Steelix', 'シャワーズ': 'Vaporeon', 'シャワ−ズ': 'Vaporeon',
  'フーデイン': 'Alakazam', 'ケンタロス': 'Tauros', 'ラプラス': 'Lapras', 'ミルタンク': 'Miltank',
  'フシギソウ': 'Ivysaur', 'フシギダネ': 'Bulbasaur', 'リザードン': 'Charizard', 'バクフーン': 'Typhlosion',
  'スイクン': 'Suicune', 'サニーゴ': 'Corsola', 'ニドキング': 'Nidoking', 'ニドクイン': 'Nidoqueen',
  'オムスター': 'Omastar', 'オムスタ−': 'Omastar', 'オムスタ－': 'Omastar', 'レディアン': 'Ledian', 'ハピナス': 'Blissey',
  'ピカチュウ': 'Pikachu', 'ニョロボン': 'Poliwrath', 'ウツドン': 'Weepinbell', 'ウツボット': 'Victreebel',
  'エーフィ': 'Espeon', 'エ−フィ': 'Espeon', 'プテラ': 'Aerodactyl', 'ルージュラ': 'Jynx', 'ゴースト': 'Haunter',
  'ゲンガー': 'Gengar', 'モルフォン': 'Venomoth', 'ドードリオ': 'Dodrio', 'ド−ドリオ': 'Dodrio',
  'ウインディ': 'Arcanine', 'オーダイル': 'Feraligatr', 'オ−ダイル': 'Feraligatr', 'ゴローニャ': 'Golem', 'ゴロ−ニャ': 'Golem',
  'マルマイン': 'Electrode', 'ワタッコ': 'Jumpluff', 'ゴース': 'Gastly', 'メタモン': 'Ditto',
  'ニドリーノ': 'Nidorino', 'ニドリ−ノ': 'Nidorino', 'キングドラ': 'Kingdra', 'ハリーセン': 'Qwilfish', 'ハリ−セン': 'Qwilfish',
  'ニューラ': 'Sneasel', 'ニュ−ラ': 'Sneasel', 'フォレトス': 'Forretress', 'グランブル': 'Granbull', 'サワムラー': 'Hitmonlee',
  'デルビル': 'Houndour', 'ベイリーフ': 'Bayleef', 'ケーシィ': 'Abra', 'ケ−シィ': 'Abra', 'ユンゲラー': 'Kadabra', 'ユンゲラ−': 'Kadabra',
  'ファイヤー': 'Moltres', 'ファイヤ−': 'Moltres', 'オニドリル': 'Fearow', 'ヤドラン': 'Slowbro', 'ヤドキング': 'Slowking',
  'ニョロゾ': 'Poliwhirl', 'カブトプス': 'Kabutops', 'クサイハナ': 'Gloom', 'ジュゴン': 'Dewgong', 'ブースター': 'Flareon', 'ブ−スタ−': 'Flareon', 'ブ－スタ－': 'Flareon',
  'ニョロモ': 'Poliwag', 'ラフレシア': 'Vileplume', 'イーブイ': 'Eevee', 'イ−ブイ': 'Eevee', 'イ－ブイ': 'Eevee',
};

const MOVES = {
  'だいばくはつ': 'Explosion', 'じばく': 'Self-Destruct', 'じしん': 'Earthquake', 'のしかかり': 'Body Slam',
  'すてみタックル': 'Double-Edge', 'おんがえし': 'Return', 'やつあたり': 'Frustration', 'いびき': 'Snore', 'かえんほうしゃ': 'Flamethrower', 'だいもんじ': 'Fire Blast',
  'こうそくスピン': 'Rapid Spin', 'とびひざげり': 'High Jump Kick', 'とびげり': 'Jump Kick',
  'いわなだれ': 'Rock Slide', 'じわれ': 'Fissure', 'つのドリル': 'Horn Drill', 'ハサミギロチン': 'Guillotine', 'ホネブーメラン': 'Bonemerang', 'ホネブ−メラン': 'Bonemerang', 'めざめるパワー': 'Hidden Power', 'めざめるパワ−': 'Hidden Power',
  'つるぎのまい': 'Swords Dance', 'のろい': 'Curse', 'はらだいこ': 'Belly Drum', 'ねむる': 'Rest', 'ねごと': 'Sleep Talk', 'こらえる': 'Endure', 'へんしん': 'Transform', 'きあいだめ': 'Focus Energy', 'ゆびをふる': 'Metronome',
  'まもる': 'Protect', 'ふきとばし': 'Whirlwind', 'ほえる': 'Roar', 'リフレクター': 'Reflect', 'ひかりのかべ': 'Light Screen',
  'どくどく': 'Toxic', 'どくばり': 'Poison Sting', 'でんじは': 'Thunder Wave', 'あやしいひかり': 'Confuse Ray', 'メロメロ': 'Attract',
  'みがわり': 'Substitute', 'ほろびのうた': 'Perish Song', 'まきびし': 'Spikes', 'やどりぎのタネ': 'Leech Seed',
  'みちづれ': 'Destiny Bond', 'くろいまなざし': 'Mean Look', 'バトンタッチ': 'Baton Pass', 'かげぶんしん': 'Double Team',
  'サイコキネシス': 'Psychic', 'サイコキネシス ': 'Psychic', 'ギガドレイン': 'Giga Drain', 'ねむりごな': 'Sleep Powder',
  'メガホーン': 'Megahorn', 'メガホ−ン': 'Megahorn', 'クロスチョップ': 'Cross Chop', 'ドリルくちばし': 'Drill Peck',
  '１０まんボルト': 'Thunderbolt', '10まんボルト': 'Thunderbolt', 'かみなり': 'Thunder', 'れいとうビーム': 'Ice Beam', 'れいとうビ−ム': 'Ice Beam',
  'ふぶき': 'Blizzard', 'なみのり': 'Surf', 'ハイドロポンプ': 'Hydro Pump', 'こごえるかぜ': 'Icy Wind', 'ソーラービーム': 'Solar Beam', 'ソ−ラ−ビ−ム': 'Solar Beam', 'ソ－ラ－ビ－ム': 'Solar Beam', 'かまいたち': 'Razor Wind',
  'シャドーボール': 'Shadow Ball', 'シャド−ボ−ル': 'Shadow Ball', 'ばくれつパンチ': 'Dynamic Punch',
  'おいうち': 'Pursuit', 'とける': 'Acid Armor', 'いやなおと': 'Screech', 'すなあらし': 'Sandstorm', 'にほんばれ': 'Sunny Day', 'あまごい': 'Rain Dance', 'くろいきり': 'Haze', 'どろぼう': 'Thief', 'じこさいせい': 'Recover', 'あまえる': 'Charm', 'いばる': 'Swagger', 'いやしのすず': 'Heal Bell', 'いたみわけ': 'Pain Split',
};

const TYPE_MAP = {
  'ノーマル': 'Normal', 'ノ−マル': 'Normal', 'ほのお': 'Fire', 'みず': 'Water', 'でんき': 'Electric',
  'くさ': 'Grass', 'こおり': 'Ice', 'かくとう': 'Fighting', 'どく': 'Poison', 'じめん': 'Ground',
  'ひこう': 'Flying', 'エスパー': 'Psychic', 'エスパ−': 'Psychic', 'むし': 'Bug', 'いわ': 'Rock',
  'ゴースト': 'Ghost', 'ゴ−スト': 'Ghost', 'ドラゴン': 'Dragon', 'あく': 'Dark', 'はがね': 'Steel',
};
function jpTypeToPs(jp) {
  const key = normalizeNameKey(jp);
  return TYPE_MAP[key] || TYPE_MAP[normalizeJa(jp)] || null;
}

const ITEM_MAP = {
  'おうごんのみ': 'Gold Berry',
  'はっかのみ': 'Mint Berry',
  'きせきのみ': 'Miracle Berry',
  'たべのこし': 'Leftovers',
  'きのみジュース': 'Berry Juice',
  'きのみ': 'Berry',
  'まひなおしのみ': 'PRZ Cure Berry',
};

function jpSpeciesToPs(jp) {
  const key = normalizeNameKey(jp);
  if (SPECIES[key]) return SPECIES[key];
  // Try original normalized string
  if (SPECIES[normalizeJa(jp)]) return SPECIES[normalizeJa(jp)];
  return null;
}
function jpMoveToPs(jp) {
  const key = normalizeNameKey(jp);
  if (MOVES[key]) return MOVES[key];
  if (MOVES[normalizeJa(jp)]) return MOVES[normalizeJa(jp)];
  return null;
}
function jpItemToPs(jp) {
  const key = normalizeNameKey(jp);
  return ITEM_MAP[key] || ITEM_MAP[normalizeJa(jp)] || null;
}

function loadJaDict(file) {
  if (!file || !fs.existsSync(file)) return null;
  const text = fs.readFileSync(file, 'utf8');
  if (file.endsWith('.json')) {
    const data = JSON.parse(text);
    const dexPath = path.join(path.dirname(file), 'gen2stadium2.json');
    const dex = fs.existsSync(dexPath) ? JSON.parse(fs.readFileSync(dexPath, 'utf8')) : {species: {}, moves: {}, items: {}};
    for (const [id, ja] of Object.entries(data.species || {})) {
      const psName = dex.species[id]?.name || id;
      SPECIES[normalizeNameKey(ja)] = psName;
      SPECIES[normalizeJa(ja)] = psName;
    }
    for (const [id, ja] of Object.entries(data.moves || {})) {
      if (id.startsWith('hiddenpower') && id !== 'hiddenpower') continue;
      const psName = dex.moves[id]?.name || id;
      MOVES[normalizeNameKey(ja)] = psName;
      MOVES[normalizeJa(ja)] = psName;
    }
    MOVES[normalizeNameKey('めざめるパワー')] = 'Hidden Power';
    MOVES[normalizeJa('めざめるパワー')] = 'Hidden Power';
    for (const [id, ja] of Object.entries(data.items || {})) {
      const psName = dex.items[id]?.name || id;
      ITEM_MAP[normalizeNameKey(ja)] = psName;
      ITEM_MAP[normalizeJa(ja)] = psName;
    }
    return file;
  }
  const sandbox = {window: {}};
  vm.runInNewContext(text, sandbox, {timeout: 1000});
  const data = sandbox.window && sandbox.window.NC2000_JA_NAMES;
  if (!data) throw new Error('ja-dict does not define window.NC2000_JA_NAMES: ' + file);
  for (const [en, ja] of Object.entries(data.pokemon || {})) {
    const base = en.replace(/-(Mega|Gmax|Alola|Galar|Hisui|Totem|Starter|Original|Partner|Belle|Cosplay|Hoenn|Kalos|Libre|PhD|Pop-Star|Rock-Star|Sinnoh|Unova|World|Paldea-Aqua|Paldea-Blaze|Paldea-Combat).*$/, '');
    const key = normalizeNameKey(ja);
    if (!SPECIES[key]) SPECIES[key] = base;
  }
  for (const [en, ja] of Object.entries(data.moves || {})) {
    const key = normalizeNameKey(ja);
    if (!MOVES[key]) MOVES[key] = en;
  }
  for (const [en, ja] of Object.entries(data.items || {})) {
    const key = normalizeNameKey(ja);
    if (!ITEM_MAP[key]) ITEM_MAP[key] = en;
  }
  return file;
}

function slug(s) {
  return String(s || '')
    .replace(/\.html?$/i, '')
    .replace(/[\s_]+/g, '-')
    .replace(/[^A-Za-z0-9一-龯ぁ-んァ-ンー\-]+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '') || 'battle';
}
function safeFile(s) { return slug(s).replace(/[一-龯ぁ-んァ-ンー]/g, c => encodeURIComponent(c).replace(/%/g, '')); }

function listHtmlFiles(dir, recursive) {
  const out = [];
  function walk(d) {
    for (const ent of fs.readdirSync(d, {withFileTypes: true})) {
      const p = path.join(d, ent.name);
      if (ent.isDirectory()) { if (recursive) walk(p); continue; }
      if (ent.isFile() && /\.html?$/i.test(ent.name)) out.push(p);
    }
  }
  walk(dir);
  out.sort();
  return out;
}

function extractSavedUrl(html) {
  const m = html.match(/saved from url=\(\d+\)([^\s"<>]+)/i);
  return m ? htmlDecode(m[1]) : '';
}
function fileStemFromSavedUrl(file, html) {
  const url = extractSavedUrl(html);
  const m = url.match(/\/logs\/([^\/]+)\.html/i);
  if (m) return m[1];
  return path.basename(file).replace(/\.html?$/i, '');
}

function sha256Hex(s) { return crypto.createHash('sha256').update(String(s || ''), 'utf8').digest('hex'); }

function roomFromSavedUrl(html) {
  const url = extractSavedUrl(html);
  const m = url.match(/\/PBS\/_\/([A-H])\/PS\.cgi/i);
  return m ? m[1].toUpperCase() : 'X';
}

function stemHasPbsTimestamp(stem) {
  return /(?:^|[-_])(20\d{6})[-_](\d{4}|\d{6})(?:[-_]|$)/.test(String(stem || ''));
}

function syntheticTimestampSeconds(hashHex) {
  const n = parseInt(String(hashHex || '0').slice(0, 8), 16) || 0;
  return SYNTHETIC_TIMESTAMP_BASE + (n % SYNTHETIC_TIMESTAMP_SPAN_SECONDS);
}

function timestampSecondsFromPbsStem(stem) {
  const m = String(stem || '').match(/(?:^|[-_])(20\d{6})[-_](\d{4}|\d{6})(?:[-_]|$)/);
  if (!m) throw new Error('PBS battle timestamp not found in battle id/stem: ' + stem);
  const ymd = m[1];
  const hms = m[2].length === 4 ? m[2] + '00' : m[2];
  const year = Number(ymd.slice(0, 4));
  const month = Number(ymd.slice(4, 6));
  const day = Number(ymd.slice(6, 8));
  const hour = Number(hms.slice(0, 2));
  const minute = Number(hms.slice(2, 4));
  const second = Number(hms.slice(4, 6));
  // PBS battle ids store local Japan time. Convert that JST wall-clock time
  // to the corresponding real UTC epoch seconds for Showdown |t:| lines.
  const JST_OFFSET_MS = 9 * 60 * 60 * 1000;
  const ms = Date.UTC(year, month - 1, day, hour, minute, second) - JST_OFFSET_MS;
  const jstCheck = new Date(ms + JST_OFFSET_MS);
  if (
    !Number.isFinite(ms) ||
    jstCheck.getUTCFullYear() !== year ||
    jstCheck.getUTCMonth() !== month - 1 ||
    jstCheck.getUTCDate() !== day ||
    jstCheck.getUTCHours() !== hour ||
    jstCheck.getUTCMinutes() !== minute ||
    jstCheck.getUTCSeconds() !== second
  ) {
    throw new Error('Invalid PBS battle timestamp in battle id/stem: ' + stem);
  }
  return Math.floor(ms / 1000);
}

function parsePreview(html) {
  const tableM = html.match(/<table[\s\S]*?<\/table>/i);
  if (!tableM) throw new Error('preview table not found');
  const table = tableM[0];
  const firstRow = table.match(/<tr>\s*<td>([\s\S]*?)<\/td>\s*<td[\s\S]*?<\/td>\s*<td>([\s\S]*?)<\/td>\s*<\/tr>/i);
  if (!firstRow) throw new Error('player row not found');
  const players = [stripTagsToText(firstRow[1]), stripTagsToText(firstRow[2])].map(normalizeJa);
  const parts = table.split(/<td[^>]*valign="?middle"?[^>]*>/i);
  if (parts.length < 3) throw new Error('team cells not found');
  const sideCells = [
    parts[1].split(/<\/td>|<td|<\/tr>/i)[0],
    parts[2].split(/<\/td>|<td|<\/tr>/i)[0],
  ];
  const teams = sideCells.slice(0, 2).map(cell => {
    const rows = [...cell.matchAll(/<tt>([\s\S]*?)<\/tt>/gi)].map(m => stripTagsToText(m[1]));
    return rows.map(parseTeamRow);
  });
  return {players, teams};
}

function parseTeamRow(txt) {
  const raw = normalizeJa(txt);
  const m = raw.match(/^(.+?)\s*LV\s*(\d+)\s*([♂♀○])?/);
  if (!m) throw new Error('team row parse failed: ' + txt);
  let namePart = normalizeJa(m[1]).replace(/[♂♀○]/g, '').trim();
  const sexM = normalizeJa(m[1]).match(/[♂♀]/) || raw.match(/[♂♀]/);
  const gender = sexM ? (sexM[0] === '♂' ? 'M' : 'F') : '';
  const level = Number(m[2]);
  const species = jpSpeciesToPs(namePart);
  if (!species) throw new Error('unknown species in team row: ' + txt);
  return {jp: normalizeNameKey(namePart), species, level, gender};
}

function invalidPreviewLevels(preview) {
  const bad = [];
  for (const [idx, side] of ['p1', 'p2'].entries()) {
    for (const mon of preview.teams[idx] || []) {
      if (!Number.isFinite(mon.level) || mon.level < 50 || mon.level > 55) {
        bad.push(`${side} ${mon.species} L${mon.level}`);
      }
    }
  }
  return bad;
}

function pokemonIdent(side, species) { return side + 'a: ' + species; }
function pokemonDetails(mon) {
  let s = mon.species + ', L' + mon.level;
  if (mon.gender) s += ', ' + mon.gender;
  return s;
}
function sideName(side) { return side === 'p1' ? 'p1' : 'p2'; }
function playerSideName(state, side) { return side + ': ' + state.players[side]; }

function percentFromAbs(current, max, status) {
  if (current <= 0) return '0 fnt';
  let pct = Math.round((current * 100) / max);
  if (pct < 1) pct = 1;
  if (pct > 100) pct = 100;
  return String(pct) + '/100' + (status ? ' ' + status : '');
}

class BattleState {
  constructor(preview) {
    this.players = {p1: preview.players[0], p2: preview.players[1]};
    this.teams = {p1: preview.teams[0], p2: preview.teams[1]};
    this.teamByJp = {p1: new Map(), p2: new Map()};
    this.monState = {p1: new Map(), p2: new Map()};
    for (const side of ['p1', 'p2']) {
      for (const mon of this.teams[side]) {
        this.teamByJp[side].set(mon.jp, mon);
        this.monState[side].set(mon.species, {hpAbs: null, maxHp: null, status: '', fainted: false, active: false, itemUsed: false, substitute: false, trapped: false, leechSeededBy: null});
      }
    }
    this.active = {p1: null, p2: null};
    this.started = false;
    this.turn = 0;
    this.lastMove = null;
    this.lastMoveUserSide = null;
    this.lastMoveTargetSide = null;
    this.lastMoveTargetSpecies = null;
    this.pendingMove = null;
    this.pendingCant = null;
    this.lastDirectDamageTarget = null;
    this.perish = {};
    this.spikes = {p1: false, p2: false};
    this.pendingHpCause = '';
    this.pendingHpOf = null;
    this.pendingPresentRecovery = null;
    this.attractOf = {p1: new Map(), p2: new Map()};
    this.leechSeededBy = {p1: new Map(), p2: new Map()};
    this.pendingLeech = null;
    this.pendingDrainOf = null;
    this.pendingBellyDrum = null;
    this.splitBeforeNextSwitch = false;
    this.timestampBeforeNextSwitch = false;
    this.suppressNextConfusionActivate = null;
    this.pendingPainSplit = null;
    this.sleepTalkPending = null;
    this.metronomePending = null;
    this.lastMoveFromMetronome = false;
    this.rejectBattleReason = '';
    this.nightmare = {p1: new Map(), p2: new Map()};
    this.cursed = {p1: new Map(), p2: new Map()};
  }
  resolveSideFromPlayer(player) {
    if (player === this.players.p1) return 'p1';
    if (player === this.players.p2) return 'p2';
    return null;
  }
  sideFromAlt(alt) {
    const t = normalizeJa(alt).replace(/の$/, '');
    if (t === this.players.p1) return 'p1';
    if (t === this.players.p2) return 'p2';
    return null;
  }
  getMonByJp(side, jp) {
    const key = normalizeNameKey(jp);
    const mon = this.teamByJp[side].get(key);
    if (!mon) return null;
    return mon;
  }
  getMonState(side, species) { return this.monState[side].get(species); }
  setMaxIfNeeded(side, species, maxHp) {
    const st = this.getMonState(side, species);
    if (st && !st.maxHp && maxHp) st.maxHp = maxHp;
  }
  updateHp(side, species, current, maxHp, status) {
    const st = this.getMonState(side, species);
    if (!st) return;
    if (maxHp) st.maxHp = maxHp;
    st.hpAbs = current;
    if (status !== undefined) st.status = status || st.status || '';
    if (current <= 0) st.fainted = true;
  }
  hpText(side, species) {
    const st = this.getMonState(side, species);
    if (!st || st.hpAbs === null || !st.maxHp) return '100/100' + (st && st.status ? ' ' + st.status : '');
    return percentFromAbs(st.hpAbs, st.maxHp, st.status);
  }
  clearNightmare(side, species) {
    if (side && species && this.nightmare[side]) this.nightmare[side].delete(species);
  }
  switchIn(side, mon) {
    const out = [];
    const previousActive = this.active[side];
    if (previousActive && previousActive !== mon.species) this.clearNightmare(side, previousActive);
    this.active[side] = mon.species;
    for (const [sp, st] of this.monState[side]) st.active = false;
    this.getMonState(side, mon.species).active = true;
    out.push(`|switch|${pokemonIdent(side, mon.species)}|${pokemonDetails(mon)}|${this.hpText(side, mon.species)}`);
    return out;
  }
  dragIn(side, mon) {
    const out = [];
    const previousActive = this.active[side];
    if (previousActive && previousActive !== mon.species) this.clearNightmare(side, previousActive);
    this.active[side] = mon.species;
    for (const [sp, st] of this.monState[side]) st.active = false;
    this.getMonState(side, mon.species).active = true;
    out.push(`|drag|${pokemonIdent(side, mon.species)}|${pokemonDetails(mon)}|${this.hpText(side, mon.species)}`);
    return out;
  }
}

function parseParagraphs(html) {
  const paras = [];
  const re = /<p\s+align="(left|right|center)"[^>]*>([\s\S]*?)<\/p>/gi;
  let m;
  while ((m = re.exec(html))) {
    const align = m[1];
    const inner = m[2];
    const altM = inner.match(/alt="([^"]*)"/i);
    const alt = altM ? htmlDecode(altM[1]) : '';
    const text = stripTagsToText(inner);
    if (!text) continue;
    paras.push({align, alt, text, raw: inner});
  }
  return paras;
}

function parseSegments(html) {
  const parts = html.split(/<hr[^>]*>/i);
  // After first hr, initial lead segment, subsequent turn segments.
  return parts.map(p => ({html: p, text: stripTagsToText(p), paras: parseParagraphs(p)}));
}

function inferSideForParagraph(state, para) {
  const fromAlt = state.sideFromAlt(para.alt);
  if (fromAlt) return fromAlt;
  return para.align === 'left' ? 'p1' : 'p2';
}
function other(side) { return side === 'p1' ? 'p2' : 'p1'; }

function parseSwitchLine(state, text) {
  const line = normalizeJa(text);
  // Player names may themselves contain the character "は", so first match
  // by known player-name prefix instead of a naive /^(.+?)は/ split.
  for (const side of ['p1', 'p2']) {
    const player = state.players[side];
    const prefix = player + 'は';
    if (!line.startsWith(prefix)) continue;
    const rest = line.slice(prefix.length);
    let m = rest.match(/^(.+?)を戻して(.+?)を繰り出した！$/);
    if (m) return {kind: 'switch', side, speciesJp: m[2]};
    m = rest.match(/^(.+?)を繰り出した！$/);
    if (m) return {kind: 'switch', side, speciesJp: m[1]};
  }
  const m = line.match(/^(.+?)が戦闘にひきずりだされた！$/);
  if (m) return {kind: 'drag-species', speciesJp: m[1]};
  return null;
}

function parseWin(html) {
  const m = html.match(/<font[^>]*color="GoldenRod"[^>]*>([\s\S]*?)ＷＩＮ！/i);
  if (m) return normalizeJa(stripTagsToText(m[1]));
  return '';
}

function truncateHtmlAtFirstWin(html) {
  const m = String(html || '').match(/<center><font[^>]*color="GoldenRod"[^>]*>[\s\S]*?ＷＩＮ！[\s\S]*?<\/center>/i);
  if (!m) return html;
  return html.slice(0, m.index + m[0].length);
}

function parseMoveStart(text) {
  const m = text.match(/^(.+?)の(.+?)！/);
  if (!m) return null;
  const speciesJp = m[1];
  const moveJp = m[2];
  const move = jpMoveToPs(moveJp);
  if (!move) return null;
  return {speciesJp, moveJp, move};
}

function parseHpLine(text) {
  const m = text.match(/^(.+?)の HP\s*(\d+)→\s*(\d+)/);
  if (!m) return null;
  return {speciesJp: m[1], from: Number(m[2]), to: Number(m[3])};
}

function parseFaintLine(text) {
  const m = text.match(/^(.+?)は倒れた。。$/);
  return m ? {speciesJp: m[1]} : null;
}

function addMoveLine(out, state, side, move, targetSide, targetSpecies) {
  const userSpecies = state.active[side];
  if (!userSpecies) throw new Error('active user missing for ' + side + ' move ' + move);
  const target = targetSide && targetSpecies ? pokemonIdent(targetSide, targetSpecies) : pokemonIdent(side, userSpecies);
  out.push(`|move|${pokemonIdent(side, userSpecies)}|${move}|${target}`);
  state.lastMove = move;
  state.lastMoveUserSide = side;
  state.lastMoveTargetSide = targetSide;
  state.lastMoveTargetSpecies = targetSpecies;
  state.lastMoveFromMetronome = false;
}

function determineTargetForMove(state, side, move) {
  const selfMoves = new Set(['Curse', 'Swords Dance', 'Belly Drum', 'Rest', 'Protect', 'Endure', 'Focus Energy', 'Metronome', 'Substitute', 'Double Team', 'Baton Pass', 'Destiny Bond', 'Acid Armor', 'Recover', 'Soft-Boiled', 'Milk Drink', 'Synthesis', 'Agility', 'Sandstorm', 'Sunny Day', 'Rain Dance', 'Haze', 'Reflect', 'Light Screen', 'Heal Bell', 'Sleep Talk', 'Conversion']);
  const opponentStatus = new Set(['Thunder Wave', 'Toxic', 'Confuse Ray', 'Attract', 'Mean Look', 'Perish Song', 'Leech Seed', 'Sleep Powder', 'Spikes', 'Whirlwind', 'Roar', 'Charm']);
  if (selfMoves.has(move)) return {targetSide: side, targetSpecies: state.active[side]};
  const os = other(side);
  return {targetSide: os, targetSpecies: state.active[os]};
}

function parseBattle(file, args, dictText) {
  const decoded = decodeHtmlFile(file);
  const html = decoded.text;
  const sourceFingerprint = html.replace(/\.\/[^\"']+?_files\//g, './FILES/');
  const preview = parsePreview(html);
  const battleHtml = truncateHtmlAtFirstWin(html);
  const state = new BattleState(preview);
  const warnings = [];
  const errors = [];
  const unknown = [];
  const statAudit = [];
  const automaticRepairs = [];
  const winName = parseWin(html);
  let stem = fileStemFromSavedUrl(file, html);
  const sourceHash = sha256Hex(sourceFingerprint);
  const sourceHash12 = sourceHash.slice(0, 12);
  const missingTimestamp = !stemHasPbsTimestamp(stem);
  const syntheticRoom = roomFromSavedUrl(html);
  const missingTimestampReason = missingTimestamp ? ('PBS battle timestamp not found in battle id/stem: ' + stem) : '';
  if (missingTimestamp) {
    if (args.missingTimestampPolicy === 'fail') throw new Error(missingTimestampReason);
    stem = `${syntheticRoom}-undated-${sourceHash12}`;
  }
  const battleId = args.battlePrefix + '-' + slug(stem);
  const lines = [];
  let t = (missingTimestamp ? syntheticTimestampSeconds(sourceHash) : timestampSecondsFromPbsStem(stem)) - 20;
  const nextTime = () => String(t += 20);

  function warn(s) { warnings.push(s); }
  function fail(s) { errors.push(s); }
  function unknownLine(ctx, s) { unknown.push(ctx + ': ' + s); }
  function statAuditLine(s) { statAudit.push(s); }
  function automaticRepairLine(s) { automaticRepairs.push(s); }

  if (missingTimestamp) {
    const msg = args.missingTimestampPolicy === 'skip'
      ? missingTimestampReason + '; skipped by --missing-timestamp-policy=skip'
      : missingTimestampReason + '; using synthetic timestamp and undated battle id';
    warn(msg);
    statAuditLine(`synthetic_timestamp=${args.missingTimestampPolicy === 'synthetic'}`);
    statAuditLine(`missing_timestamp_policy=${args.missingTimestampPolicy}`);
    statAuditLine(`original_stem=${fileStemFromSavedUrl(file, html)}`);
    statAuditLine(`synthetic_room=${syntheticRoom}`);
    statAuditLine(`source_hash=${sourceHash}`);
    statAuditLine(`synthetic_stem=${stem}`);
    statAuditLine(`synthetic_timestamp_base=${SYNTHETIC_TIMESTAMP_BASE}`);
    statAuditLine(`synthetic_timestamp_span_seconds=${SYNTHETIC_TIMESTAMP_SPAN_SECONDS}`);
    statAuditLine(`synthetic_timestamp_seconds=${t + 20}`);
    if (args.missingTimestampPolicy === 'skip') state.rejectBattleReason = msg;
  }

  const invalidLevels = invalidPreviewLevels(preview);
  if (invalidLevels.length) {
    const reason = 'non-NC2000 Pokemon level present: ' + invalidLevels.join(', ');
    state.rejectBattleReason = reason;
    warn(reason);
  }

  // Header
  lines.push('|init|battle');
  lines.push(`|title|${state.players.p1} vs. ${state.players.p2}`);
  lines.push(`|j|☆${state.players.p1}`);
  lines.push(`|j|☆${state.players.p2}`);
  lines.push(`|t:|${nextTime()}`);
  lines.push('|gametype|singles');
  lines.push('|j| nc2000logger');
  lines.push(`|player|p1|${state.players.p1}|169|`);
  lines.push(`|player|p2|${state.players.p2}|101|`);
  lines.push('|gen|2');
  lines.push(`|tier|${TIER}`);
  lines.push('|rule|Species Clause: Limit one of each Pokémon');
  lines.push('|rule|Item Clause: Limit 1 of each item');
  lines.push('|rule|Sleep Clause Mod: Limit one foe put to sleep');
  lines.push('|rule|Freeze Clause Mod: Limit one foe frozen');
  lines.push('|rule|OHKO Clause: OHKO moves are banned');
  lines.push('|rule|HP Percentage Mod: HP is shown in percentages');
  lines.push('|clearpoke');
  for (const side of ['p1', 'p2']) {
    for (const mon of state.teams[side]) lines.push(`|poke|${side}|${pokemonDetails(mon)}|item`);
  }
  lines.push('|teampreview|3');
  lines.push('|');
  lines.push(`|t:|${nextTime()}`);
  lines.push('|teamsize|p1|3');
  lines.push('|teamsize|p2|3');
  lines.push('|start');

  const segments = parseSegments(battleHtml);
  // segment 1 should have initial leads.
  const leadSeg = segments.find((s, i) => i > 0 && /繰り出した！/.test(s.text));
  if (!leadSeg) throw new Error('initial lead segment not found');
  for (const para of leadSeg.paras) {
    const sw = parseSwitchLine(state, para.text);
    if (sw && sw.kind === 'switch') {
      if (!sw.side) { fail('lead player side unresolved: ' + para.text); continue; }
      const mon = state.getMonByJp(sw.side, sw.speciesJp);
      if (!mon) { fail('lead species not found: ' + para.text); continue; }
      lines.push(...state.switchIn(sw.side, mon));
    }
  }
  lines.push('|turn|1');

  let currentTurn = 1;
  const startIndex = segments.indexOf(leadSeg) + 1;
  for (let si = startIndex; si < segments.length; si++) {
    const seg = segments[si];
    if (!seg || !seg.text) continue;
    if (/戻る\s*$/.test(seg.text) && !/！/.test(seg.text) && !/HP/.test(seg.text)) continue;
    if (/ＷＩＮ/.test(seg.text) && seg.paras.length === 0) continue;

    const turnLines = [];
    turnLines.push('|');
    turnLines.push(`|t:|${nextTime()}`);
    processSegment(seg, state, turnLines, {warn, fail, unknownLine, nextTime, statAuditLine, automaticRepairLine, segmentIndex: si, turn: currentTurn});

    // Skip empty segments except time marker.
    if (turnLines.length <= 2) continue;
    lines.push(...turnLines);
    // Do not add an upkeep after the first WIN marker; the battle has ended.
    const segHasWin = /ＷＩＮ/.test(seg.text);
    if (!segHasWin) lines.push('|upkeep');
    currentTurn++;
    // Do not add next turn if the segment contains win; win is appended after loop.
    if (si < segments.length - 1 && !segHasWin) lines.push(`|turn|${currentTurn}`);
  }

  if (winName) {
    // Remove trailing turn marker if it was appended after the last real turn.
    while (lines.length && /^\|turn\|/.test(lines[lines.length - 1])) lines.pop();
    lines.push('|');
    lines.push(`|win|${winName}`);
    lines.push(`|l|☆${state.players.p1}`);
    lines.push('|player|p1|');
    lines.push(`|l|☆${state.players.p2}`);
    lines.push('|expire|');
    lines.push('|deinit');
  } else {
    warn('WIN marker not found; raw.log is incomplete and has no |win|');
  }

  if (unknown.length) warnings.push('unknown/unhandled text lines: ' + unknown.length);
  const complete = !!winName;
  return {battleId, stem, raw: lines.join('\n') + '\n', sourceFingerprint, warnings, errors, unknown, statAudit, automaticRepairs, complete, encoding: decoded.encoding, players: state.players, rejectReason: state.rejectBattleReason || ''};
}

function processSegment(seg, state, out, hooks) {
  const paras = seg.paras;
  const consumed = new Set();
  let residualBlockOpen = false;
  const segmentOutStart = out.length;
  const segmentStatExpectations = [];

  function startResidualBlock() {
    if (!residualBlockOpen) {
      if (out[out.length - 1] !== '|') out.push('|');
      residualBlockOpen = true;
    }
  }

  function emitPerishSongSuccess() {
    out.push(`|-fieldactivate|move: Perish Song`);
    for (const s of ['p1', 'p2']) {
      const sp = state.active[s];
      if (sp) {
        out.push(`|-start|${pokemonIdent(s, sp)}|perish3|[silent]`);
        state.perish[s] = {species: sp, count: 3};
      }
    }
    return true;
  }

  function preparePresentRecovery(moveSide, para, paragraphIndex) {
    const textLines = para.text.split('\n').map(normalizeJa).filter(Boolean);
    const hpLines = textLines.map(text => ({text, hp: parseHpLine(text)})).filter(entry => entry.hp);
    if (!hpLines.length) return;

    const invalidMarkers = textLines.filter(text => /攻撃ははずれた|急所にあたった|効果抜群だ|効果はイマイチ|効果がないようだ|効かないようだ/.test(text));
    const expectedTargetSide = other(moveSide);
    const expectedTargetSpecies = state.active[expectedTargetSide];
    const structureErrors = [];
    if (hpLines.length !== 1) structureErrors.push(`HP lines=${hpLines.length}`);
    if (invalidMarkers.length) structureErrors.push('damage/miss marker in recovery paragraph');
    if (state.lastMove !== 'Present' || state.lastMoveUserSide !== moveSide) structureErrors.push('last move is not the current Present');
    if (state.lastMoveTargetSide !== expectedTargetSide) structureErrors.push(`target side=${state.lastMoveTargetSide || '(none)'} expected=${expectedTargetSide}`);
    if (!expectedTargetSpecies || state.lastMoveTargetSpecies !== expectedTargetSpecies) structureErrors.push('target is not the current opposing active Pokemon');

    state.pendingPresentRecovery = {
      para,
      paragraphIndex,
      moveSide,
      targetSide: expectedTargetSide,
      targetSpecies: expectedTargetSpecies,
      invalid: structureErrors.length > 0,
    };
    if (structureErrors.length) {
      hooks.fail(`Present recovery paragraph failed structural validation: ${structureErrors.join(', ')}`);
    }
  }

  function processTextLine(line, para, idx) {
    line = normalizeJa(line);
    if (!line || line === '戻る') return true;
    if (/ＷＩＮ/.test(line)) return true;
    if (line === 'すなあらしがふきあれる！') { startResidualBlock(); out.push('|-weather|Sandstorm|[upkeep]'); return true; }
    if (line === 'すなあらしがおさまった') { startResidualBlock(); out.push('|-weather|none'); return true; }
    if (line === '日差しが強い！') { startResidualBlock(); out.push('|-weather|SunnyDay|[upkeep]'); return true; }
    if (line === '日差しが強くなった。') return weatherStart('SunnyDay');
    if (line === '日差しが元に戻った。' || line === '日差しが弱まった') { startResidualBlock(); out.push('|-weather|none'); return true; }
    if (line === '大雨になった。') return weatherStart('RainDance');
    if (line === '雨が降り続いている') { startResidualBlock(); out.push('|-weather|RainDance|[upkeep]'); return true; }
    if (line === '雨が降り止んだ。') { startResidualBlock(); out.push('|-weather|none'); return true; }
    if (/すなあらしがふきはじめた/.test(line)) return weatherStart('Sandstorm');
    if (/特殊攻撃に強くなった。$/.test(line)) return lightScreenSuccess(line, para);
    if (line === '一撃必殺！！') return ohkoSuccess(line);

    // switch / drag / death switch
    const sw = parseSwitchLine(state, line);
    if (sw) {
      if (sw.kind === 'switch') {
        if (state.splitBeforeNextSwitch) {
          out.push('|upkeep');
          out.push('|');
          out.push(`|t:|${hooks.nextTime ? hooks.nextTime() : ''}`);
          state.splitBeforeNextSwitch = false;
          state.timestampBeforeNextSwitch = false;
        } else if (state.timestampBeforeNextSwitch) {
          out.push('|');
          out.push(`|t:|${hooks.nextTime ? hooks.nextTime() : ''}`);
          state.timestampBeforeNextSwitch = false;
        }
        if (!sw.side) { hooks.fail('switch player side unresolved: ' + line); return true; }
        const mon = state.getMonByJp(sw.side, sw.speciesJp);
        if (!mon) { hooks.fail('switch species unresolved: ' + line); return true; }
        out.push(...state.switchIn(sw.side, mon));
        return true;
      }
      if (sw.kind === 'drag-species') {
        const side = other(inferSideForParagraph(state, para)); // message is from Whirlwind user side, target dragged opposite
        const mon = state.getMonByJp(side, sw.speciesJp);
        if (!mon) { hooks.fail('drag species unresolved: ' + line); return true; }
        out.push(...state.dragIn(side, mon));
        return true;
      }
    }

    // Move starts.
    const moveStart = parseMoveStart(line);
    if (moveStart) {
      residualBlockOpen = false;
      const side = inferSideForParagraph(state, para);
      const species = jpSpeciesToPs(moveStart.speciesJp);
      let moveSide = side;
      if (!species || state.active[side] !== species) {
        // If active on opposite side, use it.
        const os = other(side);
        if (species && state.active[os] === species) {
          moveSide = os;
        } else {
          hooks.fail(`move user unresolved/active mismatch: side=${side} line=${line}`);
          return true;
        }
      }
      const handled = processMoveStart(moveSide, species, moveStart, line);
      if (moveStart.move === 'Present') preparePresentRecovery(moveSide, para, idx);
      return handled;
    }

    // HP line.
    const hp = parseHpLine(line);
    if (hp) {
      if (state.pendingPresentRecovery && state.pendingPresentRecovery.para === para) {
        return processPresentRecoveryHpLine(hp, para, line);
      }
      return processHpLine(hp, para, line);
    }

    // Faint.
    const faint = parseFaintLine(line);
    if (faint) {
      const side = sideForFaintLine(state, para, faint.speciesJp);
      const species = jpSpeciesToPs(faint.speciesJp);
      if (!side || !species) { hooks.fail('faint side/species unresolved: ' + line); return true; }
      const st = state.getMonState(side, species);
      if (st && st.fainted && state.active[side] !== species) return true;
      function emitFaintOnce(fSide, fSpecies) {
        const fst = state.getMonState(fSide, fSpecies);
        if (fst && fst.fainted && state.active[fSide] !== fSpecies) return false;
        if (fst) { fst.hpAbs = 0; fst.fainted = true; fst.active = false; fst.status = ''; }
        state.clearNightmare(fSide, fSpecies);
        if (state.active[fSide] === fSpecies) state.active[fSide] = null;
        out.push(`|faint|${pokemonIdent(fSide, fSpecies)}`);
        return true;
      }
      const isSelfKo = state.lastMove === 'Self-Destruct' || state.lastMove === 'Explosion';
      if (isSelfKo && side === state.lastMoveUserSide && state.lastMoveTargetSide && state.lastMoveTargetSpecies) {
        emitFaintOnce(side, species);
        const tst = state.getMonState(state.lastMoveTargetSide, state.lastMoveTargetSpecies);
        if (tst && tst.hpAbs === 0) emitFaintOnce(state.lastMoveTargetSide, state.lastMoveTargetSpecies);
      } else {
        emitFaintOnce(side, species);
      }
      if (state.pendingPerishZeroFaint && state.pendingPerishZeroFaint.species === species && state.pendingPerishZeroFaint.side === side) state.pendingPerishZeroFaint = null;
      if (!state.splitBeforeNextSwitch) state.timestampBeforeNextSwitch = true;
      return true;
    }

    // Status starts and cures.
    if (/は麻痺した！$/.test(line)) return statusLine(line, para, 'par');
    if (/はメロメロになった！$/.test(line)) return startVolatile(line, para, 'Attract');
    if (/は眠ってしまった！$/.test(line) || /は眠りはじめた！/.test(line)) {
      return statusLine(line, para, 'slp');
    }
    if (/は混乱した！$/.test(line)) return startVolatile(line, para, 'confusion');
    if (/は猛毒におかされた！$/.test(line)) return statusLine(line, para, 'tox');
    if (/は毒におかされた！$/.test(line)) return statusLine(line, para, 'psn');
    if (/は火傷した！$/.test(line) || /は火傷をおった！$/.test(line) || /はやけどをおった！$/.test(line)) return statusLine(line, para, 'brn');
    if (/は凍ってしまった！$/.test(line) || /はこおりづけになった！$/.test(line)) return statusLine(line, para, 'frz');
    if (/は目を覚ました！$/.test(line)) return cureByWake(line, para);
    if (/の氷がとけた！$/.test(line)) return cureFreeze(line, para);
    if (/は正気に戻った！$/.test(line)) return confusionEnd(line, para);
    if (/でめをさました！$/.test(line)) return itemCure(line, para, 'slp');
    if (/でどくをなおした！$/.test(line)) return itemCure(line, para, 'tox');
    if (/でまひをなおした！$/.test(line)) return itemCure(line, para, 'par');
    if (/でこおりをとかした！$/.test(line)) return itemCure(line, para, 'frz');
    if (/でやけどをなおした！$/.test(line)) return itemCure(line, para, 'brn');
    if (/でこんらんを回復した！$/.test(line)) return itemCureConfusion(line, para);

    // Cant lines.
    if (/は眠っていて動けない！$/.test(line)) return true;
    if (/はからだが痺れて動けない！$/.test(line)) return cantForSubject(line, para, 'par');
    if (/はひるんだ！$/.test(line)) return cantForSubject(line, para, 'flinch');
    if (/はメロメロで技がだせなかった！$/.test(line)) return cantForSubject(line, para, 'Attract');
    if (/は凍って動けない！$/.test(line) || /は凍っていて動けない！$/.test(line)) return cantForSubject(line, para, 'frz');
    if (/は反動で動けない！$/.test(line)) return cantForSubject(line, para, 'recharge');
    if (/^しかし.+?は封じられている！$/.test(line)) return disableCant(line);

    // Activation-only text.
    if (/はぐうぐう眠っている$/.test(line)) return sleepingDisplay(line, para);
    if (/は混乱している$/.test(line)) { const subj = subjectSpecies(line, /^(.*?)は混乱している$/); if (subj) activateVolatile(subj, para, 'confusion'); return true; }
    if (/はこらえている！$/.test(line)) return endureActivate(line, para);
    if (/はわけもわからず自分を攻撃した！$/.test(line)) { const subj = subjectSpecies(line, /^(.*?)はわけもわからず自分を攻撃した！$/); if (subj) { const side = sideForOwnedSpeciesInLine(state, para, subj); const species = jpSpeciesToPs(subj); if (side && species) state.pendingHpCause = 'confusion'; } return true; }
    if (/は.*にメロメロだ！$/.test(line)) { const subj = subjectSpecies(line, /^(.*?)は.*にメロメロだ！$/); if (subj) activateVolatile(subj, para, 'move: Attract'); return true; }
    if (/はアンコールをうけた！$/.test(line)) return encoreStart(line, para);
    if (/のアンコール状態がとけた。$/.test(line)) return encoreEnd(line, para);
    if (/^.+?は.+?の.+?を封じた！$/.test(line)) return disableStart(line, para);
    if (/はあくむをみはじめた！$/.test(line)) return nightmareStart(line, para);
    if (/はあくむにうなされている！$/.test(line)) { state.pendingHpCause = 'nightmare'; return true; }
    if (/^.+?は.+?タイプを自分に貼り付けた$/.test(line)) return conversionTypeChange(line, para);

    // Miss/immune/effectiveness/crit/protect handled as lines adjacent to moves.
    if (/急所にあたった！/.test(line)) { addCritToLastTarget(); return true; }
    if (/効果抜群だ！/.test(line)) { addEffectivenessToLastTarget('supereffective'); return true; }
    if (/効果はイマイチのようだ/.test(line)) { addEffectivenessToLastTarget('resisted'); return true; }
    if (/には効果がないようだ/.test(line) || /には効かないようだ/.test(line)) { addImmuneOrFail(line, para); return true; }
    if (/(しかし)? .+?の攻撃ははずれた。$/.test(line) || /^.+?の攻撃ははずれた。$/.test(line)) { addMiss(); return true; }
    if (/しかし.*はまもっている！/.test(line)) { addProtectActivate(); return true; }
    if (/分身が攻撃を受けた！/.test(line)) { addSubHit(); return true; }
    if (/分身はきえてしまった/.test(line)) { addSubBreak(); return true; }

    if (/の持っていた.*はかいのいでんし.*さどうした！/.test(line)) return berserkGene(line, para);

    // Item/heal textual lines.
    if (/はたべのこしでＨＰを回復/.test(line)) { startResidualBlock(); state.pendingHpCause = 'leftovers'; return true; }
    if (/はおうごんのみでＨＰを回復した！/.test(line)) { state.pendingHpCause = 'goldberry'; return true; }
    if (/はきのみジュースでＨＰを回復した！/.test(line)) { state.pendingHpCause = 'berryjuice'; return true; }
    if (/はきのみでＨＰを回復した！/.test(line)) { hooks.warn('nonstandard NC2000 item Berry detected: ' + line); state.pendingHpCause = 'berry'; return true; }
    if (/は攻撃の反動をうけた！/.test(line)) {
      state.pendingHpCause = 'recoil';
      if (state.lastMoveTargetSide && state.lastMoveTargetSpecies) state.pendingHpOf = pokemonIdent(state.lastMoveTargetSide, state.lastMoveTargetSpecies);
      return true;
    }
    if (/は毒のダメージを受けている！/.test(line)) { startResidualBlock(); state.pendingHpCause = 'psn'; return true; }
    if (/は火傷のダメージを受けている！/.test(line) || /はやけどのダメージを受けている！/.test(line)) { startResidualBlock(); state.pendingHpCause = 'brn'; return true; }
    if (/はすなあらしのダメージを受けている！/.test(line)) { startResidualBlock(); state.pendingHpCause = 'sandstorm'; return true; }
    if (/はまきびしのダメージをうけた！/.test(line)) { state.pendingHpCause = 'spikes'; return true; }
    if (/はＨＰを回復した。/.test(line)) { state.pendingHpCause = state.lastMove === 'Rest' ? 'rest-heal' : 'heal'; return true; }
    if (/体力を吸い取った！/.test(line)) { state.pendingHpCause = 'drain-heal'; return true; }
    if (/のろいが.*のたいりょくをうばう！/.test(line) || /のろいで.*のたいりょくがうばわれる！/.test(line) || /はのろいで体力を奪われている！/.test(line) || /はのろわれている！/.test(line)) {
      state.pendingHpCause = 'curse';
      return true;
    }
    if (/やどりぎが.*のたいりょくをうばう！/.test(line)) {
      const m = line.match(/やどりぎが(.+?)のたいりょくをうばう！/);
      const targetJp = m ? m[1] : '';
      const targetSide = targetJp ? sideForOwnedSpeciesInLine(state, para, targetJp) : null;
      const targetSpecies = targetJp ? jpSpeciesToPs(targetJp) : null;
      let sourceSide = null;
      let sourceIdent = null;
      if (targetSide && targetSpecies) {
        const source = state.leechSeededBy[targetSide].get(targetSpecies) || null;
        sourceSide = source && source.sourceSide ? source.sourceSide : null;
        const sourceSpecies = sourceSide ? state.active[sourceSide] : null;
        if (sourceSide && sourceSpecies) sourceIdent = pokemonIdent(sourceSide, sourceSpecies);
      }
      state.pendingHpCause = 'leech';
      state.pendingLeech = {targetSide, targetSpecies, sourceSide, sourceIdent};
      return true;
    }

    // Move-specific continuation lines.
    if (/ははりきっている！$/.test(line)) return focusEnergyStart(line, para);
    if (/はいきおいあまって地面に激突した！$/.test(line)) { state.pendingHpCause = 'jump-kick-crash'; return true; }
    if (/のまわりを空気がうずまく。$/.test(line)) return prepareTwoTurnMove(line, para, 'Razor Wind', /^(.*?)のまわりを空気がうずまく。$/);
    if (/はひかりを吸収している。$/.test(line)) return prepareTwoTurnMove(line, para, 'Solar Beam', /^(.*?)はひかりを吸収している。$/);
    if (/を激しい光が包む！$/.test(line)) return prepareTwoTurnMove(line, para, 'Sky Attack', /^(.*?)を激しい光が包む！$/);
    if (/はＨＰの１／４を消費して分身をつくりだした！/.test(line)) return substituteSuccess(line, para);
    if (/は物理攻撃に強くなった。/.test(line)) return reflectSuccess(line, para);
    if (/にたねをうえつけた！$/.test(line)) return leechSeedStart(line, para);
    if (/はもうにげられない！$/.test(line)) return trapStart(line, para);
    if (/は相手をみちづれにしようとしている。/.test(line)) return destinyBondPrepare(line, para);
    if (/は.*をみちづれにした！$/.test(line)) return destinyBondActivate(line, para);
    if (/おたがいのポケモンは３ターン後にほろんでしまう！/.test(line)) return emitPerishSongSuccess();
    if (/ほろびのカウントが([0-9０-９])になった！$/.test(line)) return perishCount(line, para);
    if (/はまきびしをまいた！/.test(line)) return spikesSuccess(line, para);
    if (/は.*の攻撃から解放された！$/.test(line)) return rapidSpinRelease(line, para);
    if (/はたいりょくを削って相手にのろいをかけた！/.test(line)) return ghostCurseStart(line, para);
    if (isStatCapLine(line)) return true;
    if (/しかし交換できるポケモンがいなかった。/.test(line)) { addFailForLastMoveUser(); return true; }
    if (/しかしＨＰがたりなかった！/.test(line)) { substituteWeakFail(); return true; }
    if (/すべての能力が元に戻った！/.test(line)) { out.push('|-clearallboost'); return true; }
    if (/^.+?は.+?から.+?を奪った！$/.test(line)) return thiefItem(line, para);
    if (/しかし、うまくきまらなかった。/.test(line)) {
      if (state.pendingBellyDrum) state.pendingBellyDrum = null;
      if (state.lastMove === 'Transform') transformFailed(); else protectFailed();
      return true;
    }
    if (/はまもりの体勢にはいった！/.test(line)) return protectSuccess(line, para);
    if (/はこらえる体勢にはいった！/.test(line)) return true;
    if (/は.*にへんしんした！$/.test(line)) return transformSuccess(line, para);
    if (/にかかっていた.*リフレクター.*効果がきれた/.test(line)) { out.push(`|-sideend|${playerSideName(state, inferSideForParagraph(state, para))}|Reflect`); return true; }
    if (/にかかっていた.*ひかりのかべ.*効果がきれた/.test(line)) return lightScreenEnd(line, para);
    if (/はたいりょくを削ってこうげきを最大まであげた！/.test(line)) return bellyDrumSuccess(line, para);
    if (/味方のすべての状態異常が回復した！/.test(line)) return healBellSuccess(line, para);
    if (/おたがいの体力をわかちあった！/.test(line)) { state.pendingHpCause = 'pain-split'; state.pendingPainSplit = {entries: []}; return true; }

    const statChange = parseGenericStatChangeLine(line);
    if (statChange) return handleGenericStatChange(statChange, para, line);

    hooks.unknownLine('segment', line);
    return false;
  }

  function parseGenericStatChangeLine(line) {
    const m = line.match(/^(.*?)の(.+?)が(ぐーんと|ぐ～んと|ぐぐーんと|ぐぐ～んと|がくっと|がくーんと|がく～んと)?\s*(あがった|上がった|さがった|下がった)/);
    if (!m) return null;
    const subj = m[1];
    const statJp = normalizeJa(m[2]);
    const degree = m[3] || '';
    const dirText = m[4];
    const statMap = new Map([
      ['こうげき', 'atk'], ['攻撃', 'atk'], ['攻撃力', 'atk'],
      ['ぼうぎょ', 'def'], ['防御', 'def'], ['防御力', 'def'],
      ['すばやさ', 'spe'], ['素早さ', 'spe'],
      ['とくこう', 'spa'], ['特攻', 'spa'],
      ['とくぼう', 'spd'], ['特防', 'spd'],
      ['命中率', 'accuracy'], ['めいちゅうりつ', 'accuracy'],
      ['回避率', 'evasion'], ['かいひりつ', 'evasion'],
    ]);
    const stat = statMap.get(statJp);
    if (!stat) return null;
    const up = dirText === 'あがった' || dirText === '上がった';
    const amount = degree ? 2 : 1;
    return {subj, stat, amount, kind: up ? 'boost' : 'unboost'};
  }

  function statProtocolLineFromChange(change, para) {
    const side = sideForSpeciesInLine(state, para, change.subj);
    const species = jpSpeciesToPs(change.subj);
    if (!side || !species) return null;
    const op = change.kind === 'boost' ? '-boost' : '-unboost';
    return `|${op}|${pokemonIdent(side, species)}|${change.stat}|${change.amount}`;
  }

  function findRecentSameStatProtocol(protocolLine) {
    for (let i = out.length - 1; i >= 0 && i >= out.length - 16; i--) {
      if (out[i] === protocolLine || out[i].startsWith(protocolLine + '|')) return i;
      if (out[i].startsWith('|turn|') || out[i].startsWith('|t:|')) break;
    }
    return -1;
  }

  function recordSpecialStatExpectation(subj, para, kind, stat, amount, line) {
    if (!subj) return;
    const side = sideForSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (!side || !species) return;
    const op = kind === 'boost' ? '-boost' : '-unboost';
    const protocolLine = `|${op}|${pokemonIdent(side, species)}|${stat}|${amount}`;
    segmentStatExpectations.push({line, protocolLine, change: {subj, kind, stat, amount}});
  }

  function handleGenericStatChange(change, para, line) {
    const protocolLine = statProtocolLineFromChange(change, para);
    if (!protocolLine) { hooks.fail('stat change unresolved: ' + line); return true; }
    segmentStatExpectations.push({line, protocolLine, change});
    const existingAt = findRecentSameStatProtocol(protocolLine);
    if (existingAt >= 0) {
      return true;
    }
    out.push(protocolLine);
    return true;
  }

  function parseStatProtocolLine(rawLine) {
    const m = String(rawLine || '').match(/^\|-(boost|unboost)\|([^|]+)\|([^|]+)\|(\d+)(?:\||$)/);
    if (!m) return null;
    return {kind: m[1], ident: m[2], stat: m[3], amount: Number(m[4]), line: rawLine};
  }

  function statLineInfoFromProtocol(protocolLine) {
    return parseStatProtocolLine(protocolLine);
  }

  function auditSegmentStatCoverage() {
    if (!hooks.statAuditLine) return;
    const segStats = [];
    for (let i = segmentOutStart; i < out.length; i++) {
      const info = parseStatProtocolLine(out[i]);
      if (info) segStats.push({...info, outIndex: i});
    }
    const used = new Set();
    const prefix = `turn=${hooks.turn || '?'}	segment=${hooks.segmentIndex || '?'}	`;
    for (const exp of segmentStatExpectations) {
      const expected = statLineInfoFromProtocol(exp.protocolLine);
      let exact = -1;
      for (const cand of segStats) {
        if (used.has(cand.outIndex)) continue;
        if (cand.line === exp.protocolLine || cand.line.startsWith(exp.protocolLine + '|')) { exact = cand.outIndex; break; }
      }
      if (exact >= 0) {
        used.add(exact);
        hooks.statAuditLine(prefix + `matched	text=${exp.line}	expected=${exp.protocolLine}	actual=${out[exact]}`);
        continue;
      }
      let mismatch = null;
      if (expected) {
        for (const cand of segStats) {
          if (used.has(cand.outIndex)) continue;
          const sameIdentStat = cand.ident === expected.ident && cand.stat === expected.stat;
          const sameStatAmountKind = cand.stat === expected.stat && cand.amount === expected.amount && cand.kind === expected.kind;
          const sameIdentKind = cand.ident === expected.ident && cand.kind === expected.kind;
          if (sameIdentStat || sameStatAmountKind || sameIdentKind) { mismatch = cand; break; }
        }
      }
      if (mismatch) {
        used.add(mismatch.outIndex);
        hooks.statAuditLine(prefix + `mismatch	text=${exp.line}	expected=${exp.protocolLine}	actual=${mismatch.line}`);
      } else {
        hooks.statAuditLine(prefix + `missing	text=${exp.line}	expected=${exp.protocolLine}`);
      }
    }
    for (const cand of segStats) {
      if (!used.has(cand.outIndex)) hooks.statAuditLine(prefix + `extra	actual=${cand.line}`);
    }
  }

  function lastMoveLineIs(move) {
    for (let i = out.length - 1; i >= 0; i--) {
      if (out[i].startsWith('|move|')) return out[i].includes('|' + move + '|');
    }
    return false;
  }

  function processMoveStart(side, species, moveStart, line) {
    if (!moveStart.move) { hooks.fail('unknown move: ' + moveStart.moveJp + ' line=' + line); return true; }
    const fromSleepTalk = !!(state.sleepTalkPending && state.sleepTalkPending.side === side && state.sleepTalkPending.species === species && moveStart.move !== 'Sleep Talk');
    const fromMetronome = !!(state.metronomePending && state.metronomePending.side === side && state.metronomePending.species === species && moveStart.move !== 'Metronome');
    const {targetSide, targetSpecies} = determineTargetForMove(state, side, moveStart.move);
    addMoveLine(out, state, side, moveStart.move, targetSide, targetSpecies);
    if (fromSleepTalk) {
      out[out.length - 1] += '|[from] Sleep Talk';
      state.sleepTalkPending = null;
    }
    if (fromMetronome) {
      out[out.length - 1] += '|[from] Metronome';
      state.metronomePending = null;
      state.lastMoveFromMetronome = true;
    }
    if (moveStart.move === 'Sleep Talk') {
      state.sleepTalkPending = {side, species};
    }
    if (moveStart.move === 'Metronome') {
      state.metronomePending = {side, species};
    }
    if (moveStart.move === 'Giga Drain' || moveStart.move === 'Absorb' || moveStart.move === 'Mega Drain') {
      if (targetSide && targetSpecies) state.pendingDrainOf = pokemonIdent(targetSide, targetSpecies);
    }
    return true;
  }

  function isLiveActive(state, side, species) {
    if (!side || !species) return false;
    if (state.active[side] !== species) return false;
    const st = state.getMonState(side, species);
    return !!st && !st.fainted;
  }

  
  function sideForOwnedSpeciesInLine(state, para, speciesJp) {
    const species = jpSpeciesToPs(speciesJp);
    if (!species) return null;
    const altSide = inferSideForParagraph(state, para);
    const os = other(altSide);
    if (isLiveActive(state, altSide, species)) return altSide;
    if (isLiveActive(state, os, species)) return os;
    if (state.monState[altSide].has(species)) return altSide;
    if (state.monState[os].has(species)) return os;
    return null;
  }

  function sideForSpeciesInLine(state, para, speciesJp) {
    const species = jpSpeciesToPs(speciesJp);
    if (!species) return null;
    // Prefer the explicit current move target/user when active species names are duplicated across sides.
    if (state.lastMoveTargetSide && state.lastMoveTargetSpecies === species && isLiveActive(state, state.lastMoveTargetSide, species)) return state.lastMoveTargetSide;
    if (state.lastMoveUserSide && isLiveActive(state, state.lastMoveUserSide, species)) return state.lastMoveUserSide;
    const altSide = inferSideForParagraph(state, para);
    if (isLiveActive(state, altSide, species)) return altSide;
    const os = other(altSide);
    if (isLiveActive(state, os, species)) return os;
    if (state.monState[altSide].has(species)) return altSide;
    if (state.monState[os].has(species)) return os;
    return null;
  }



  function sideForFaintLine(state, para, speciesJp) {
    const species = jpSpeciesToPs(speciesJp);
    if (!species) return null;
    if (state.pendingPerishZeroFaint && state.pendingPerishZeroFaint.species === species && isLiveActive(state, state.pendingPerishZeroFaint.side, species)) {
      return state.pendingPerishZeroFaint.side;
    }
    const altSide = inferSideForParagraph(state, para);
    const os = other(altSide);
    // Faint lines are ownership messages in PBS; when alt identifies a side
    // and that side currently has this species active, prefer it over the last
    // move target. This avoids clearing the wrong active in mirror species cases.
    if (isLiveActive(state, altSide, species)) return altSide;
    if (isLiveActive(state, os, species)) return os;
    if (state.lastMoveTargetSide && state.lastMoveTargetSpecies === species && isLiveActive(state, state.lastMoveTargetSide, species)) return state.lastMoveTargetSide;
    if (state.lastMoveUserSide && isLiveActive(state, state.lastMoveUserSide, species)) return state.lastMoveUserSide;
    if (state.monState[altSide].has(species)) return altSide;
    if (state.monState[os].has(species)) return os;
    return null;
  }

  function sideForHpLine(state, para, speciesJp) {
    const species = jpSpeciesToPs(speciesJp);
    if (!species) return null;
    const altSide = inferSideForParagraph(state, para);
    const os = other(altSide);

    // HP lines in PBS usually include an img alt like "<player>の".
    // For HP accounting, that explicit owner side must win over last move
    // user/target. This avoids side swaps when both active Pokemon share a species.
    if (state.monState[altSide].has(species)) return altSide;

    // Fall back to live active candidates only. Fainted active Pokemon must not be
    // selected for later HP lines of the same species.
    if (isLiveActive(state, altSide, species)) return altSide;
    if (isLiveActive(state, os, species)) return os;

    // Last move user/target are only supplemental for HP lines.
    if (state.lastMoveTargetSide && state.lastMoveTargetSpecies === species && isLiveActive(state, state.lastMoveTargetSide, species)) return state.lastMoveTargetSide;
    if (state.lastMoveUserSide && isLiveActive(state, state.lastMoveUserSide, species)) return state.lastMoveUserSide;

    if (state.monState[os].has(species)) return os;
    return null;
  }

  function sideForPainSplitHpLine(state, para, speciesJp) {
    const species = jpSpeciesToPs(speciesJp);
    if (!species) return null;
    if (state.lastMoveUserSide && state.active[state.lastMoveUserSide] === species) return state.lastMoveUserSide;
    if (state.lastMoveTargetSide && state.lastMoveTargetSpecies === species) return state.lastMoveTargetSide;
    return sideForHpLine(state, para, speciesJp);
  }

  function sideForLeechHpLine(state, para, speciesJp, hp) {
    const species = jpSpeciesToPs(speciesJp);
    const leech = state.pendingLeech || {};
    if (!species) return null;
    if (hp && hp.to < hp.from && leech.targetSide && leech.targetSpecies === species) return leech.targetSide;
    if (hp && hp.to >= hp.from && leech.sourceSide && state.active[leech.sourceSide] === species) return leech.sourceSide;
    return sideForHpLine(state, para, speciesJp);
  }

  function processPresentRecoveryHpLine(hp, para, line) {
    const pending = state.pendingPresentRecovery;
    state.pendingPresentRecovery = null;
    if (!pending || pending.para !== para) {
      hooks.fail('Present recovery HP line lost its paragraph context: ' + line);
      return true;
    }
    if (pending.invalid) return true;

    const species = jpSpeciesToPs(hp.speciesJp);
    const {targetSide, targetSpecies, moveSide} = pending;
    const failures = [];
    if (!species) failures.push('HP species is unknown');
    if (species !== targetSpecies) failures.push(`HP species=${species || '(unknown)'} expected=${targetSpecies || '(none)'}`);
    if (targetSide !== other(moveSide)) failures.push(`target side=${targetSide || '(none)'} is not opponent of ${moveSide}`);
    if (!isLiveActive(state, targetSide, targetSpecies)) failures.push('resolved target is not the current live opposing active Pokemon');

    const st = targetSide && targetSpecies ? state.getMonState(targetSide, targetSpecies) : null;
    if (!st) failures.push('resolved target has no tracked HP state');

    // PBS emits Present recovery as "after -> before". Only this structurally
    // identified case is reversed; ordinary Present damage remains untouched.
    const actualBefore = hp.to;
    const actualAfter = hp.from;
    let baseline = 'tracked';
    if (!Number.isInteger(actualBefore) || !Number.isInteger(actualAfter) || actualBefore <= 0 || actualAfter <= 0) {
      failures.push(`invalid HP values source=${hp.from}->${hp.to}`);
    }
    if (actualAfter < actualBefore) failures.push(`reversed values do not describe healing: corrected=${actualBefore}->${actualAfter}`);

    if (st) {
      if (st.hpAbs !== null && st.hpAbs !== actualBefore) {
        failures.push(`tracked current HP=${st.hpAbs} but corrected before HP=${actualBefore}`);
      }
      if (st.maxHp && actualAfter > st.maxHp) {
        failures.push(`corrected after HP=${actualAfter} exceeds max HP=${st.maxHp}`);
      }
      if (st.hpAbs === null) {
        if (st.maxHp) {
          if (actualBefore !== actualAfter || actualAfter !== st.maxHp) {
            failures.push(`unobserved current HP cannot be validated against max HP=${st.maxHp}`);
          } else {
            baseline = 'inferred-full-hp';
          }
        } else if (actualBefore === actualAfter) {
          baseline = 'inferred-full-hp';
        } else {
          failures.push('non-full recovery has no prior absolute HP baseline');
        }
      }
    }

    if (failures.length) {
      hooks.fail(`Present recovery HP/target consistency failure: ${failures.join(', ')} line=${line}`);
      return true;
    }

    if (!st.maxHp) st.maxHp = actualAfter;
    st.hpAbs = actualAfter;
    out.push(`|-heal|${pokemonIdent(targetSide, targetSpecies)}|${percentFromAbs(actualAfter, st.maxHp, st.status)}`);

    const owners = ['p1', 'p2'].filter(side => state.monState[side].has(targetSpecies));
    const paragraphSide = inferSideForParagraph(state, para);
    if (hooks.automaticRepairLine) {
      hooks.automaticRepairLine([
        'type=pbs-present-recovery',
        `turn=${hooks.turn}`,
        `segment=${hooks.segmentIndex}`,
        `paragraph=${pending.paragraphIndex}`,
        `user=${pokemonIdent(moveSide, state.active[moveSide])}`,
        `target=${pokemonIdent(targetSide, targetSpecies)}`,
        `paragraph_side=${paragraphSide}`,
        `source_hp=${hp.from}->${hp.to}`,
        `corrected_hp=${actualBefore}->${actualAfter}`,
        `baseline=${baseline}`,
        `mirrored_species=${owners.length > 1}`,
      ].join('\t'));
    }
    state.pendingHpCause = '';
    state.pendingHpOf = null;
    return true;
  }

  function processHpLine(hp, para, line) {
    const hpCauseForSide = state.pendingHpCause;
    const side = hpCauseForSide === 'pain-split'
      ? sideForPainSplitHpLine(state, para, hp.speciesJp)
      : (hpCauseForSide === 'leech'
        ? sideForLeechHpLine(state, para, hp.speciesJp, hp)
        : (hpCauseForSide === 'drain-heal'
          ? sideForSpeciesInLine(state, para, hp.speciesJp)
          : sideForHpLine(state, para, hp.speciesJp)));
    const species = jpSpeciesToPs(hp.speciesJp);
    if (!side || !species) { hooks.fail('HP side/species unresolved: ' + line); return true; }
    if (state.pendingHpCause === 'pain-split' && state.pendingPainSplit) {
      state.setMaxIfNeeded(side, species, hp.from > hp.to ? hp.from : Math.max(hp.from, hp.to));
      const stPs = state.getMonState(side, species);
      if (stPs) {
        if (!stPs.maxHp || hp.from > stPs.maxHp) stPs.maxHp = Math.max(hp.from, hp.to);
        stPs.hpAbs = hp.to;
      }
      state.pendingPainSplit.entries.push({side, species, from: hp.from, to: hp.to});
      if (state.pendingPainSplit.entries.length >= 2) {
        emitPainSplitEntries(state.pendingPainSplit.entries);
        state.pendingHpCause = '';
        state.pendingPainSplit = null;
      }
      return true;
    }
    state.setMaxIfNeeded(side, species, hp.from > hp.to ? hp.from : Math.max(hp.from, hp.to));
    const st = state.getMonState(side, species);
    if (!st.maxHp || hp.from > st.maxHp) st.maxHp = Math.max(hp.from, hp.to);

    const causeBefore = state.pendingHpCause;
    let type = hp.to < hp.from ? '-damage' : '-heal';
    let suffix = '';
    // Recoil is a damage event in Showdown protocol even when the absolute HP
    // shown by the source log does not decrease (for example 186 -> 186).
    // Without this, same-HP recoil is misclassified as a heal.
    if (state.pendingHpCause === 'recoil') type = '-damage';

    if (state.pendingHpCause === 'recoil') {
      suffix = '|[from] Recoil' + (state.pendingHpOf ? '|[of] ' + state.pendingHpOf : '');
    }
    if (state.pendingHpCause === 'jump-kick-crash') suffix = '';
    if (state.pendingHpCause === 'leech') {
      const leech = state.pendingLeech || {};
      if (type === '-damage') {
        const ofIdent = leech.sourceIdent || '';
        suffix = ofIdent ? `|[from] Leech Seed|[of] ${ofIdent}` : '|[from] Leech Seed';
      } else {
        suffix = '|[silent]';
      }
    }
    if (state.pendingHpCause === 'drain-heal') suffix = '|[from] drain' + (state.pendingDrainOf ? '|[of] ' + state.pendingDrainOf : '');
    if (state.pendingHpCause === 'confusion') suffix = '|[from] confusion';
    if (state.pendingHpCause === 'psn') suffix = '|[from] psn';
    if (state.pendingHpCause === 'brn') suffix = '|[from] brn';
    if (state.pendingHpCause === 'nightmare') suffix = '|[from] Nightmare';
    if (state.pendingHpCause === 'curse') suffix = '|[from] Curse';
    if (state.pendingHpCause === 'sandstorm') suffix = '|[from] Sandstorm';
    if (state.pendingHpCause === 'spikes') suffix = '|[from] Spikes';
    if (state.pendingHpCause === 'leftovers') suffix = '|[from] item: Leftovers';
    if (state.pendingHpCause === 'heal') suffix = '';
    if (state.pendingHpCause === 'rest-heal') suffix = '|[silent]';

    if (state.pendingHpCause === 'goldberry') {
      out.push(`|-enditem|${pokemonIdent(side, species)}|Gold Berry|[eat]`);
      suffix = '|[from] item: Gold Berry';
    }
    if (state.pendingHpCause === 'berryjuice') {
      out.push(`|-enditem|${pokemonIdent(side, species)}|Berry Juice`);
      suffix = '|[from] item: Berry Juice';
    }
    if (state.pendingHpCause === 'berry') {
      out.push(`|-enditem|${pokemonIdent(side, species)}|Berry|[eat]`);
      suffix = '|[from] item: Berry';
    }

    if (hp.to <= 0) {
      st.hpAbs = 0;
      const s = percentFromAbs(0, st.maxHp, st.status);
      out.push(`|-damage|${pokemonIdent(side, species)}|${s}${suffix}`);
      maybeEmitMustRechargeAfterHyperBeamDamage(side, species, causeBefore);
      if (causeBefore === 'leech' && type === '-damage') {
        state.pendingHpCause = 'leech';
      } else {
        state.pendingHpCause = '';
      }
      state.pendingHpOf = null;
      if (causeBefore !== 'leech' || type === '-heal') state.pendingLeech = null;
      if (!(state.lastMove === 'Giga Drain' || state.lastMove === 'Absorb' || state.lastMove === 'Mega Drain') || causeBefore === 'drain-heal') state.pendingDrainOf = null;
      return true;
    }
    st.hpAbs = hp.to;

    out.push(`|${type}|${pokemonIdent(side, species)}|${percentFromAbs(hp.to, st.maxHp, st.status)}${suffix}`);
    maybeEmitMustRechargeAfterHyperBeamDamage(side, species, causeBefore);
    if (state.pendingBellyDrum && state.pendingBellyDrum.side === side && state.pendingBellyDrum.species === species) {
      out.push(`|-boost|${pokemonIdent(side, species)}|atk|6`);
      state.pendingBellyDrum = null;
    }
    if ((causeBefore === 'leech' && type === '-heal') || causeBefore === 'drain-heal') {
      if (out[out.length - 1] !== '|') out.push('|');
    }
    if (causeBefore === 'leech' && type === '-damage') {
      state.pendingHpCause = 'leech';
    } else {
      state.pendingHpCause = '';
    }
    state.pendingHpOf = null;
    if (causeBefore !== 'leech' || type === '-heal') state.pendingLeech = null;
    if (!(state.lastMove === 'Giga Drain' || state.lastMove === 'Absorb' || state.lastMove === 'Mega Drain') || causeBefore === 'drain-heal') state.pendingDrainOf = null;
    return true;
  }

  function maybeEmitMustRechargeAfterHyperBeamDamage(side, species, causeBefore) {
    if (state.lastMove !== 'Hyper Beam') return;
    if (causeBefore) return;
    if (state.lastMoveTargetSide !== side || state.lastMoveTargetSpecies !== species) return;
    if (!state.lastMoveUserSide || !state.active[state.lastMoveUserSide]) return;
    out.push(`|-mustrecharge|${pokemonIdent(state.lastMoveUserSide, state.active[state.lastMoveUserSide])}`);
  }

  function emitPainSplitEntries(entries) {
    const ordered = entries.slice();
    if (state.lastMoveTargetSide && state.lastMoveTargetSpecies) {
      ordered.sort((a, b) => {
        const at = a.side === state.lastMoveTargetSide && a.species === state.lastMoveTargetSpecies ? 0 : 1;
        const bt = b.side === state.lastMoveTargetSide && b.species === state.lastMoveTargetSpecies ? 0 : 1;
        return at - bt;
      });
    }
    for (let i = 0; i < ordered.length; i++) {
      const e = ordered[i];
      const st = state.getMonState(e.side, e.species);
      const hpText = percentFromAbs(e.to, st && st.maxHp ? st.maxHp : Math.max(e.from, e.to), st && st.status ? st.status : '');
      out.push(`|-sethp|${pokemonIdent(e.side, e.species)}|${hpText}|[from] move: Pain Split${i === 0 ? '|[silent]' : ''}`);
    }
  }

  function subjectSpecies(line, re) { const m = line.match(re); return m ? m[1] : null; }
  function statusLine(line, para, status) {
    const subj = line.match(/^(.*?)は/)[1];
    const side = sideForSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (!side || !species) { hooks.fail('status unresolved: ' + line); return true; }
    state.getMonState(side, species).status = status;
    const fromRest = status === 'slp' && state.lastMove === 'Rest' && state.lastMoveUserSide === side && state.active[side] === species;
    out.push(`|-status|${pokemonIdent(side, species)}|${status}${fromRest ? '|[from] move: Rest' : ''}`);
    return true;
  }
  function startVolatile(line, para, volatile) {
    const subj = line.match(/^(.*?)は/)[1];
    const side = sideForSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (!side || !species) { hooks.fail('volatile unresolved: ' + line); return true; }
    out.push(`|-start|${pokemonIdent(side, species)}|${volatile}`);
    if (volatile === 'Attract') {
      const ofSide = other(side);
      const ofSpecies = state.active[ofSide];
      if (ofSpecies) state.attractOf[side].set(species, pokemonIdent(ofSide, ofSpecies));
    }
    return true;
  }
  function sleepingDisplay(line, para) {
    const subj = (line.match(/^(.*?)はぐうぐう眠っている$/) || [])[1];
    const side = sideForOwnedSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) out.push(`|cant|${pokemonIdent(side, species)}|slp`);
    return true;
  }

  function activateVolatile(speciesJp, para, volatile) {
    const side = sideForOwnedSpeciesInLine(state, para, speciesJp);
    const species = jpSpeciesToPs(speciesJp);
    if (side && species) {
      if (volatile === 'move: Attract') {
        const ofIdent = state.attractOf[side].get(species);
        out.push(`|-activate|${pokemonIdent(side, species)}|${volatile}${ofIdent ? '|[of] ' + ofIdent : ''}`);
      } else {
        out.push(`|-activate|${pokemonIdent(side, species)}|${volatile}`);
      }
    }
  }
  function cureByWake(line, para) {
    const subj = line.match(/^(.*?)は目を覚ました！/)[1];
    const side = sideForOwnedSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) {
      state.getMonState(side, species).status = '';
      out.push(`|-curestatus|${pokemonIdent(side, species)}|slp|[msg]`);
      if (state.nightmare[side] && state.nightmare[side].has(species)) {
        out.push(`|-end|${pokemonIdent(side, species)}|Nightmare|[silent]`);
        state.nightmare[side].delete(species);
      }
    }
    return true;
  }

  function cureFreeze(line, para) {
    const subj = line.match(/^(.*?)の氷がとけた！/)[1];
    const side = sideForOwnedSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) {
      state.getMonState(side, species).status = '';
      out.push(`|-curestatus|${pokemonIdent(side, species)}|frz|[msg]`);
    }
    return true;
  }
  function confusionEnd(line, para) {
    const subj = line.match(/^(.*?)は正気に戻った！/)[1];
    const side = sideForOwnedSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) out.push(`|-end|${pokemonIdent(side, species)}|confusion`);
    return true;
  }
  function encoreStart(line, para) {
    const subj = (line.match(/^(.*?)はアンコールをうけた！$/) || [])[1];
    const side = sideForSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) out.push(`|-start|${pokemonIdent(side, species)}|Encore`);
    return true;
  }
  function encoreEnd(line, para) {
    const subj = (line.match(/^(.*?)のアンコール状態がとけた。$/) || [])[1];
    const side = sideForSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) out.push(`|-end|${pokemonIdent(side, species)}|Encore`);
    return true;
  }
  function disableStart(line, para) {
    const m = line.match(/^(.*?)は(.*?)の(.*?)を封じた！$/);
    const sourceJp = m ? m[1] : '';
    const targetJp = m ? m[2] : '';
    const moveJp = m ? m[3] : '';
    const sourceSpecies = jpSpeciesToPs(sourceJp);
    const targetSpecies = jpSpeciesToPs(targetJp);
    const move = jpMoveToPs(moveJp);
    const sourceSide = sourceJp ? sideForOwnedSpeciesInLine(state, para, sourceJp) : null;
    let targetSide = sourceSide ? other(sourceSide) : null;
    if (!targetSide || state.active[targetSide] !== targetSpecies) {
      targetSide = targetJp ? sideForOwnedSpeciesInLine(state, para, targetJp) : null;
    }
    if (!sourceSide || !sourceSpecies || !targetSide || !targetSpecies || !move) {
      hooks.fail('Disable start unresolved: ' + line);
      return true;
    }
    const sourceIdent = pokemonIdent(sourceSide, sourceSpecies);
    const targetIdent = pokemonIdent(targetSide, targetSpecies);
    retargetLastMoveLine(sourceIdent, 'Disable', targetIdent);
    out.push(`|-start|${targetIdent}|Disable|${move}`);
    return true;
  }
  function disableCant(line) {
    const m = line.match(/^しかし(.+?)は封じられている！$/);
    const move = m ? jpMoveToPs(m[1]) : null;
    const side = state.lastMoveUserSide;
    const species = side ? state.active[side] : null;
    if (!move || !side || !species) {
      hooks.fail('Disable cant unresolved: ' + line);
      return true;
    }
    const ident = pokemonIdent(side, species);
    let removedMoveLine = false;
    for (let i = out.length - 1; i >= 0 && i >= out.length - 24; i--) {
      const parts = out[i].split('|');
      if (parts[1] === 'move' && parts[2] === ident && parts[3] === move) {
        out.splice(i, 1);
        removedMoveLine = true;
        break;
      }
      if (out[i].startsWith('|turn|') || out[i].startsWith('|t:|')) break;
    }
    if (!removedMoveLine) {
      hooks.fail('Disable cant move line not found: ' + line);
      return true;
    }
    out.push(`|cant|${ident}|Disable|${move}`);
    return true;
  }
  function nightmareStart(line, para) {
    const subj = (line.match(/^(.*?)はあくむをみはじめた！$/) || [])[1];
    const side = sideForSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) {
      out.push(`|-start|${pokemonIdent(side, species)}|Nightmare`);
      state.nightmare[side].set(species, true);
    }
    return true;
  }
  function conversionTypeChange(line, para) {
    const m = line.match(/^(.*?)は(.+?)タイプを自分に貼り付けた$/);
    const subj = m ? m[1] : null;
    const typeJp = m ? m[2] : null;
    const side = sideForSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    const type = jpTypeToPs(typeJp);
    if (side && species && type) {
      out.push(`|-start|${pokemonIdent(side, species)}|typechange|${type}`);
    } else {
      hooks.fail('Conversion typechange unresolved: ' + line);
    }
    return true;
  }
  function isStatCapLine(line) {
    return /^.+?の.+?はもう(あがらない|上がらない|さがらない|下がらない)$/.test(line);
  }
  function retargetLastMoveLine(userIdent, move, targetIdent) {
    for (let i = out.length - 1; i >= 0 && i >= out.length - 24; i--) {
      const parts = out[i].split('|');
      if (parts[1] === 'move' && parts[2] === userIdent && parts[3] === move) {
        parts[4] = targetIdent;
        out[i] = parts.join('|');
        return true;
      }
      if (out[i].startsWith('|turn|') || out[i].startsWith('|t:|')) break;
    }
    return false;
  }
  function ghostCurseStart(line, para) {
    const subj = (line.match(/^(.*?)はたいりょくを削って相手にのろいをかけた！$/) || [])[1];
    const userSide = subj ? sideForSpeciesInLine(state, para, subj) : state.lastMoveUserSide;
    const userSpecies = subj ? jpSpeciesToPs(subj) : (userSide ? state.active[userSide] : null);
    const targetSide = userSide ? other(userSide) : state.lastMoveTargetSide;
    const targetSpecies = targetSide ? state.active[targetSide] : null;
    if (userSide && userSpecies && targetSide && targetSpecies) {
      const userIdent = pokemonIdent(userSide, userSpecies);
      const targetIdent = pokemonIdent(targetSide, targetSpecies);
      state.lastMoveTargetSide = targetSide;
      state.lastMoveTargetSpecies = targetSpecies;
      retargetLastMoveLine(userIdent, 'Curse', targetIdent);
      out.push(`|-start|${targetIdent}|Curse|[of] ${userIdent}`);
      state.cursed[targetSide].set(targetSpecies, userIdent);
    }
    return true;
  }

  

  function subjectBeforeWa(line) {
    return (line.match(/^(.*?)は/) || [])[1] || '';
  }

  function ownedIdentFromSubject(line, para) {
    const subj = subjectBeforeWa(line);
    const side = sideForOwnedSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (!side || !species) return null;
    return {side, species, ident: pokemonIdent(side, species)};
  }

  function weatherStart(weather) {
    out.push(`|-weather|${weather}`);
    return true;
  }

  function protectSuccess(line, para) {
    const info = ownedIdentFromSubject(line, para);
    if (info) out.push(`|-singleturn|${info.ident}|Protect`);
    return true;
  }

  function substituteSuccess(line, para) {
    const info = ownedIdentFromSubject(line, para);
    if (info) {
      out.push(`|-start|${info.ident}|Substitute`);
      const st = state.getMonState(info.side, info.species);
      if (st) st.substitute = true;
    }
    return true;
  }

  function destinyBondPrepare(line, para) {
    const info = ownedIdentFromSubject(line, para);
    if (info) out.push(`|-singlemove|${info.ident}|Destiny Bond`);
    return true;
  }

  function spikesSuccess(line, para) {
    const info = ownedIdentFromSubject(line, para);
    if (info) {
      const targetSide = other(info.side);
      out.push(`|-sidestart|${playerSideName(state, targetSide)}|Spikes`);
      state.spikes[targetSide] = true;
    }
    return true;
  }

  function reflectSuccess(line, para) {
    const info = ownedIdentFromSubject(line, para);
    const side = info ? info.side : inferSideForParagraph(state, para);
    out.push(`|-sidestart|${playerSideName(state, side)}|Reflect`);
    return true;
  }

  function lightScreenSuccess(line, para) {
    const info = ownedIdentFromSubject(line, para);
    const side = info ? info.side : inferSideForParagraph(state, para);
    out.push(`|-sidestart|${playerSideName(state, side)}|move: Light Screen`);
    return true;
  }

  function healBellSuccess(line, para) {
    const side = state.lastMoveUserSide || inferSideForParagraph(state, para);
    const species = side ? state.active[side] : null;
    if (side && species) {
      out.push(`|-cureteam|${pokemonIdent(side, species)}|[from] move: Heal Bell`);
      for (const st of state.monState[side].values()) st.status = '';
    }
    return true;
  }

  function bellyDrumSuccess(line, para) {
    const info = ownedIdentFromSubject(line, para);
    if (info) {
      state.pendingBellyDrum = {side: info.side, species: info.species};
      recordSpecialStatExpectation(subjectBeforeWa(line), para, 'boost', 'atk', 6, line);
    }
    return true;
  }

  function focusEnergyStart(line, para) {
    const subj = (line.match(/^(.*?)ははりきっている！$/) || [])[1];
    const side = sideForOwnedSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) out.push(`|-start|${pokemonIdent(side, species)}|move: Focus Energy`);
    return true;
  }

  function prepareTwoTurnMove(line, para, move, re) {
    const subj = (line.match(re) || [])[1];
    const side = sideForOwnedSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (!side || !species) return true;
    const ident = pokemonIdent(side, species);
    const fromMetronome = !!(state.metronomePending && state.metronomePending.side === side && state.metronomePending.species === species);
    out.push(`|move|${ident}|${move}||${fromMetronome ? '[from] Metronome|' : ''}[still]`);
    out.push(`|-prepare|${ident}|${move}`);
    state.lastMove = move;
    state.lastMoveUserSide = side;
    state.lastMoveTargetSide = null;
    state.lastMoveTargetSpecies = null;
    state.lastMoveFromMetronome = fromMetronome;
    if (fromMetronome) state.metronomePending = null;
    return true;
  }

  function ohkoSuccess(line) {
    const fromMetronome = !!state.lastMoveFromMetronome;
    if (fromMetronome) {
      out.push('|-ohko');
    } else {
      state.rejectBattleReason = 'non-NC2000 OHKO success message present: ' + line;
      hooks.warn(state.rejectBattleReason);
    }
    return true;
  }

  function endureActivate(line, para) {
    const subj = (line.match(/^(.*?)はこらえている！$/) || [])[1];
    const side = sideForSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) out.push(`|-activate|${pokemonIdent(side, species)}|move: Endure`);
    return true;
  }

  function transformSuccess(line, para) {
    if (state.lastMove !== 'Transform') return true;
    if (!state.lastMoveUserSide || !state.active[state.lastMoveUserSide]) return true;
    const userIdent = pokemonIdent(state.lastMoveUserSide, state.active[state.lastMoveUserSide]);
    let targetSide = state.lastMoveTargetSide;
    let targetSpecies = state.lastMoveTargetSpecies;
    if (!targetSide || !targetSpecies) {
      const m = line.match(/^.*?は(.+?)にへんしんした！$/);
      const targetJp = m ? m[1] : '';
      const targetPs = jpSpeciesToPs(targetJp);
      if (targetPs) {
        for (const s of ['p1', 'p2']) {
          if (state.active[s] === targetPs) { targetSide = s; targetSpecies = targetPs; break; }
        }
      }
    }
    if (targetSide && targetSpecies) out.push(`|-transform|${userIdent}|${pokemonIdent(targetSide, targetSpecies)}`);
    return true;
  }

  function transformFailed() {
    if (state.lastMoveTargetSide && state.lastMoveTargetSpecies) {
      out.push(`|-fail|${pokemonIdent(state.lastMoveTargetSide, state.lastMoveTargetSpecies)}`);
      return true;
    }
    addFailForLastMoveUser();
    return true;
  }

  function berserkGene(line, para) {
    const subj = (line.match(/^(.*?)の持っていた/) || [])[1];
    const side = sideForSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) {
      const ident = pokemonIdent(side, species);
      out.push(`|-enditem|${ident}|Berserk Gene`);
      out.push(`|-boost|${ident}|atk|2|[from] item: Berserk Gene`);
    }
    return true;
  }
  function itemCureConfusion(line, para) {
    const subj = line.match(/^(.*?)は/)[1];
    const itemJp = (line.match(/は(.+?)で/) || [])[1] || '';
    const item = jpItemToPs(itemJp) || 'Miracle Berry';
    const side = sideForOwnedSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) {
      out.push(`|-enditem|${pokemonIdent(side, species)}|${item}|[eat]`);
      out.push(`|-end|${pokemonIdent(side, species)}|confusion`);
    }
    return true;
  }

  function itemCure(line, para, status) {
    const subj = line.match(/^(.*?)は/)[1];
    const itemJp = (line.match(/は(.+?)で/) || [])[1] || '';
    const item = jpItemToPs(itemJp) || (status === 'slp' ? 'Mint Berry' : 'Miracle Berry');
    const side = sideForOwnedSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) {
      out.push(`|-enditem|${pokemonIdent(side, species)}|${item}|[eat]`);
      out.push(`|-curestatus|${pokemonIdent(side, species)}|${status}|[msg]`);
      state.getMonState(side, species).status = '';
    }
    return true;
  }
  function cantForSubject(line, para, reason) {
    const subj = line.match(/^(.*?)は/)[1];
    const side = sideForOwnedSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) out.push(`|cant|${pokemonIdent(side, species)}|${reason}`);
    return true;
  }

  function lightScreenEnd(line, para) {
    const subj = (line.match(/^(.*?)にかかっていた/) || [])[1] || '';
    const side = subj ? sideForSpeciesInLine(state, para, subj) : inferSideForParagraph(state, para);
    out.push(`|-sideend|${playerSideName(state, side || inferSideForParagraph(state, para))}|move: Light Screen`);
    return true;
  }
  function addCritToLastTarget() {
    if (state.lastMoveTargetSide && state.lastMoveTargetSpecies) out.push(`|-crit|${pokemonIdent(state.lastMoveTargetSide, state.lastMoveTargetSpecies)}`);
  }
  function addEffectivenessToLastTarget(kind) {
    if (state.lastMoveTargetSide && state.lastMoveTargetSpecies) out.push(`|-${kind}|${pokemonIdent(state.lastMoveTargetSide, state.lastMoveTargetSpecies)}`);
  }
  function addImmuneOrFail(line, para) {
    if (state.lastMoveTargetSide && state.lastMoveTargetSpecies) out.push(`|-immune|${pokemonIdent(state.lastMoveTargetSide, state.lastMoveTargetSpecies)}`);
  }
  function markLastMoveFlag(flag) {
    for (let i = out.length - 1; i >= 0; i--) {
      if (out[i].startsWith('|move|')) {
        if (!out[i].includes(flag)) out[i] += flag;
        return;
      }
    }
  }
  function addMiss() {
    if (state.lastMoveUserSide) {
      markLastMoveFlag('|[miss]');
      out.push(`|-miss|${pokemonIdent(state.lastMoveUserSide, state.active[state.lastMoveUserSide])}`);
    }
  }
  function addProtectActivate() {
    if (state.lastMoveTargetSide && state.lastMoveTargetSpecies) out.push(`|-activate|${pokemonIdent(state.lastMoveTargetSide, state.lastMoveTargetSpecies)}|Protect`);
  }
  function protectFailed() {
    const ident = state.lastMoveUserSide ? pokemonIdent(state.lastMoveUserSide, state.active[state.lastMoveUserSide]) : null;
    if (ident) out.push(`|-fail|${ident}`);
  }

  function addFailForLastMoveUser() {
    const ident = state.lastMoveUserSide ? pokemonIdent(state.lastMoveUserSide, state.active[state.lastMoveUserSide]) : null;
    if (ident) out.push(`|-fail|${ident}`);
  }


  function substituteWeakFail() {
    const ident = state.lastMoveUserSide ? pokemonIdent(state.lastMoveUserSide, state.active[state.lastMoveUserSide]) : null;
    if (ident) {
      const st = state.getMonState(state.lastMoveUserSide, state.active[state.lastMoveUserSide]);
      if (st) st.substitute = false;
      out.push(`|-fail|${ident}|move: Substitute|[weak]`);
    }
    return true;
  }

  function thiefItem(line, para) {
    const m = line.match(/^(.*?)は(.*?)から(.*?)を奪った！$/);
    if (!m) return true;
    const thiefJp = m[1];
    const victimJp = m[2];
    const itemJp = m[3];
    const thiefSide = sideForSpeciesInLine(state, para, thiefJp);
    const thiefSpecies = jpSpeciesToPs(thiefJp);
    const victimSide = state.lastMoveTargetSide && state.lastMoveTargetSpecies === jpSpeciesToPs(victimJp)
      ? state.lastMoveTargetSide
      : sideForSpeciesInLine(state, para, victimJp);
    const victimSpecies = jpSpeciesToPs(victimJp);
    const item = jpItemToPs(itemJp);
    if (!thiefSide || !thiefSpecies || !victimSide || !victimSpecies || !item) {
      hooks.fail('thief item unresolved: ' + line);
      return true;
    }
    out.push(`|-item|${pokemonIdent(thiefSide, thiefSpecies)}|${item}|[from] move: Thief|[of] ${pokemonIdent(victimSide, victimSpecies)}`);
    return true;
  }

  function rapidSpinRelease(line, para) {
    const subj = (line.match(/^(.*?)は/) || [])[1];
    const side = subj ? sideForSpeciesInLine(state, para, subj) : state.lastMoveUserSide;
    const species = subj ? jpSpeciesToPs(subj) : (side ? state.active[side] : null);
    const ident = side && species ? pokemonIdent(side, species) : (state.lastMoveUserSide ? pokemonIdent(state.lastMoveUserSide, state.active[state.lastMoveUserSide]) : null);
    const clearSide = side || state.lastMoveUserSide;
    if (clearSide && state.spikes[clearSide]) {
      out.push(`|-sideend|${playerSideName(state, clearSide)}|Spikes|[from] move: Rapid Spin${ident ? '|[of] ' + ident : ''}`);
      state.spikes[clearSide] = false;
    }
    return true;
  }
  function addSubHit() {
    if (state.lastMoveTargetSide && state.lastMoveTargetSpecies) {
      if (/分身はきえてしまった/.test(seg.text)) return;
      out.push(`|-activate|${pokemonIdent(state.lastMoveTargetSide, state.lastMoveTargetSpecies)}|Substitute|[damage]`);
    }
  }
  function addSubBreak() {
    if (state.lastMoveTargetSide && state.lastMoveTargetSpecies) {
      if (out.__subBreakEmitted) return;
      out.__subBreakEmitted = true;
      out.push(`|-end|${pokemonIdent(state.lastMoveTargetSide, state.lastMoveTargetSpecies)}|Substitute`);
      const st = state.getMonState(state.lastMoveTargetSide, state.lastMoveTargetSpecies);
      if (st) st.substitute = false;
    }
  }
  function statDrop(line, para, stat, amount) {
    const subj = (line.match(/^(.*?)の/) || [])[1];
    const side = sideForSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) out.push(`|-unboost|${pokemonIdent(side, species)}|${stat}|${amount}`);
    return true;
  }
  function statBoost(line, para, stat, amount) {
    const subj = (line.match(/^(.*?)の/) || [])[1];
    const side = sideForSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) out.push(`|-boost|${pokemonIdent(side, species)}|${stat}|${amount}`);
    return true;
  }
  function leechSeedStart(line, para) {
    const subj = line.match(/^(.*?)にたねをうえつけた！/)[1];
    let side = null;
    let species = jpSpeciesToPs(subj);
    if (species && state.lastMoveTargetSide && state.lastMoveTargetSpecies === species) side = state.lastMoveTargetSide;
    if (!side) side = sideForSpeciesInLine(state, para, subj);
    if (side && species) {
      out.push(`|-start|${pokemonIdent(side, species)}|move: Leech Seed`);
      const sourceSide = state.lastMoveUserSide || null;
      if (sourceSide) state.leechSeededBy[side].set(species, {sourceSide});
    }
    return true;
  }
  function trapStart(line, para) {
    const subj = line.match(/^(.*?)はもうにげられない！/)[1];
    const side = sideForSpeciesInLine(state, para, subj);
    const species = jpSpeciesToPs(subj);
    if (side && species) out.push(`|-activate|${pokemonIdent(side, species)}|trapped`);
    return true;
  }
  function destinyBondActivate(line, para) {
    const m = line.match(/^(.*?)は(.*?)をみちづれにした！$/);
    if (!m) return true;
    const side = sideForSpeciesInLine(state, para, m[1]);
    const species = jpSpeciesToPs(m[1]);
    if (side && species) out.push(`|-activate|${pokemonIdent(side, species)}|move: Destiny Bond`);
    return true;
  }
  function perishCount(line, para) {
    const m = line.match(/^(.*?)のほろびのカウントが([0-9０-９])になった！$/);
    if (!m) return true;
    const side = sideForOwnedSpeciesInLine(state, para, m[1]);
    const species = jpSpeciesToPs(m[1]);
    const count = Number(String(m[2]).replace(/[０-９]/g, ch => String.fromCharCode(ch.charCodeAt(0) - 0xFEE0)));
    if (side && species) {
      if (count > 0) out.push(`|-start|${pokemonIdent(side, species)}|perish${count}`);
      else {
        if (out[out.length - 1] !== '|') out.push('|');
        out.push(`|-start|${pokemonIdent(side, species)}|perish0`);
        state.pendingPerishZeroFaint = {side, species};
        state.splitBeforeNextSwitch = true;
      }
    }
    return true;
  }

  for (let pi = 0; pi < paras.length; pi++) {
    const para = paras[pi];
    const textLines = para.text.split('\n').map(normalizeJa).filter(Boolean);
    for (const line of textLines) processTextLine(line, para, pi);
  }
  auditSegmentStatCoverage();
}

function hashCode(s) {
  let h = 0;
  for (let i = 0; i < String(s).length; i++) h = ((h << 5) - h + s.charCodeAt(i)) | 0;
  return h;
}

function validateRaw(raw) {
  const warnings = [];
  const lines = raw.split('\n').filter(Boolean);
  if (!lines.includes('|init|battle')) warnings.push('missing |init|battle');
  if (!lines.some(l => l.startsWith('|title|'))) warnings.push('missing |title|');
  if (!lines.some(l => l.startsWith('|teampreview|'))) warnings.push('missing |teampreview|');
  if (!lines.some(l => l === '|start')) warnings.push('missing |start');
  if (!lines.some(l => l.startsWith('|switch|'))) warnings.push('no switch lines');
  if (/(undefined|NaN|null)/.test(raw)) warnings.push('raw contains undefined/NaN/null');
  return warnings;
}

function main() {
  const args = parseArgs(process.argv);
  if (args.help) { usage(); return; }
  if (!args.inputDir || !args.outDir) { usage(); process.exit(2); }
  if (!fs.existsSync(args.inputDir) || !fs.statSync(args.inputDir).isDirectory()) throw new Error('input-dir not found or not a directory: ' + args.inputDir);
  if (!args.allowNoDict && !args.dict) throw new Error('--dict raw_log.dict is required unless --allow-no-dict is set');
  let dictText = '';
  let jaDictLoaded = null;
  if (!args.jaDict && args.dict) {
    const candidate = path.join(path.dirname(args.dict), 'local-ja-dict.js');
    if (fs.existsSync(candidate)) args.jaDict = candidate;
  }
  if (args.jaDict) jaDictLoaded = loadJaDict(args.jaDict);
  if (args.dict) {
    if (!fs.existsSync(args.dict)) throw new Error('dict not found: ' + args.dict);
    dictText = fs.readFileSync(args.dict, 'utf8');
    for (const must of ['|-status|', '|-start|', 'Belly Drum', 'Leech Seed', 'Spikes']) {
      if (!dictText.includes(must)) throw new Error('dict does not look like raw_log.dict; missing token: ' + must);
    }
  }

  const rawDir = path.join(args.outDir, 'raw');
  const auditDir = path.join(args.outDir, 'audit');
  const perBattleDir = path.join(auditDir, 'per-battle');
  ensureDir(rawDir); ensureDir(auditDir); ensureDir(perBattleDir);

  const files = listHtmlFiles(args.inputDir, args.recursive);
  const summary = {version: VERSION, input_dir: path.resolve(args.inputDir), out_dir: path.resolve(args.outDir), dict: args.dict ? path.resolve(args.dict) : null, ja_dict: jaDictLoaded ? path.resolve(jaDictLoaded) : null, missing_timestamp_policy: args.missingTimestampPolicy, total_html: files.length, converted: 0, skipped: 0, failed: 0, warnings: 0, automatic_repairs: 0, results: []};
  const warningLines = [];
  const failedLines = [];
  const skippedLines = [];
  const seenBattleIds = new Map();

  for (const file of files) {
    let res;
    try {
      res = parseBattle(file, args, dictText);
      res.warnings.push(...validateRaw(res.raw));
      const hasWarnings = res.warnings.length || res.unknown.length;
      let status = res.errors.length ? 'failed' : (res.rejectReason ? 'skipped' : (!res.complete && !args.includeIncomplete ? 'skipped' : (args.strict && hasWarnings ? 'failed' : 'converted')));
      const outName = res.battleId + '.raw.log';
      const rawPath = path.join(rawDir, outName);
      let skipReason = res.rejectReason || (!res.complete && !args.includeIncomplete ? 'WIN marker not found' : '');
      if (status === 'converted' && seenBattleIds.has(res.battleId)) {
        const prev = seenBattleIds.get(res.battleId);
        if (prev.sourceFingerprint === res.sourceFingerprint) {
          status = 'skipped';
          skipReason = `duplicate battle_id with identical HTML content; first_file=${prev.file}`;
        } else {
          status = 'failed';
          res.errors.push(`duplicate battle_id with different HTML content; first_file=${prev.file}`);
        }
      }
      if (status === 'converted') {
        fs.writeFileSync(rawPath, res.raw, 'utf8');
        seenBattleIds.set(res.battleId, {file, raw: res.raw, sourceFingerprint: res.sourceFingerprint});
        summary.converted++;
      } else if (status === 'skipped') {
        summary.skipped++;
        skippedLines.push(`${file}	${res.battleId}	${skipReason}`);
      } else {
        summary.failed++;
        failedLines.push(`${file}	${res.battleId}	${res.errors.join('; ') || res.warnings.join('; ')}`);
      }
      if (hasWarnings) {
        summary.warnings++;
        warningLines.push(`${file}\t${res.battleId}\t${res.warnings.join('; ')}\tunknown=${res.unknown.length}`);
      }
      summary.automatic_repairs += res.automaticRepairs ? res.automaticRepairs.length : 0;
      const audit = [
        `file=${file}`,
        `battle_id=${res.battleId}`,
        `encoding=${res.encoding}`,
        `status=${status}`,
        `complete=${res.complete}`,
        `players=${res.players.p1} vs ${res.players.p2}`,
        `raw_lines=${res.raw.split('\n').filter(Boolean).length}`,
        '',
        '[warnings]',
        ...(res.warnings.length ? res.warnings : ['(none)']),
        '',
        '[errors]',
        ...(res.errors.length ? res.errors : ['(none)']),
        '',
        '[unknown/unhandled]',
        ...(res.unknown.length ? res.unknown : ['(none)']),
        '',
        '[automatic-repairs]',
        ...((res.automaticRepairs && res.automaticRepairs.length) ? res.automaticRepairs : ['(none)']),
        '',
        '[stat-audit]',
        ...((res.statAudit && res.statAudit.length) ? res.statAudit : ['(none)']),
      ].join('\n') + '\n';
      fs.writeFileSync(path.join(perBattleDir, res.battleId + '.audit.txt'), audit, 'utf8');
      summary.results.push({file, battle_id: res.battleId, status, complete: res.complete, warnings: res.warnings.length, unknown: res.unknown.length, errors: res.errors.length, automatic_repairs: res.automaticRepairs ? res.automaticRepairs.length : 0, stat_audit: res.statAudit ? res.statAudit.length : 0, raw_path: status === 'converted' ? rawPath : null});
    } catch (e) {
      summary.failed++;
      const id = safeFile(path.basename(file).replace(/\.html?$/i, ''));
      failedLines.push(`${file}\t${e.stack || e.message}`);
      fs.writeFileSync(path.join(perBattleDir, id + '.audit.txt'), `file=${file}\nstatus=failed\nerror=${e.stack || e.message}\n`, 'utf8');
      summary.results.push({file, battle_id: null, status: 'failed', complete: false, warnings: 0, unknown: 0, errors: 1, automatic_repairs: 0, error: e.message});
    }
  }

  fs.writeFileSync(path.join(auditDir, 'summary.json'), JSON.stringify(summary, null, 2) + '\n', 'utf8');
  fs.writeFileSync(path.join(auditDir, 'warnings.txt'), warningLines.join('\n') + (warningLines.length ? '\n' : ''), 'utf8');
  fs.writeFileSync(path.join(auditDir, 'failed.txt'), failedLines.join('\n') + (failedLines.length ? '\n' : ''), 'utf8');
  fs.writeFileSync(path.join(auditDir, 'skipped.txt'), skippedLines.join('\n') + (skippedLines.length ? '\n' : ''), 'utf8');
  console.log(JSON.stringify(summary, null, 2));
  if (summary.failed) process.exitCode = 1;
}

if (require.main === module) {
  try { main(); } catch (e) { console.error(e.stack || e.message); process.exit(1); }
}
