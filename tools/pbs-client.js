#!/usr/bin/env node
/**
 * PBS (`psense.lib.net/PBS/_/C/PS.cgi` and sibling rooms) direct bot client.
 *
 * Connects our imperfect-information NC2000 bot (`crates/wasm` ProtocolSearcher)
 * directly to the classic Gym Leader's Castle (`ジムリーダーの城`) CGI battle
 * simulator so human players can play against the bot through the native
 * `PS.cgi` browser UI without any external wrapper or custom entry point.
 *
 * Design invariants:
 *   1. Native UI preservation: operates purely as an HTTP/Cookie CGI participant
 *      (`ACTION=ENTORY` -> `PDINPUT.cgi` -> `CHECK`/`SELECT` -> `INPUT`/`CHANGE`).
 *   2. Zero-crash guarantee: PBS server mechanics are treated as a black box.
 *      Every network call retries on timeout/reset; every board state is
 *      sanitized into a declarative `nc2000-position-v1` snapshot; every WASM
 *      call is guarded by `try / catch`; and every submitted move/switch is
 *      projected strictly onto the actual `<select Name="MOVE">` `<option>`
 *      values offered by `PS.cgi` on that turn.
 *
 * Usage:
 *   node tools/pbs-client.js --room K --name nc2000bot --team pool:nash [options]
 */
'use strict';

const fs = require('fs');
const path = require('path');
const net = require('net');
const { encodeTeamForPdinput, PDINPUT_ITEM_CODES } = require('./export-pdinput');

const REPO = path.join(__dirname, '..');

// ------------------------------------------------------------------ CLI args
const args = {};
for (let i = 2; i < process.argv.length; i++) {
  const a = process.argv[i];
  if (!a.startsWith('--')) continue;
  const key = a.slice(2);
  if (i + 1 < process.argv.length && !process.argv[i + 1].startsWith('--')) {
    args[key] = process.argv[++i];
  } else {
    args[key] = true;
  }
}

if (args.help || args.h) {
  console.log(`node tools/pbs-client.js — NC2000 bot client for psense.lib.net/PBS/_/X/PS.cgi

  --room ROOM          PBS room letter (C..O, default K), Castle alias 第1..第6,
                       or full PS.cgi URL
                       (e.g. http://psense.lib.net/PBS/_/C/PS.cgi)
  --name NAME          Player name registered in PS.cgi (default nc2000bot;
                       ASCII alphanumeric recommended for non-IE PBS cookie)
  --pass PASS          Entry password (default nc2pass)
  --msg MSG            Entry comment when hosting as P1 (default よろしくおねがいします)
  --mode MODE          auto | host | join (default auto)
                         host: only enter as P1 when the room is empty
                         join: wait until a human P1 enters, then join as P2
                         auto: join as P2 if P1 is waiting, else enter as P1
  --tier TIER          default | mercy (default: default = 81k + pool:nash;
                       mercy = 81k + pool:tier-a human-sized sub-party box)
  --team SPEC          pool:nash | pool:tier-a | pool:random | pool:ID | FILE.json
                       (default determined by --tier)
  --iters N            Search iterations per decision (default: live-search strong 81000)
  --seed N             RNG / searcher seed (default 1)
  --poll-ms MS         Polling interval while waiting for opponent (default 2500)
  --host-timeout-sec S Max seconds to wait as P1 before leaving empty room (default 240, 0=none)
  --games N            Number of games to play (default 1)
  --tlimit SEC         Turn time limit field when entering as P1 (default 0)
  --lturn N            Max turn limit field when entering as P1 (default 0)
  --opponent-db FILE   JSON path for persistent opponent memory & online adaptation
                       (default: data/pbs-opponent-db.json; --no-opponent-db to disable)
  --match-log FILE     JSONL path for completed match records (default: data/pbs-matches.jsonl)
  --no-human-delay     Disable human-like minimum pacing delays
  --random             Fast uniform-legal driver (no WASM search; useful for testing)
  --decision-log F     Append JSONL decision records to F
  --quiet              Minimal console logging`);
  process.exit(0);
}

const DISCORD_DAI_TO_LETTER = {
  '1': 'C', '１': 'C', '一': 'C',
  '2': 'D', '２': 'D', '二': 'D',
  '3': 'E', '３': 'E', '三': 'E',
  '4': 'F', '４': 'F', '四': 'F',
  '5': 'G', '５': 'G', '五': 'G',
  '6': 'H', '６': 'H', '六': 'H',
};

function resolveRoomUrl(raw) {
  const s = String(raw || 'C').trim();
  if (/^https?:\/\//i.test(s)) return s.replace(/\?.*$/, '');
  const daiMatch = s.match(/^(?:第)?([1-6１-６一二三四五六])$/);
  if (daiMatch && DISCORD_DAI_TO_LETTER[daiMatch[1]]) {
    return `http://psense.lib.net/PBS/_/${DISCORD_DAI_TO_LETTER[daiMatch[1]]}/PS.cgi`;
  }
  const letter = s.toUpperCase();
  return `http://psense.lib.net/PBS/_/${letter}/PS.cgi`;
}

const ROOM_URL = resolveRoomUrl(args.room || 'C');
const NAME = String(args.name || 'nc2000bot');
const PASS = String(args.pass || 'nc2pass');
const MSG = String(args.msg || 'よろしくおねがいします');
const MODE = String(args.mode || 'auto').toLowerCase();
const TIER = String(args.tier || 'default').toLowerCase();
const DEFAULT_TEAM_BY_TIER = (TIER === 'mercy' || TIER === 'tiera' || TIER === 'tier-a' || TIER === 'sub')
  ? 'pool:tier-a'
  : 'pool:nash';
const TEAMSPEC = String(args.team || DEFAULT_TEAM_BY_TIER);
const GAMES = Math.max(1, parseInt(args.games || '1', 10));
const POLL_MS = Math.max(500, parseInt(args['poll-ms'] || '2500', 10));
const HOST_TIMEOUT_SEC = Math.max(0, parseInt(args['host-timeout-sec'] ?? '240', 10));
const TLIMIT = Math.max(0, parseInt(args.tlimit || '0', 10));
const LTURN = Math.max(0, parseInt(args.lturn || '0', 10));
const SEED = parseInt(args.seed || String(Date.now() & 0x7fffffff), 10);
const RANDOM = Boolean(args.random);
const QUIET = Boolean(args.quiet);
const DECISION_LOG = args['decision-log'] && args['decision-log'] !== true
  ? path.resolve(String(args['decision-log']))
  : '';
const OPPONENT_DB_PATH = args['no-opponent-db']
  ? ''
  : path.resolve(
      REPO,
      args['opponent-db'] && args['opponent-db'] !== true
        ? String(args['opponent-db'])
        : 'data/pbs-opponent-db.json'
    );
const MATCH_LOG_PATH = args['no-match-log']
  ? ''
  : path.resolve(
      REPO,
      args['match-log'] && args['match-log'] !== true
        ? String(args['match-log'])
        : 'data/pbs-matches.jsonl'
    );

const profileText = fs.readFileSync(path.join(REPO, 'data/search-profiles.json'), 'utf8');
const PROFILE = JSON.parse(profileText).blind;
let defaultStrongIters = 81000;
try {
  const liveSearch = JSON.parse(fs.readFileSync(path.join(REPO, 'data/live-search.json'), 'utf8'));
  if (liveSearch?.battle?.strong) defaultStrongIters = liveSearch.battle.strong;
} catch {}
const ITERS = parseInt(args.iters || String(defaultStrongIters), 10);

// ------------------------------------------------------------------ data & i18n
const dexData = JSON.parse(fs.readFileSync(path.join(REPO, 'data/gen2stadium2.json'), 'utf8'));
const i18n = JSON.parse(fs.readFileSync(path.join(REPO, 'data/i18n-ja.json'), 'utf8'));
const ownPool = JSON.parse(fs.readFileSync(path.join(REPO, 'data/team-pool-v2/team-pool.json'), 'utf8'));
const nashDeme = JSON.parse(fs.readFileSync(path.join(REPO, 'data/kodoku-r3/conclusion-equilibrium-deme.json'), 'utf8'));
const beliefPoolJson = fs.readFileSync(path.join(REPO, 'data/belief-pool-v3/belief-pool.json'), 'utf8');
const baseBeliefPool = JSON.parse(beliefPoolJson);
let compiledPreview = null;
try {
  compiledPreview = JSON.parse(fs.readFileSync(path.join(REPO, 'data/compiled-preview-v1.json'), 'utf8'));
} catch {}

// Skeuomorphic "1 human player's sub-party box" for Tier A (mercy/handicap mode):
// 8 distinct iconic human-origin archetypes from data/team-pool-v2/team-pool.json
const TIER_A_BOX_IDS = [
  'sample-07',        // マルマイン/ケンタロス55/パルシェン/ガラガラ55/カビゴン/サンダー55
  'sample-08',        // ナッシー/カイリキー/ミルタンク/パルシェン/カビゴン55/サンダー55
  'sample-12',        // ゲンガー/スターミー/パルシェン/ブラッキー/ガラガラ55/カビゴン55
  'mjj-2020-rraikou', // ライコウ55/パルシェン/カビゴン55/ガラガラ/ヘラクロス/カイリキー
  'mjj-2014-thdrcomb',// サンダー55/バンギラス55/フシギバナ/ハガネール/パルシェン/カビゴン
  'sample-17-orig',   // ハガネール/ナッシー/ポリゴン2-55/カビゴン55/サンダー/スイクン
  'sample-11',        // フーディン/エアームド/ハピナス/フシギバナ/スイクン55/バンギラス55
  'rental-cban-8',    // バンギラス55/ナッシー/フシギバナ/ゴローニャ/カビゴン55/ミルタンク
];

function toId(s) {
  return String(s || '')
    .normalize('NFD')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '');
}

function normJp(s) {
  return String(s || '')
    .replace(/<[^>]*>/g, '')
    .replace(/[\u2212\uff0d\u2015\u2010\u2013\u2014\-]/g, 'ー')
    .replace(/[◯○●♂♀\s　]/g, '')
    .trim();
}

const JP_TO_SPECIES_ID = {};
for (const [id, jp] of Object.entries(i18n.species)) {
  JP_TO_SPECIES_ID[normJp(jp)] = id;
}

const JP_TO_MOVE_ID = {};
for (const [id, jp] of Object.entries(i18n.moves)) {
  if (id.startsWith('hiddenpower') && id !== 'hiddenpower') continue;
  JP_TO_MOVE_ID[normJp(jp)] = id;
}
JP_TO_MOVE_ID[normJp('めざめるパワー')] = 'hiddenpower';

const JP_TO_ITEM_ID = {};
for (const [id, jp] of Object.entries(i18n.items)) {
  JP_TO_ITEM_ID[normJp(jp)] = id;
}
// Common PBS / Gen 2 item spelling aliases
Object.assign(JP_TO_ITEM_ID, {
  [normJp('おうごんのきのみ')]: 'goldberry',
  [normJp('まひなおしのきのみ')]: 'przcureberry',
  [normJp('どくけしのきのみ')]: 'psncureberry',
  [normJp('はっかのきのみ')]: 'mintberry',
  [normJp('きせきのきのみ')]: 'miracleberry',
  [normJp('ふしぎのきのみ')]: 'mysteryberry',
  [normJp('こおりなおしのきのみ')]: 'iceberry',
  [normJp('やけどなおしのきのみ')]: 'burntberry',
});

// ------------------------------------------------------------------ RNG & Teams
let rngState = (SEED ^ 0x9e3779b9) >>> 0;
function rng() {
  rngState = (rngState + 0x6d2b79f5) >>> 0;
  let t = rngState;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
function rngInt(n) {
  return Math.floor(rng() * n);
}

function pickTeam(specOverride) {
  const spec = String(specOverride || TEAMSPEC);
  if (spec === 'pool:nash' || spec === 'nash' || spec === 'default') {
    const teams = nashDeme.teams;
    const total = teams.reduce((a, t) => a + (t.weight || 0), 0);
    let r = rng() * total;
    let chosen = teams[teams.length - 1];
    for (const t of teams) {
      if (r < (t.weight || 0)) {
        chosen = t;
        break;
      }
      r -= (t.weight || 0);
    }
    return { id: chosen.id, sets: chosen.sets, label: `pool:nash(${chosen.id})`, tier: 'nash' };
  }
  if (spec === 'pool:tier-a' || spec === 'pool:tiera' || spec === 'tier-a' || spec === 'mercy') {
    const boxTeams = ownPool.teams.filter(t => TIER_A_BOX_IDS.includes(t.id));
    const candidates = boxTeams.length > 0
      ? boxTeams
      : ownPool.teams.filter(t => t.provenance?.origin === 'human');
    const chosen = candidates[rngInt(candidates.length)];
    return { id: chosen.id, sets: chosen.sets, label: `pool:tier-a(${chosen.id})`, tier: 'tier-a' };
  }
  if (spec.startsWith('pool:')) {
    const which = spec.slice(5);
    const teams = ownPool.teams;
    let idx = -1;
    if (which === 'random') {
      const total = teams.reduce((a, t) => a + (t.drawWeight || 1), 0);
      let r = rng() * total;
      idx = teams.length - 1;
      for (let i = 0; i < teams.length; i++) {
        const w = teams[i].drawWeight || 1;
        if (r < w) {
          idx = i;
          break;
        }
        r -= w;
      }
    } else {
      idx = teams.findIndex(t => t.id === which);
      if (idx < 0 && /^\d+$/.test(which)) idx = parseInt(which, 10);
    }
    if (!(idx >= 0 && idx < teams.length)) {
      throw new Error(`Unknown team spec: ${spec}`);
    }
    return { id: teams[idx].id, sets: teams[idx].sets, label: `pool:${teams[idx].id}`, tier: 'catalog' };
  }
  const raw = JSON.parse(fs.readFileSync(path.resolve(spec), 'utf8'));
  const sets = Array.isArray(raw) ? raw : raw.sets;
  if (!Array.isArray(sets) || sets.length === 0) {
    throw new Error(`${spec}: expected JSON array of sets or { sets: [...] }`);
  }
  return { id: raw.id || path.basename(spec), sets, label: spec, tier: 'custom' };
}

/**
 * Encode a 6-Pokemon team into PBS `PD0` Cookie format.
 * Applies the PBS `pbsbugreport.txt` workaround where `bitterberry` (84, broken in PBS)
 * is mapped to `iceberry` (81, which cures both burn and confusion in PBS).
 */
function encodeTeamForPbsCookie(team) {
  const clonedSets = team.sets.map(s => {
    const itemId = toId(s.item || '');
    if (itemId === 'bitterberry') {
      return { ...s, item: 'Ice Berry' };
    }
    return s;
  });
  return encodeTeamForPdinput({ id: team.id || 'bot', sets: clonedSets });
}

// ------------------------------------------------------------------ Pure-JS EUC-JP helpers
// Built by inverting Node's built-in TextDecoder('euc-jp') over JIS X 0208 & halfwidth kana
// so the client has zero external binary dependencies (`iconv` not needed on Windows).
const eucJpDecoder = new TextDecoder('euc-jp');
const UTF8_TO_EUCJP_MAP = (() => {
  const map = new Map();
  for (let b1 = 0xa1; b1 <= 0xfe; b1++) {
    for (let b2 = 0xa1; b2 <= 0xfe; b2++) {
      const ch = eucJpDecoder.decode(Uint8Array.from([b1, b2]));
      if (ch.length === 1 && ch !== '\uFFFD' && !map.has(ch)) {
        map.set(ch, [b1, b2]);
      }
    }
  }
  for (let b2 = 0xa1; b2 <= 0xdf; b2++) {
    const ch = eucJpDecoder.decode(Uint8Array.from([0x8e, b2]));
    if (ch.length === 1 && ch !== '\uFFFD' && !map.has(ch)) {
      map.set(ch, [0x8e, b2]);
    }
  }
  return map;
})();

function encodeEucJpPercent(str) {
  const s = String(str ?? '');
  let out = '';
  for (const ch of s) {
    const code = ch.codePointAt(0);
    if (code < 0x80) {
      if (
        (code >= 0x30 && code <= 0x39) ||
        (code >= 0x41 && code <= 0x5a) ||
        (code >= 0x61 && code <= 0x7a) ||
        code === 0x2d || code === 0x5f || code === 0x2e || code === 0x2a
      ) {
        out += ch;
      } else if (code === 0x20) {
        out += '+';
      } else {
        out += '%' + code.toString(16).toUpperCase().padStart(2, '0');
      }
    } else {
      const bytes = UTF8_TO_EUCJP_MAP.get(ch);
      if (bytes) {
        for (const b of bytes) {
          out += '%' + b.toString(16).toUpperCase().padStart(2, '0');
        }
      }
    }
  }
  return out;
}

function encodeFormEucJp(fields) {
  const parts = [];
  for (const [k, v] of Object.entries(fields)) {
    if (Array.isArray(v)) {
      for (const item of v) {
        parts.push(`${encodeEucJpPercent(k)}=${encodeEucJpPercent(item)}`);
      }
    } else if (v !== undefined && v !== null) {
      parts.push(`${encodeEucJpPercent(k)}=${encodeEucJpPercent(v)}`);
    }
  }
  return parts.join('&');
}

// ------------------------------------------------------------------ HTTP Session
class PbsHttpClient {
  constructor() {
    this.cookies = new Map();
  }

  setCookie(name, value) {
    this.cookies.set(name, value);
  }

  cookieHeader() {
    if (this.cookies.size === 0) return '';
    return [...this.cookies.entries()].map(([k, v]) => `${k}=${v}`).join('; ');
  }

  ingestSetCookieHeaders(setCookieHeader) {
    if (!setCookieHeader) return;
    const list = Array.isArray(setCookieHeader) ? setCookieHeader : [setCookieHeader];
    for (const raw of list) {
      const first = String(raw).split(';')[0].trim();
      const eq = first.indexOf('=');
      if (eq > 0) {
        const k = first.slice(0, eq).trim();
        const v = first.slice(eq + 1).trim();
        this.cookies.set(k, v);
      }
    }
  }

  async request(urlStr, options = {}) {
    const method = options.method || 'GET';
    const body = options.body !== undefined && options.body !== null ? String(options.body) : null;
    const maxAttempts = options.maxAttempts || 6;
    const timeoutMs = options.timeoutMs || 10000;

    for (let attempt = 1; attempt <= maxAttempts; attempt++) {
      try {
        const result = await new Promise((resolve, reject) => {
          const u = new URL(urlStr);
          const port = parseInt(u.port || '80', 10);
          const reqPath = (u.pathname || '/') + (u.search || '');
          const headerLines = [
            `${method} ${reqPath} HTTP/1.0`,
            `Host: ${u.host}`,
            'User-Agent: Mozilla/5.0 (compatible; NC2000Bot/1.0)',
            'Accept: text/html,application/xhtml+xml,*/*',
            'Connection: close',
          ];
          const cookieStr = this.cookieHeader();
          if (cookieStr) {
            headerLines.push(`Cookie: ${cookieStr}`);
          }
          const bodyBuf = body !== null ? Buffer.from(body, 'ascii') : null;
          if (bodyBuf !== null) {
            headerLines.push('Content-Type: application/x-www-form-urlencoded');
            headerLines.push(`Content-Length: ${bodyBuf.length}`);
          }
          const headBuf = Buffer.from(headerLines.join('\r\n') + '\r\n\r\n', 'ascii');

          const sock = net.connect(port, u.hostname);
          const chunks = [];
          let settled = false;
          const finishErr = err => {
            if (settled) return;
            settled = true;
            try { sock.destroy(); } catch {}
            reject(err);
          };

          sock.setTimeout(timeoutMs);
          sock.on('timeout', () => finishErr(new Error(`HTTP/1.0 socket timeout after ${timeoutMs}ms`)));
          sock.on('error', err => finishErr(err));
          sock.on('connect', () => {
            sock.write(headBuf);
            if (bodyBuf !== null) sock.write(bodyBuf);
          });
          sock.on('data', c => chunks.push(c));
          sock.on('end', () => {
            if (settled) return;
            settled = true;
            const raw = Buffer.concat(chunks);
            let sepIdx = raw.indexOf('\r\n\r\n');
            let sepLen = 4;
            if (sepIdx < 0) {
              sepIdx = raw.indexOf('\n\n');
              sepLen = 2;
            }
            const headText = sepIdx >= 0 ? raw.slice(0, sepIdx).toString('latin1') : '';
            const bodyRaw = sepIdx >= 0 ? raw.slice(sepIdx + sepLen) : raw;
            const lines = headText.split(/\r?\n/);
            const statusLine = lines[0] || '';
            const stMatch = statusLine.match(/^HTTP\/\d+\.\d+\s+(\d+)/i);
            const statusCode = stMatch ? parseInt(stMatch[1], 10) : 200;
            const headers = {};
            const setCookies = [];
            for (let i = 1; i < lines.length; i++) {
              const colon = lines[i].indexOf(':');
              if (colon > 0) {
                const k = lines[i].slice(0, colon).trim().toLowerCase();
                const v = lines[i].slice(colon + 1).trim();
                if (k === 'set-cookie') {
                  setCookies.push(v);
                } else {
                  headers[k] = v;
                }
              }
            }
            if (setCookies.length > 0) {
              headers['set-cookie'] = setCookies;
              this.ingestSetCookieHeaders(setCookies);
            }
            const html = eucJpDecoder.decode(bodyRaw);
            resolve({
              statusCode,
              headers,
              location: headers.location || '',
              html,
              rawBuf: bodyRaw,
            });
          });
        });
        return result;
      } catch (err) {
        if (attempt >= maxAttempts) throw err;
        await sleep(500 * attempt);
      }
    }
    throw new Error('Unreachable');
  }

  async get(urlStr, options = {}) {
    return this.request(urlStr, { ...options, method: 'GET' });
  }

  async postForm(urlStr, fields, options = {}) {
    const body = typeof fields === 'string' ? fields : encodeFormEucJp(fields);
    return this.request(urlStr, { ...options, method: 'POST', body });
  }
}

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

// ------------------------------------------------------------------ HTML Parsers
function classifyPage(res) {
  const loc = res.location || '';
  const html = res.html || '';

  if ((res.statusCode === 301 || res.statusCode === 302) && /PDINPUT\.cgi/i.test(loc)) {
    const u = new URL(loc, ROOM_URL);
    const sim = u.searchParams.get('sim') || '';
    return { kind: 'REDIRECT_PDINPUT', location: u.toString(), sim };
  }
  if ((res.statusCode === 301 || res.statusCode === 302) && /PS\.html/i.test(loc)) {
    return { kind: 'FINISHED', reason: 'redirect_ps_html', html };
  }
  if ((res.statusCode === 301 || res.statusCode === 302) && loc) {
    return { kind: 'REDIRECT_OTHER', location: new URL(loc, ROOM_URL).toString() };
  }

  if (/<title>\s*ERROR!\s*<\/title>/i.test(html) || /<h1>\s*ERROR!\s*<\/h1>/i.test(html)) {
    const msgMatch = html.match(/<h1>\s*ERROR!\s*<\/h1>\s*([\s\S]*?)(?:<hr|<\/body|$)/i);
    const errText = msgMatch ? msgMatch[1].replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ').trim() : 'ERROR';
    return { kind: 'ERROR', errorText: errText, html };
  }

  if (
    /ＷＩＮ！/.test(html) ||
    /ＬＯＳＥ/.test(html) ||
    /ＤＲＡＷ/.test(html) ||
    /PS\.cgi\?ACTION=END/i.test(html)
  ) {
    return { kind: 'FINISHED', reason: 'match_end_page', html };
  }

  if (/エントリー(?:完了)?しました/i.test(html) && !/name="?ACTION"?/i.test(html)) {
    return { kind: 'WAITING_ENTRY', html };
  }
  if (/入力終了、相手の入力を待って下さい/i.test(html)) {
    return { kind: 'WAITING_SELECT', html };
  }
  if (/入力が完了しました/i.test(html) && !/name="?ACTION"?/i.test(html)) {
    return { kind: 'SELECT_DONE', html };
  }
  if (
    /相手の入力を待っています/i.test(html) ||
    /<title>[^<]*入力待ち[^<]*<\/title>/i.test(html)
  ) {
    return { kind: 'WAITING_TURN', html };
  }

  if (/name="?ACTION"?\s+value="?ENTORY"?/i.test(html)) {
    const waitMatch = html.match(/<font[^>]*>\s*([^<\s]+?)さんがエントリーしています。\s*<\/font>/i);
    return {
      kind: 'ENTORY',
      p1Waiting: Boolean(waitMatch),
      p1Name: waitMatch ? waitMatch[1].trim() : null,
      html,
    };
  }

  if (/name="?ACTION"?\s+value="?CHECK"?/i.test(html)) {
    return { kind: 'TEAM_PREVIEW_CHECK', html };
  }
  if (/name="?ACTION"?\s+value="?SELECT"?/i.test(html)) {
    return { kind: 'TEAM_PREVIEW_SELECT', html };
  }
  if (/name="?ACTION"?\s+value="?CHANGE"?/i.test(html)) {
    return { kind: 'BATTLE_CHANGE', html };
  }
  if (/name="?ACTION"?\s+value="?INPUT"?/i.test(html)) {
    return { kind: 'BATTLE_INPUT', html };
  }

  return { kind: 'UNKNOWN', html };
}

/**
 * Parse the 2-column team preview table in `TEAM_PREVIEW_CHECK` HTML.
 * Returns `{ p1Name, p2Name, p1Mons, p2Mons }` where each mon is
 * `{ species, speciesId, level, gender, item_flag }`.
 */
function parseTeamPreviewHtml(html) {
  const result = {
    p1Name: 'P1',
    p2Name: 'P2',
    p1Mons: [],
    p2Mons: [],
  };

  const nameRowMatch = html.match(
    /<tr[^>]*>\s*<t[dh][^>]*>\s*([^<]*?)\s*<\/t[dh]>\s*<td[^>]*ROWSPAN="?2"?[^>]*>[\s\S]*?<\/td>\s*<t[dh][^>]*>\s*([^<]*?)\s*<\/t[dh]>\s*<\/tr>/i
  );
  if (nameRowMatch) {
    result.p1Name = nameRowMatch[1].trim();
    result.p2Name = nameRowMatch[2].trim();
  }

  const tdMatches = [...html.matchAll(/<td[^>]*VAlign="?middle"?[^>]*>([\s\S]*?)<\/td>/gi)];
  if (tdMatches.length >= 2) {
    result.p1Mons = parsePreviewTtCell(tdMatches[0][1]);
    result.p2Mons = parsePreviewTtCell(tdMatches[1][1]);
  }
  return result;
}

function parsePreviewTtCell(ttHtml) {
  const lines = String(ttHtml)
    .split(/<br\s*\/?>/i)
    .map(l => l.replace(/\r|\n/g, '').trim())
    .filter(Boolean);

  const mons = [];
  for (const line of lines) {
    const itemFlag = /[◯○●]/.test(line);
    const lvMatch = line.match(/LV\s*(\d+)/i);
    const level = lvMatch ? parseInt(lvMatch[1], 10) : 50;
    const beforeLv = line.split(/<b>|LV\s*\d+/i)[0] || line;
    const gender = beforeLv.includes('♂') ? 'M' : beforeLv.includes('♀') ? 'F' : '';
    const jpName = normJp(beforeLv);
    const speciesId = JP_TO_SPECIES_ID[jpName] || 'pikachu';
    const spObj = dexData.species[speciesId];
    mons.push({
      species: spObj ? spObj.name : 'Pikachu',
      speciesId,
      level,
      gender,
      item_flag: itemFlag,
    });
  }
  return mons;
}

function parseStatusText(raw) {
  const s = String(raw || '').replace(/<[^>]*>/g, '').replace(/[\s　\[\]]+/g, '').trim();
  if (!s || s === '通常') return '';
  if (s.includes('ひんし') || s.includes('瀕死') || s.includes('FNT')) return 'fnt';
  if (s.includes('ねむり') || s.includes('眠')) return 'slp';
  if (s.includes('まひ') || s.includes('麻痺')) return 'par';
  if (s.includes('こおり') || s.includes('氷') || s.includes('凍')) return 'frz';
  if (s.includes('やけど') || s.includes('火傷')) return 'brn';
  if (s.includes('猛毒') || s.includes('もうどく')) return 'tox';
  if (s.includes('どく') || s.includes('毒')) return 'psn';
  return '';
}

/**
 * Parse a `BATTLE_INPUT` or `BATTLE_CHANGE` page:
 * - `nowVal`: value of hidden input `NOW`
 * - `options`: array of `{ value, label, kind: 'move'|'switch', slotIndex, moveId, pp, speciesId, hp, maxHp }`
 * - `sides`: `[p1ActiveSummary, p2ActiveSummary]` from the 4-row top status table
 */
function parseBattleTurnHtml(html) {
  const nowMatch =
    html.match(/name="?NOW"?\s+value="?([^"\s>]*)"?/i) ||
    html.match(/value="?([^"\s>]*)"?\s+name="?NOW"?/i);
  const nowVal = nowMatch ? nowMatch[1] : '';

  const options = [];
  const selectMatch = html.match(/<select[^>]*name="?MOVE"?[^>]*>([\s\S]*?)<\/select>/i);
  if (selectMatch) {
    const optRegex = /<option[^>]*value="?([^"\s>]+)"?[^>]*>([^<\r\n]*)/gi;
    for (const m of selectMatch[1].matchAll(optRegex)) {
      const value = m[1].trim();
      const label = m[2].replace(/\s+/g, ' ').trim();
      if (/^W(\d)$/i.test(value)) {
        const slotIndex = parseInt(value.slice(1), 10); // 1..4
        const ppMatch = label.match(/\[\s*(\d+)\s*\]/);
        const pp = ppMatch ? parseInt(ppMatch[1], 10) : null;
        const moveNameJp = normJp(label.replace(/\[\s*\d+\s*\].*$/, ''));
        const moveId = JP_TO_MOVE_ID[moveNameJp] || null;
        options.push({
          value: value.toUpperCase(),
          label,
          kind: 'move',
          slotIndex,
          moveId,
          pp,
        });
      } else if (/^C(\d)$/i.test(value)) {
        const slotIndex = parseInt(value.slice(1), 10); // 1..3
        const hpMatch = label.match(/\[\s*(\d+)\s*\/\s*(\d+)\s*\]/);
        const hp = hpMatch ? parseInt(hpMatch[1], 10) : null;
        const maxHp = hpMatch ? parseInt(hpMatch[2], 10) : null;
        const spNameJp = normJp(label.replace(/\[\s*\d+\s*\/\s*\d+\s*\].*$/, ''));
        const speciesId = JP_TO_SPECIES_ID[spNameJp] || null;
        options.push({
          value: value.toUpperCase(),
          label,
          kind: 'switch',
          slotIndex,
          speciesId,
          hp,
          maxHp,
        });
      } else {
        options.push({
          value,
          label,
          kind: 'other',
        });
      }
    }
  }

  // Parse the 4-row top status table
  const sides = [
    { name: 'P1', speciesId: null, gender: '', level: 50, hp: null, maxHp: null, status: '' },
    { name: 'P2', speciesId: null, gender: '', level: 50, hp: null, maxHp: null, status: '' },
  ];

  // Row 2: <td NOWRAP Width="150" Align="center">フ−ディン♂ L55</td>
  const monRowMatch = html.match(
    /<TR[^>]*>\s*<TD[^>]*>\s*([^<>]*?L\d+[^<>]*?)\s*<\/TD>\s*<TD[^>]*>\s*([^<>]*?L\d+[^<>]*?)\s*<\/TD>\s*<\/TR>/i
  );
  if (monRowMatch) {
    for (let s = 0; s < 2; s++) {
      const cell = monRowMatch[s + 1].replace(/<[^>]*>/g, '').trim();
      const lvM = cell.match(/L(\d+)/i);
      if (lvM) sides[s].level = parseInt(lvM[1], 10);
      const beforeL = cell.replace(/L\d+.*$/i, '');
      sides[s].gender = beforeL.includes('♂') ? 'M' : beforeL.includes('♀') ? 'F' : '';
      const spId = JP_TO_SPECIES_ID[normJp(beforeL)] || null;
      sides[s].speciesId = spId;
    }
  }

  // Row 3: <td NOWRAP Width="150" Align="center">ＨＰ 176/176</td>
  const hpRowMatch = html.match(
    /<TR[^>]*>\s*<TD[^>]*>\s*(?:ＨＰ|HP)\s*(\d+)\s*\/\s*(\d+)\s*<\/TD>\s*<TD[^>]*>\s*(?:ＨＰ|HP)\s*(\d+)\s*\/\s*(\d+)\s*<\/TD>\s*<\/TR>/i
  );
  if (hpRowMatch) {
    sides[0].hp = parseInt(hpRowMatch[1], 10);
    sides[0].maxHp = parseInt(hpRowMatch[2], 10);
    sides[1].hp = parseInt(hpRowMatch[3], 10);
    sides[1].maxHp = parseInt(hpRowMatch[4], 10);

    // Row 4 is immediately after Row 3
    const afterHp = html.slice(hpRowMatch.index + hpRowMatch[0].length);
    const stRowMatch = afterHp.match(/^\s*<TR[^>]*>\s*<TD[^>]*>([\s\S]*?)<\/TD>\s*<TD[^>]*>([\s\S]*?)<\/TD>\s*<\/TR>/i);
    if (stRowMatch) {
      sides[0].status = parseStatusText(stRowMatch[1]);
      sides[1].status = parseStatusText(stRowMatch[2]);
    }
  }

  return { nowVal, options, sides };
}

/**
 * Parse `?ACTION=PDAT` response HTML.
 * Returns `{ mons: [{ speciesId, level, gender, status, hp, maxHp, item, moves: [{ moveId, pp }] }], foeRemaining }`.
 */
function parsePdatHtml(html) {
  const mons = [];
  // Table format: <tr><TD ALIGN="right">55</TD><TD ALIGN="center">フ−ディン♂</TD><TD ALIGN="center">176／176</TD><TD ALIGN="center">　</TD><TD ALIGN="center">サイコキネシス(16)</TD>...<TD ALIGN="center">まひなおしのみ</TD></tr>
  const rowRegex = /<tr[^>]*>\s*<td[^>]*>\s*(\d+)\s*<\/td>\s*<td[^>]*>\s*([^<]+?)\s*<\/td>\s*<td[^>]*>\s*(\d+)\s*[\/／]\s*(\d+)\s*<\/td>\s*<td[^>]*>([\s\S]*?)<\/td>\s*<td[^>]*>([\s\S]*?)<\/td>\s*<td[^>]*>([\s\S]*?)<\/td>\s*<td[^>]*>([\s\S]*?)<\/td>\s*<td[^>]*>([\s\S]*?)<\/td>\s*<td[^>]*>([\s\S]*?)<\/td>\s*<\/tr>/gi;

  for (const m of html.matchAll(rowRegex)) {
    const level = parseInt(m[1], 10);
    const rawName = m[2].trim();
    const gender = rawName.includes('♂') ? 'M' : rawName.includes('♀') ? 'F' : '';
    const speciesId = JP_TO_SPECIES_ID[normJp(rawName)] || null;
    const hp = parseInt(m[3], 10);
    const maxHp = parseInt(m[4], 10);
    const status = parseStatusText(m[5]);
    const moves = [];
    for (let i = 6; i <= 9; i++) {
      const cell = m[i].replace(/<[^>]*>/g, '').trim();
      const mvMatch = cell.match(/^(.+?)\s*\(\s*(\d+)\s*\)$/);
      if (mvMatch) {
        const mvId = JP_TO_MOVE_ID[normJp(mvMatch[1])] || null;
        const pp = parseInt(mvMatch[2], 10);
        if (mvId) moves.push({ moveId: mvId, pp });
      }
    }
    const rawItem = normJp(m[10] || '');
    const item = rawItem && rawItem !== 'なし' ? (JP_TO_ITEM_ID[rawItem] || '') : '';
    mons.push({
      speciesId,
      level,
      gender,
      status: hp <= 0 ? 'fnt' : status,
      hp,
      maxHp,
      item,
      moves,
    });
  }

  const remMatch = html.match(/相手の残り[：:]\s*(\d+)/);
  const foeRemaining = remMatch ? parseInt(remMatch[1], 10) : null;
  return { mons, foeRemaining };
}

// ------------------------------------------------------------------ Exact Gen 2 MaxHP & Hidden Power DV Leak Inference
const HP_TYPES_LIST = [
  'Fighting', 'Flying', 'Poison', 'Ground',
  'Rock', 'Bug', 'Ghost', 'Steel',
  'Fire', 'Water', 'Grass', 'Electric',
  'Psychic', 'Ice', 'Dragon', 'Dark',
];

const HP_TYPE_INDEX_BY_ID = Object.fromEntries(
  HP_TYPES_LIST.map((t, i) => [toId(t), i])
);

const HP_DVS_BY_TYPE_ID = {
  bug:      { atk: 13, def: 13 },
  dark:     { atk: 15, def: 15 },
  dragon:   { atk: 15, def: 14 },
  electric: { atk: 14, def: 15 },
  fighting: { atk: 12, def: 12 },
  fire:     { atk: 14, def: 12 },
  flying:   { atk: 12, def: 13 },
  ghost:    { atk: 13, def: 14 },
  grass:    { atk: 14, def: 14 },
  ground:   { atk: 12, def: 15 },
  ice:      { atk: 15, def: 13 },
  poison:   { atk: 12, def: 14 },
  psychic:  { atk: 15, def: 12 },
  rock:     { atk: 13, def: 12 },
  steel:    { atk: 13, def: 15 },
  water:    { atk: 14, def: 13 },
};

/**
 * Compute exact Gen 2 MaxHP for a set in `belief-pool-v3` or `team-pool-v2` at a given level.
 * Mirrors `crates/bot/src/belief.rs::resolve_dvs` and `Dex::max_hp`.
 */
function calcSetMaxHp(set, levelOverride) {
  if (!set) return null;
  const spId = toId(set.species);
  const sp = dexData.species[spId];
  if (!sp || !sp.baseStats) return null;
  const level = levelOverride || set.level || 50;

  let atk = 15, def = 15, spe = 15, spc = 15;
  for (const m of set.moves || []) {
    const mId = toId(m);
    if (mId.startsWith('hiddenpower') && mId.length > 'hiddenpower'.length) {
      const hpType = mId.slice('hiddenpower'.length);
      if (HP_DVS_BY_TYPE_ID[hpType]) {
        atk = HP_DVS_BY_TYPE_ID[hpType].atk;
        def = HP_DVS_BY_TYPE_ID[hpType].def;
      }
    }
  }
  if (set.ivs) {
    if (set.ivs.atk !== undefined) atk = Math.floor(set.ivs.atk / 2);
    if (set.ivs.def !== undefined) def = Math.floor(set.ivs.def / 2);
    if (set.ivs.spe !== undefined) spe = Math.floor(set.ivs.spe / 2);
    if (set.ivs.spa !== undefined) spc = Math.floor(set.ivs.spa / 2);
  }
  if (set.dvs) {
    if (set.dvs.atk !== undefined) atk = set.dvs.atk;
    if (set.dvs.def !== undefined) def = set.dvs.def;
    if (set.dvs.spe !== undefined) spe = set.dvs.spe;
    if (set.dvs.spc !== undefined) spc = set.dvs.spc;
  }

  let hpDv = ((atk & 1) << 3) | ((def & 1) << 2) | ((spe & 1) << 1) | (spc & 1);
  if (set.ivs && set.ivs.hp !== undefined) {
    hpDv = Math.floor(set.ivs.hp / 2);
  }
  if (set.dvs && set.dvs.hp !== undefined) {
    hpDv = set.dvs.hp;
  }

  const evHp = set.evs && set.evs.hp !== undefined ? set.evs.hp : 252;
  const evTerm = Math.min(63, Math.floor(evHp / 4));
  const baseHp = sp.baseStats.hp;
  return Math.floor(((baseHp + hpDv) * 2 + evTerm) * level / 100) + level + 10;
}

/**
 * Given a Pokemon's species, level, gender, and observed exact `maxHp` from `PS.cgi`,
 * infer the 16-bit `hp_type_mask` over `HP_TYPES_LIST` (`crates/bot/src/hp_evidence.rs`).
 * Assuming standard max HP StatExp (`evTerm = 63`), `observedMaxHp` narrows `hpDv`,
 * which fixes `(atkDv & 1)` (bit 3 of `hpDv`) and `(defDv & 1)` (bit 2 of `hpDv`),
 * reducing the 16 Hidden Power types to at most 4 compatible types!
 */
function inferHpTypeMaskFromMaxHp(speciesId, level, gender, observedMaxHp) {
  const sp = dexData.species[toId(speciesId)];
  if (!sp || !sp.baseStats || !observedMaxHp || observedMaxHp <= 0) return 0xffff;

  // Compute gender-legal HP type mask (`hp_evidence::possible_types`)
  const ratio = sp.genderRatio
    ? Math.round((sp.genderRatio.F || 0) * 254)
    : (sp.gender === 'N' ? 255 : sp.gender === 'F' ? 254 : sp.gender === 'M' ? 0 : 127);

  let atkMin = 0, atkMax = 15;
  if (ratio > 0 && ratio < 254) {
    const threshold = Math.floor(ratio / 16);
    if (gender === 'F') {
      atkMax = Math.max(0, threshold - 1);
    } else if (gender === 'M') {
      atkMin = threshold;
    }
  }

  let genderPossibleMask = 0;
  for (let a = atkMin; a <= atkMax; a++) {
    for (let d = 0; d <= 15; d++) {
      const idx = 4 * (a & 3) + (d & 3);
      genderPossibleMask |= (1 << idx);
    }
  }

  // Match `hpDv` under full HP EVs (`evTerm = 63`)
  let maxHpMask = 0;
  for (let hpDv = 0; hpDv <= 15; hpDv++) {
    const mHp = Math.floor(((sp.baseStats.hp + hpDv) * 2 + 63) * level / 100) + level + 10;
    if (mHp === observedMaxHp) {
      const atkParity = (hpDv >> 3) & 1;
      const defParity = (hpDv >> 2) & 1;
      for (let a = atkMin; a <= atkMax; a++) {
        if ((a & 1) !== atkParity) continue;
        for (let d = 0; d <= 15; d++) {
          if ((d & 1) !== defParity) continue;
          const idx = 4 * (a & 3) + (d & 3);
          maxHpMask |= (1 << idx);
        }
      }
    }
  }

  const intersection = maxHpMask & genderPossibleMask;
  return intersection !== 0 ? intersection : 0xffff;
}

/**
 * Parse `?ACTION=LLOG` cumulative battle log HTML to extract public battle facts:
 * - appeared mons, fainted mons, move uses per mon, active boosts, weather,
 *   side conditions (Reflect, Light Screen, Spikes), key volatiles, and item reveals.
 */
function parseLlogHtml(html, previewSides) {
  const info = {
    sides: [
      {
        mons: (previewSides[0] || []).map(() => ({
          appeared: false,
          fainted: false,
          status: '',
          rest: false,
          boosts: [0, 0, 0, 0, 0, 0, 0],
          uses: {},
          revealedMoves: new Set(),
          itemOriginal: null,
          itemCurrent: null,
          volatiles: new Map(),
        })),
        activeSlot: null,
        conditions: new Map(),
      },
      {
        mons: (previewSides[1] || []).map(() => ({
          appeared: false,
          fainted: false,
          status: '',
          rest: false,
          boosts: [0, 0, 0, 0, 0, 0, 0],
          uses: {},
          revealedMoves: new Set(),
          itemOriginal: null,
          itemCurrent: null,
          volatiles: new Map(),
        })),
        activeSlot: null,
        conditions: new Map(),
      },
    ],
    weather: null,
  };

  if (!html) return info;

  const findSlot = (sideIdx, text) => {
    const roster = previewSides[sideIdx] || [];
    for (let i = 0; i < roster.length; i++) {
      const spId = roster[i].speciesId;
      const jp = i18n.species[spId];
      if (!jp) continue;
      if (normJp(text).includes(normJp(jp))) return i;
    }
    return info.sides[sideIdx].activeSlot;
  };

  const setActive = (sideIdx, slot) => {
    if (slot === null || slot === undefined || slot < 0) return;
    const side = info.sides[sideIdx];
    if (side.activeSlot !== null && side.activeSlot !== slot && side.mons[side.activeSlot]) {
      side.mons[side.activeSlot].boosts = [0, 0, 0, 0, 0, 0, 0];
      side.mons[side.activeSlot].volatiles.clear();
    }
    side.activeSlot = slot;
    if (side.mons[slot]) {
      side.mons[slot].appeared = true;
    }
  };

  const CONSUMED_BERRIES_JP = [
    ['おうごんのきのみ', 'goldberry'],
    ['きせきのきのみ', 'miracleberry'],
    ['はっかのきのみ', 'mintberry'],
    ['まひなおしのみ', 'przcureberry'],
    ['こおりなおしのみ', 'iceberry'],
    ['やけどなおしのみ', 'burntberry'],
    ['どくけしのみ', 'psncureberry'],
    ['にがきのみ', 'bitterberry'],
    ['ふしぎのきのみ', 'mysteryberry'],
    ['きのみジュース', 'berryjuice'],
    ['きのみ', 'berry'],
  ];

  // Walk paragraph blocks `<P ALIGN="left|right">...</P>` or turn blocks
  const pRegex = /<P\s+ALIGN="?(left|right)"?\s*>([\s\S]*?)(?=<P\s+ALIGN|<HR|<\/BODY|$)/gi;
  let turnNum = 1;

  for (const m of html.matchAll(pRegex)) {
    const sideIdx = m[1].toLowerCase() === 'left' ? 0 : 1;
    const foeIdx = 1 - sideIdx;
    const block = m[2];
    const clean = block.replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ').trim();

    // Check switch-in / send-out cues
    if (
      clean.includes('を繰り出した') ||
      clean.includes('をくりだした') ||
      clean.includes('ゆけっ！') ||
      clean.includes('いけっ！') ||
      clean.includes('出てこい')
    ) {
      const sendOutB = block.match(/<B>\s*([^<]+?)\s*<\/B>\s*を(?:繰り出した|くりだした)/i);
      const slot = sendOutB ? findSlot(sideIdx, sendOutB[1]) : findSlot(sideIdx, clean);
      if (slot !== null) setActive(sideIdx, slot);
    }

    // Check move usage: `<B>MOVE</B>` or `の MOVE ！`
    const bMatches = [...block.matchAll(/<B>\s*([^<]+?)\s*<\/B>/gi)];
    for (const bm of bMatches) {
      const mvNorm = normJp(bm[1]);
      const mvId = JP_TO_MOVE_ID[mvNorm];
      if (mvId) {
        const actorSlot = info.sides[sideIdx].activeSlot ?? findSlot(sideIdx, clean);
        if (actorSlot !== null && info.sides[sideIdx].mons[actorSlot]) {
          setActive(sideIdx, actorSlot);
          const mon = info.sides[sideIdx].mons[actorSlot];
          mon.uses[mvId] = (mon.uses[mvId] || 0) + 1;
          mon.revealedMoves.add(mvId);
          if (mvId === 'rest') {
            mon.rest = true;
            mon.status = 'slp';
          } else if (mvId === 'raindance') {
            info.weather = { key: 'raindance', upkeeps: 0 };
          } else if (mvId === 'sunnyday') {
            info.weather = { key: 'sunnyday', upkeeps: 0 };
          } else if (mvId === 'sandstorm') {
            info.weather = { key: 'sandstorm', upkeeps: 0 };
          } else if (mvId === 'reflect') {
            info.sides[sideIdx].conditions.set('reflect', turnNum);
          } else if (mvId === 'lightscreen') {
            info.sides[sideIdx].conditions.set('lightscreen', turnNum);
          } else if (mvId === 'spikes') {
            info.sides[foeIdx].conditions.set('spikes', turnNum);
          } else if (mvId === 'substitute' && !clean.includes('うまくきまらなかった') && !clean.includes('失敗')) {
            mon.volatiles.set('substitute', { key: 'substitute', start_turn: turnNum });
          } else if (mvId === 'perishsong') {
            for (const s of [0, 1]) {
              const a = info.sides[s].activeSlot;
              if (a !== null && info.sides[s].mons[a]) {
                info.sides[s].mons[a].volatiles.set('perishsong', {
                  key: 'perishsong',
                  start_turn: turnNum,
                  counter: 3,
                });
              }
            }
          }
        }
      }
    }

    // Item reveal cues in LLOG (`たべのこしで体力を回復`, berries consumed, Focus Band, Quick Claw)
    const itemSlot = findSlot(sideIdx, clean);
    if (itemSlot !== null && info.sides[sideIdx].mons[itemSlot]) {
      const mon = info.sides[sideIdx].mons[itemSlot];
      if (clean.includes('たべのこし') || clean.includes('食べ残し')) {
        mon.itemOriginal = mon.itemOriginal || 'leftovers';
        mon.itemCurrent = 'leftovers';
      } else if (clean.includes('きあいのハチマキ')) {
        mon.itemOriginal = mon.itemOriginal || 'focusband';
        mon.itemCurrent = 'focusband';
      } else if (clean.includes('せんせいのツメ')) {
        mon.itemOriginal = mon.itemOriginal || 'quickclaw';
        mon.itemCurrent = 'quickclaw';
      } else {
        for (const [jpBerry, berryId] of CONSUMED_BERRIES_JP) {
          if (clean.includes(jpBerry)) {
            mon.itemOriginal = mon.itemOriginal || berryId;
            mon.itemCurrent = '';
            break;
          }
        }
      }
    }

    // Parse stat comments `<!--actorSide targetSide slot statIdx delta ...//-->` embedded by PS.cgi
    for (const cm of block.matchAll(/<!--([\d\s\-]+)\/\/-->/g)) {
      const nums = cm[1].trim().split(/\s+/).map(Number).filter(n => !Number.isNaN(n));
      if (nums.length >= 5 && (nums[1] === 0 || nums[1] === 1) && nums[3] >= 0 && nums[3] <= 6) {
        const targetSide = nums[1];
        const statIdx = nums[3];
        const delta = nums[4];
        const aSlot = info.sides[targetSide].activeSlot;
        if (aSlot !== null && info.sides[targetSide].mons[aSlot] && delta !== 0 && Math.abs(delta) <= 6) {
          const mon = info.sides[targetSide].mons[aSlot];
          mon.boosts[statIdx] = Math.max(-6, Math.min(6, mon.boosts[statIdx] + delta));
        }
      } else {
        const aSlot = info.sides[sideIdx].activeSlot;
        if (aSlot !== null && info.sides[sideIdx].mons[aSlot] && (clean.includes('あがった') || clean.includes('さがった'))) {
          const mon = info.sides[sideIdx].mons[aSlot];
          const statNames = [
            ['こうげき', 0],
            ['ぼうぎょ', 1],
            ['とくこう', 2],
            ['とくぼう', 3],
            ['すばやさ', 4],
            ['めいちゅう', 5],
            ['かいひ', 6],
          ];
          const delta = (clean.includes('ぐーんとあがった') || clean.includes('ぐ〜んとあがった')) ? 2
            : clean.includes('あがった') ? 1
            : (clean.includes('がくっとさがった') || clean.includes('がく〜んとさがった')) ? -2
            : -1;
          for (const [jpStat, idx] of statNames) {
            if (clean.includes(jpStat)) {
              mon.boosts[idx] = Math.max(-6, Math.min(6, mon.boosts[idx] + delta));
            }
          }
        }
      }
    }

    // Faint detection
    if (clean.includes('たおれた') || clean.includes('倒れた')) {
      const slot = findSlot(sideIdx, clean);
      if (slot !== null && info.sides[sideIdx].mons[slot]) {
        info.sides[sideIdx].mons[slot].appeared = true;
        info.sides[sideIdx].mons[slot].fainted = true;
        info.sides[sideIdx].mons[slot].status = '';
        info.sides[sideIdx].mons[slot].boosts = [0, 0, 0, 0, 0, 0, 0];
        info.sides[sideIdx].mons[slot].volatiles.clear();
      }
    }

    // Confusion / Leech Seed / Substitute break cues
    const activeSlot = info.sides[sideIdx].activeSlot;
    if (activeSlot !== null && info.sides[sideIdx].mons[activeSlot]) {
      const mon = info.sides[sideIdx].mons[activeSlot];
      if (clean.includes('こんらんした')) {
        mon.volatiles.set('confusion', { key: 'confusion', start_turn: turnNum });
      } else if (clean.includes('こんらんがとけた') || clean.includes('こんらんがなおった')) {
        mon.volatiles.delete('confusion');
      }
      if (clean.includes('みがわりが消えた') || clean.includes('みがわりは消えてしまった')) {
        mon.volatiles.delete('substitute');
      }
    }

    // Weather end cues
    if (clean.includes('雨はやんだ') || clean.includes('日差しが戻った') || clean.includes('砂嵐はおさまった')) {
      info.weather = null;
    }
  }

  return info;
}

// ------------------------------------------------------------------ Opponent Memory DB & Dynamic Belief Adaptation
function computeShellKey(previewRoster) {
  if (!Array.isArray(previewRoster) || previewRoster.length === 0) return '';
  return previewRoster
    .map(m => `${toId(m.speciesId || m.species)}:${m.level || 50}`)
    .sort()
    .join('/');
}

function loadOpponentDb(dbPath = OPPONENT_DB_PATH) {
  if (!dbPath || !fs.existsSync(dbPath)) {
    return { schema: 'pbs-opponent-db-v1', updatedAt: null, players: {}, shells: {} };
  }
  try {
    const parsed = JSON.parse(fs.readFileSync(dbPath, 'utf8'));
    parsed.players = parsed.players || {};
    parsed.shells = parsed.shells || {};
    return parsed;
  } catch {
    return { schema: 'pbs-opponent-db-v1', updatedAt: null, players: {}, shells: {} };
  }
}

function saveOpponentDb(db, dbPath = OPPONENT_DB_PATH) {
  if (!dbPath || !db) return;
  try {
    db.updatedAt = new Date().toISOString();
    fs.mkdirSync(path.dirname(dbPath), { recursive: true });
    fs.writeFileSync(dbPath, JSON.stringify(db, null, 2) + '\n', { mode: 0o600 });
  } catch {
    // Non-fatal persistence
  }
}

function appendMatchLog(entry, logPath = MATCH_LOG_PATH) {
  if (!logPath) return;
  try {
    fs.mkdirSync(path.dirname(logPath), { recursive: true });
    fs.appendFileSync(logPath, JSON.stringify(entry) + '\n', { mode: 0o600 });
  } catch {
    // Non-fatal logging
  }
}

/**
 * Record observed opponent facts from a match into `opponentDb` so subsequent games
 * (and prior expansion offline) immediately adapt to the opponent's sets and habits.
 */
function recordOpponentObservation({
  opponentDb,
  foeName,
  foePreviewMons,
  seenFoeMons,
  llogFoeMons,
  outcome,
}) {
  if (!opponentDb || !Array.isArray(foePreviewMons) || foePreviewMons.length !== 6) return;
  const shellKey = computeShellKey(foePreviewMons);
  if (!shellKey) return;

  const updateShellBucket = bucket => {
    bucket.games = (bucket.games || 0) + 1;
    bucket.lastSeen = new Date().toISOString();
    bucket.mons = bucket.mons || {};

    for (let i = 0; i < 6; i++) {
      const pm = foePreviewMons[i];
      const spId = toId(pm.speciesId || pm.species);
      const monKey = `${spId}:${pm.level || 50}`;
      const rec = bucket.mons[monKey] || {
        species: dexData.species[spId]?.name || pm.species,
        speciesId: spId,
        level: pm.level || 50,
        gender: pm.gender || 'M',
        itemFlag: Boolean(pm.item_flag),
        pickedCount: 0,
        maxHps: {},
        items: {},
        noLeftoversCount: 0,
        moves: {},
      };

      const seen = seenFoeMons && seenFoeMons[i] ? seenFoeMons[i] : null;
      const llogMon = llogFoeMons && llogFoeMons[i] ? llogFoeMons[i] : null;
      const appeared = Boolean(seen?.appeared || llogMon?.appeared);
      if (appeared) rec.pickedCount = (rec.pickedCount || 0) + 1;
      if (seen?.maxHp && seen.maxHp > 0) {
        rec.maxHps[String(seen.maxHp)] = (rec.maxHps[String(seen.maxHp)] || 0) + 1;
      }
      const itemOrig = seen?.itemOriginal || llogMon?.itemOriginal || null;
      if (itemOrig) {
        rec.items[itemOrig] = (rec.items[itemOrig] || 0) + 1;
      } else if (seen?.noLeftovers) {
        rec.noLeftoversCount = (rec.noLeftoversCount || 0) + 1;
      }

      const moveSet = new Set([
        ...(seen?.revealedMoves || []),
        ...(llogMon?.revealedMoves ? [...llogMon.revealedMoves] : []),
      ]);
      for (const mvId of moveSet) {
        if (dexData.moves[mvId]) {
          rec.moves[mvId] = (rec.moves[mvId] || 0) + 1;
        }
      }

      bucket.mons[monKey] = rec;
    }
  };

  if (foeName) {
    opponentDb.players[foeName] = opponentDb.players[foeName] || {
      name: foeName,
      games: 0,
      winsAgainst: 0,
      lossesAgainst: 0,
      shells: {},
    };
    const p = opponentDb.players[foeName];
    p.games = (p.games || 0) + 1;
    if (outcome === 'WIN') p.winsAgainst = (p.winsAgainst || 0) + 1;
    if (outcome === 'LOSE') p.lossesAgainst = (p.lossesAgainst || 0) + 1;
    p.shells[shellKey] = p.shells[shellKey] || {};
    updateShellBucket(p.shells[shellKey]);
  }

  opponentDb.shells[shellKey] = opponentDb.shells[shellKey] || {};
  updateShellBucket(opponentDb.shells[shellKey]);
}

const ITEM_DISPLAY_BY_ID = {
  leftovers: 'Leftovers',
  mintberry: 'Mint Berry',
  miracleberry: 'Miracle Berry',
  goldberry: 'Gold Berry',
  przcureberry: 'PRZ Cure Berry',
  iceberry: 'Ice Berry',
  burntberry: 'Burnt Berry',
  psncureberry: 'PSN Cure Berry',
  bitterberry: 'Bitter Berry',
  mysteryberry: 'Mystery Berry',
  berryjuice: 'Berry Juice',
  berry: 'Berry',
  thickclub: 'Thick Club',
  lightball: 'Light Ball',
  metalpowder: 'Metal Powder',
  stick: 'Stick',
  scopelens: 'Scope Lens',
  focusband: 'Focus Band',
  quickclaw: 'Quick Claw',
  kingsrock: 'King\'s Rock',
  brightpowder: 'BrightPowder',
  charcoal: 'Charcoal',
  mysticwater: 'Mystic Water',
  magnet: 'Magnet',
  miracleseed: 'Miracle Seed',
  nevermeltice: 'NeverMeltIce',
  twistedspoon: 'TwistedSpoon',
  pinkbow: 'Pink Bow',
  polkadotbow: 'Polkadot Bow',
  softsand: 'Soft Sand',
  hardstone: 'Hard Stone',
  sharpbeak: 'Sharp Beak',
  poisonbarb: 'Poison Barb',
  silverpowder: 'SilverPowder',
  spelltag: 'Spell Tag',
  dragonfang: 'Dragon Fang',
  blackglasses: 'BlackGlasses',
  blackbelt: 'Black Belt',
  metalcoat: 'Metal Coat',
};

/**
 * Build a per-game adapted `beliefPoolJson` string that combines:
 * 1. Exact `maxHp` and `noLeftovers` leak reweighting on `belief-pool-v3` candidates.
 * 2. Synthesized high-prior team candidate(s) from `opponentDb` (past matches against
 *    this player/shell) + current-match observations, completing any unrevealed move slots
 *    from the closest `belief-pool-v3` template for each species.
 */
function buildDynamicBeliefPoolJson({
  foePreviewMons,
  foeName,
  seenFoeMons,
  opponentDb,
}) {
  if (!Array.isArray(foePreviewMons) || foePreviewMons.length !== 6) {
    return { json: beliefPoolJson, hasCustomShellMemory: false };
  }

  const shellKey = computeShellKey(foePreviewMons);
  const playerShell = foeName && opponentDb?.players?.[foeName]?.shells?.[shellKey]
    ? opponentDb.players[foeName].shells[shellKey]
    : null;
  const globalShell = opponentDb?.shells?.[shellKey] || null;
  const shellMem = playerShell || globalShell;
  const hasCustomShellMemory = Boolean(shellMem && shellMem.games > 0);

  // Check whether we have any live leaks (`maxHp` or `noLeftovers`) or historical shell memory
  const hasLiveLeaks = Array.isArray(seenFoeMons) && seenFoeMons.some(
    m => m && ((m.maxHp && m.maxHp > 0) || m.noLeftovers || m.itemOriginal)
  );
  if (!hasCustomShellMemory && !hasLiveLeaks) {
    return { json: beliefPoolJson, hasCustomShellMemory: false };
  }

  const clonedTeams = [];

  for (const team of baseBeliefPool.teams || []) {
    const newSets = [];
    let multiplier = 1.0;

    for (const s of team.sets || []) {
      const spId = toId(s.species);
      const foeSlot = foePreviewMons.findIndex(pm => toId(pm.speciesId || pm.species) === spId);
      if (foeSlot < 0) {
        newSets.push(s);
        continue;
      }

      const pm = foePreviewMons[foeSlot];
      const seen = seenFoeMons && seenFoeMons[foeSlot] ? seenFoeMons[foeSlot] : null;
      const monKey = `${spId}:${pm.level || s.level || 50}`;
      const memMon = shellMem?.mons?.[monKey] || null;

      // 1. Check exact maxHp leak (live first, then historical most-frequent maxHp)
      let targetMaxHp = seen?.maxHp || null;
      if (!targetMaxHp && memMon && memMon.maxHps) {
        const sortedHps = Object.entries(memMon.maxHps).sort((a, b) => b[1] - a[1]);
        if (sortedHps.length > 0) targetMaxHp = parseInt(sortedHps[0][0], 10);
      }
      if (targetMaxHp && targetMaxHp > 0) {
        const candMaxHp = calcSetMaxHp(s, pm.level || s.level || 50);
        if (candMaxHp !== null && candMaxHp !== targetMaxHp) {
          multiplier *= 0.03;
        }
      }

      // 2. Check Leftovers absence (`noLeftovers`)
      const noLeftovers = Boolean(
        seen?.noLeftovers ||
        (!seen?.itemOriginal && memMon && memMon.noLeftoversCount > 0 && !memMon.items?.leftovers)
      );
      if (noLeftovers && toId(s.item || '') === 'leftovers') {
        multiplier *= 0.02;
      }

      newSets.push(s);
    }

    clonedTeams.push({
      ...team,
      sets: newSets,
      weight: Math.max(1e-6, (team.weight ?? 1.0) * multiplier),
    });
  }

  // Synthesize adapted team(s) for this opponent's exact 6-mon shell when we have memory or leaks
  if (hasCustomShellMemory || hasLiveLeaks) {
    const synthSets = [];
    for (let i = 0; i < 6; i++) {
      const pm = foePreviewMons[i];
      const spId = toId(pm.speciesId || pm.species);
      const level = pm.level || 50;
      const seen = seenFoeMons && seenFoeMons[i] ? seenFoeMons[i] : null;
      const monKey = `${spId}:${level}`;
      const memMon = shellMem?.mons?.[monKey] || null;

      // Gather known moves (live + historical top moves)
      const knownMoveIds = [];
      if (seen?.revealedMoves) {
        for (const mId of seen.revealedMoves) {
          if (!knownMoveIds.includes(mId)) knownMoveIds.push(mId);
        }
      }
      if (memMon?.moves) {
        const sortedMoves = Object.entries(memMon.moves)
          .sort((a, b) => b[1] - a[1])
          .map(([k]) => k);
        for (const mId of sortedMoves) {
          if (!knownMoveIds.includes(mId) && knownMoveIds.length < 4) {
            knownMoveIds.push(mId);
          }
        }
      }

      // Gather known item or noLeftovers constraint
      let knownItemId = seen?.itemOriginal || null;
      if (!knownItemId && memMon?.items) {
        const sortedItems = Object.entries(memMon.items).sort((a, b) => b[1] - a[1]);
        if (sortedItems.length > 0) knownItemId = sortedItems[0][0];
      }
      const noLeftovers = Boolean(
        seen?.noLeftovers ||
        (!knownItemId && memMon && memMon.noLeftoversCount > 0)
      );

      // Gather target exact maxHp
      let targetMaxHp = seen?.maxHp || null;
      if (!targetMaxHp && memMon?.maxHps) {
        const sortedHps = Object.entries(memMon.maxHps).sort((a, b) => b[1] - a[1]);
        if (sortedHps.length > 0) targetMaxHp = parseInt(sortedHps[0][0], 10);
      }

      // Find the best-matching template set for this species in baseBeliefPool
      let bestTemplate = null;
      let bestScore = -1e9;
      for (const t of baseBeliefPool.teams || []) {
        for (const cand of t.sets || []) {
          if (toId(cand.species) !== spId) continue;
          let score = Math.log(Math.max(1e-4, t.weight ?? 1.0));
          if ((cand.level || 50) === level) score += 4;
          const candMoves = (cand.moves || []).map(m => {
            const id = toId(m);
            return id.startsWith('hiddenpower') ? 'hiddenpower' : id;
          });
          for (const km of knownMoveIds) {
            const baseKm = km.startsWith('hiddenpower') ? 'hiddenpower' : km;
            if (candMoves.includes(baseKm)) score += 10;
          }
          const candItem = toId(cand.item || '');
          if (knownItemId && candItem === knownItemId) score += 8;
          if (noLeftovers && candItem === 'leftovers') score -= 12;
          if (targetMaxHp && calcSetMaxHp(cand, level) === targetMaxHp) score += 8;
          if (score > bestScore) {
            bestScore = score;
            bestTemplate = cand;
          }
        }
      }

      const baseMoves = bestTemplate
        ? [...bestTemplate.moves]
        : ['Return', 'Rest', 'Sleep Talk', 'Curse'];
      const mergedMoves = [];
      for (const km of knownMoveIds) {
        if (km === 'hiddenpower') {
          const tmplHp = baseMoves.find(m => toId(m).startsWith('hiddenpower'));
          mergedMoves.push(tmplHp || 'Hidden Power Ice');
        } else {
          mergedMoves.push(dexData.moves[km]?.name || km);
        }
      }
      for (const bm of baseMoves) {
        if (mergedMoves.length >= 4) break;
        const bmId = toId(bm);
        const bmBase = bmId.startsWith('hiddenpower') ? 'hiddenpower' : bmId;
        const already = mergedMoves.some(existing => {
          const exId = toId(existing);
          const exBase = exId.startsWith('hiddenpower') ? 'hiddenpower' : exId;
          return exBase === bmBase;
        });
        if (!already) mergedMoves.push(bm);
      }

      let finalItem = bestTemplate?.item || 'Leftovers';
      if (!pm.item_flag) {
        finalItem = '';
      } else if (knownItemId && ITEM_DISPLAY_BY_ID[knownItemId]) {
        finalItem = ITEM_DISPLAY_BY_ID[knownItemId];
      } else if (noLeftovers && toId(finalItem) === 'leftovers') {
        finalItem = spId === 'marowak' ? 'Thick Club' : 'Mint Berry';
      }

      const speciesName = dexData.species[spId]?.name || pm.species;
      const synthSet = {
        name: speciesName,
        species: speciesName,
        level,
        item: finalItem,
        ability: 'No Ability',
        moves: mergedMoves.slice(0, 4),
        gender: pm.gender || bestTemplate?.gender || 'M',
        happiness: 255,
        evs: bestTemplate?.evs
          ? { ...bestTemplate.evs }
          : { hp: 255, atk: 255, def: 255, spa: 255, spd: 255, spe: 255 },
        ivs: bestTemplate?.ivs
          ? { ...bestTemplate.ivs }
          : { hp: 31, atk: 31, def: 31, spa: 31, spd: 31, spe: 31 },
      };

      // If targetMaxHp differs from default maxHp, set `ivs.hp` so `PokemonSet` maxhp matches `targetMaxHp`!
      if (targetMaxHp && calcSetMaxHp(synthSet, level) !== targetMaxHp) {
        const sp = dexData.species[spId];
        if (sp && sp.baseStats) {
          for (let hpDv = 0; hpDv <= 15; hpDv++) {
            const mHp = Math.floor(((sp.baseStats.hp + hpDv) * 2 + 63) * level / 100) + level + 10;
            if (mHp === targetMaxHp) {
              synthSet.ivs = {
                ...synthSet.ivs,
                hp: hpDv * 2,
                atk: (hpDv & 8) ? 30 : 28,
                def: (hpDv & 4) ? 30 : 28,
              };
              break;
            }
          }
        }
      }

      synthSets.push(synthSet);
    }

    clonedTeams.push({
      id: `OPPONENT_ADAPTED_${shellKey.slice(0, 24)}`,
      v1Label: 'strong',
      sets: synthSets,
      weight: hasCustomShellMemory ? 6.0 : 2.5,
    });
  }

  return {
    json: JSON.stringify({
      ...baseBeliefPool,
      teams: clonedTeams,
    }),
    hasCustomShellMemory,
  };
}

// ------------------------------------------------------------------ PositionSpec Builder & Sanitizer
/**
 * Build and strictly sanitize an `nc2000-position-v1` object so `PositionSpec::check()`
 * in `crates/bot/src/position.rs` is guaranteed to succeed.
 * Incorporates Castle-specific information leaks via `seenFoeMons`:
 * - Exact bench `hp / maxHp` and `status` for opponent Pokemon that switched out
 * - Item reveals (`item_original`, `item_current`)
 * - Exact `maxHp` -> `hp_type_mask` constraint once `hiddenpower` is revealed
 */
function buildAndSanitizePositionSpec({
  mySide,
  turn,
  ownSets,
  previewSides,
  pickedRosterSlots,
  battleTurn,
  pdat,
  llog,
  seenFoeMons,
  teamPreview,
  forceSwitch,
}) {
  const foeSide = 1 - mySide;
  const spec = {
    schema: 'nc2000-position-v1',
    side: mySide,
    turn: teamPreview ? 0 : Math.max(1, turn || 1),
    upkeep_this_turn: false,
    own_sets: ownSets,
    sides: [
      { mons: [], active: null, conditions: [], party: [] },
      { mons: [], active: null, conditions: [], party: [] },
    ],
    weather: teamPreview ? null : (llog && llog.weather ? llog.weather : null),
    team_preview: Boolean(teamPreview),
    force_switch: Boolean(forceSwitch),
    trapped: false,
  };

  // Populate both sides' 6-mon rosters
  for (const s of [0, 1]) {
    const isMe = s === mySide;
    const previewMons = (previewSides && previewSides[s] && previewSides[s].length === 6)
      ? previewSides[s]
      : ownSets.map(x => ({
          species: x.species,
          speciesId: toId(x.species),
          level: x.level || 55,
          gender: x.gender || 'M',
          item_flag: Boolean(x.item),
        }));

    for (let i = 0; i < 6; i++) {
      const pm = previewMons[i] || {
        species: 'Pikachu',
        speciesId: 'pikachu',
        level: 50,
        gender: 'M',
        item_flag: true,
      };
      const ownSet = isMe ? ownSets[i] : null;
      const llogMon = llog && llog.sides[s] && llog.sides[s].mons[i] ? llog.sides[s].mons[i] : null;
      const seenFoe = !isMe && seenFoeMons && seenFoeMons[i] ? seenFoeMons[i] : null;

      const speciesName = ownSet
        ? ownSet.species
        : (dexData.species[pm.speciesId]?.name || pm.species || 'Pikachu');
      const level = ownSet ? (ownSet.level || 55) : (pm.level || 55);
      const gender = pm.gender || (ownSet?.gender || 'M');
      const itemFlag = isMe ? Boolean(ownSet?.item) : Boolean(pm.item_flag);

      const uses = [];
      const revealedSet = new Set();
      if (llogMon) {
        for (const [mvId, count] of Object.entries(llogMon.uses || {})) {
          if (dexData.moves[mvId]) {
            uses.push({ move: mvId, n: count });
          }
        }
        for (const mvId of llogMon.revealedMoves || []) {
          if (dexData.moves[mvId]) {
            revealedSet.add(mvId);
          }
        }
      }
      if (seenFoe && seenFoe.revealedMoves) {
        for (const mvId of seenFoe.revealedMoves) {
          if (dexData.moves[mvId]) {
            revealedSet.add(mvId);
          }
        }
      }
      const revealedMoves = [...revealedSet];

      const fainted = Boolean(!teamPreview && (llogMon?.fainted || seenFoe?.fainted));
      const appeared = Boolean(!teamPreview && (llogMon?.appeared || seenFoe?.appeared || fainted));

      let hpDen = 100;
      let hpNum = fainted ? 0 : 100;
      let status = !teamPreview && llogMon?.status ? llogMon.status : '';

      // Restore persistent exact HP & status for opponent bench mons from `seenFoeMons`
      if (!teamPreview && !isMe && seenFoe) {
        if (seenFoe.maxHp && seenFoe.maxHp > 0) {
          hpDen = seenFoe.maxHp;
          hpNum = fainted
            ? 0
            : Math.max(1, Math.min(seenFoe.maxHp, seenFoe.hp ?? seenFoe.maxHp));
        }
        if (!fainted && seenFoe.status !== undefined) {
          status = seenFoe.status || status;
        }
      }

      const monEntry = {
        species: speciesName,
        level,
        gender,
        name: '',
        item_flag: itemFlag,
        appeared,
        appear_count: appeared ? 1 : 0,
        switch_in_turn: 0,
        active: false,
        fainted,
        hp_num: hpNum,
        hp_den: hpDen,
        hp_exact: null,
        status: fainted ? '' : status,
        rest: Boolean(!teamPreview && llogMon?.rest),
        slept: 0,
        boosts: !teamPreview && llogMon?.boosts ? [...llogMon.boosts] : [0, 0, 0, 0, 0, 0, 0],
        volatiles: !teamPreview && llogMon?.volatiles ? [...llogMon.volatiles.values()] : [],
        uses,
        revealed_moves: isMe ? [] : revealedMoves,
      };

      // Attach Castle item reveal & Hidden Power DV mask on opponent mons
      if (!teamPreview && !isMe) {
        const itemOrig = seenFoe?.itemOriginal || llogMon?.itemOriginal || null;
        const itemCur = seenFoe?.itemCurrent !== undefined && seenFoe?.itemCurrent !== null
          ? seenFoe.itemCurrent
          : (llogMon?.itemCurrent !== undefined && llogMon?.itemCurrent !== null ? llogMon.itemCurrent : null);
        if (itemOrig && itemFlag) {
          monEntry.item_original = { known: true, item: itemOrig };
          const curStr = itemCur !== null ? itemCur : itemOrig;
          monEntry.item_current = { known: true, item: curStr ? curStr : null };
        }
        // Observer::from_position allows hp_type_mask != 0xFFFF iff revealed_moves has 'hiddenpower'
        if (revealedSet.has('hiddenpower') && seenFoe?.maxHp && seenFoe.maxHp > 0) {
          const hpMask = inferHpTypeMaskFromMaxHp(pm.speciesId || pm.species, level, gender, seenFoe.maxHp);
          if (hpMask !== 0xffff && hpMask !== 0) {
            monEntry.hp_type_mask = hpMask;
          }
        }
      }

      spec.sides[s].mons.push(monEntry);
    }

    if (!teamPreview && llog && llog.sides[s] && llog.sides[s].conditions) {
      for (const [key, startTurn] of llog.sides[s].conditions.entries()) {
        spec.sides[s].conditions.push({ key, start_turn: startTurn });
      }
    }
  }

  if (teamPreview) {
    return sanitizePositionSpec(spec);
  }

  // Apply PDAT authority for our own 3 picked mons
  const myMons = spec.sides[mySide].mons;
  if (pdat && Array.isArray(pdat.mons)) {
    for (let cIdx = 0; cIdx < pdat.mons.length; cIdx++) {
      const pm = pdat.mons[cIdx];
      let rSlot = Array.isArray(pickedRosterSlots) && pickedRosterSlots[cIdx] !== undefined
        ? pickedRosterSlots[cIdx]
        : myMons.findIndex(m => toId(m.species) === pm.speciesId);
      if (rSlot < 0 || rSlot >= myMons.length) continue;
      const target = myMons[rSlot];
      if (pm.hp <= 0 || pm.status === 'fnt') {
        target.fainted = true;
        target.appeared = true;
        target.hp_num = 0;
        target.hp_den = Math.max(1, pm.maxHp || 100);
        target.hp_exact = 0;
        target.status = '';
      } else {
        target.fainted = false;
        target.hp_num = Math.max(1, Math.min(pm.maxHp || 100, pm.hp));
        target.hp_den = Math.max(1, pm.maxHp || 100);
        target.hp_exact = pm.hp;
        target.status = pm.status || '';
      }
      target.item_now = pm.item || '';
    }
  }

  // Apply `<select Name="MOVE">` bench `C1..C3` HP authority
  if (battleTurn && Array.isArray(battleTurn.options)) {
    for (const opt of battleTurn.options) {
      if (opt.kind === 'switch' && opt.hp !== null && opt.maxHp !== null) {
        const cIdx = opt.slotIndex - 1;
        let rSlot = Array.isArray(pickedRosterSlots) && pickedRosterSlots[cIdx] !== undefined
          ? pickedRosterSlots[cIdx]
          : myMons.findIndex(m => toId(m.species) === opt.speciesId);
        if (rSlot >= 0 && rSlot < myMons.length) {
          const target = myMons[rSlot];
          if (opt.hp <= 0) {
            target.fainted = true;
            target.appeared = true;
            target.hp_num = 0;
            target.hp_den = Math.max(1, opt.maxHp);
            target.hp_exact = 0;
            target.status = '';
          } else {
            target.fainted = false;
            target.hp_num = Math.max(1, Math.min(opt.maxHp, opt.hp));
            target.hp_den = Math.max(1, opt.maxHp);
            target.hp_exact = opt.hp;
          }
        }
      }
    }
  }

  // Apply Top Status Table authority for both active mons
  for (const s of [0, 1]) {
    const isMe = s === mySide;
    const sideMons = spec.sides[s].mons;
    const topInfo = battleTurn && battleTurn.sides ? battleTurn.sides[s] : null;
    let activeSlot = null;

    if (topInfo && topInfo.speciesId) {
      activeSlot = sideMons.findIndex(m => toId(m.species) === topInfo.speciesId);
    }
    if ((activeSlot === null || activeSlot < 0) && isMe && !forceSwitch && battleTurn && Array.isArray(battleTurn.options)) {
      const benchSpecies = new Set(
        battleTurn.options.filter(o => o.kind === 'switch' && o.speciesId).map(o => o.speciesId)
      );
      if (Array.isArray(pickedRosterSlots) && pickedRosterSlots.length === 3 && benchSpecies.size > 0) {
        const nonBench = pickedRosterSlots.find(rIdx => !benchSpecies.has(toId(sideMons[rIdx]?.species)));
        if (nonBench !== undefined) activeSlot = nonBench;
      }
    }
    if (activeSlot === null || activeSlot < 0) {
      if (llog && llog.sides[s] && llog.sides[s].activeSlot !== null) {
        activeSlot = llog.sides[s].activeSlot;
      } else if (isMe && Array.isArray(pickedRosterSlots) && pickedRosterSlots.length > 0) {
        activeSlot = pickedRosterSlots[0];
      } else {
        activeSlot = 0;
      }
    }

    for (let i = 0; i < sideMons.length; i++) {
      sideMons[i].active = i === activeSlot;
    }
    spec.sides[s].active = activeSlot;

    const actMon = sideMons[activeSlot];
    if (actMon) {
      actMon.appeared = true;
      actMon.appear_count = Math.max(1, actMon.appear_count || 1);
      if (topInfo && topInfo.hp !== null && topInfo.maxHp !== null && topInfo.maxHp > 0) {
        if (topInfo.hp <= 0) {
          if (forceSwitch) {
            actMon.fainted = true;
            actMon.hp_num = 0;
            actMon.hp_den = topInfo.maxHp;
            if (isMe) actMon.hp_exact = 0;
            actMon.status = '';
          } else {
            // Normal move turn cannot have a fainted active mon
            actMon.fainted = false;
            actMon.hp_num = 1;
            actMon.hp_den = topInfo.maxHp;
            if (isMe) actMon.hp_exact = 1;
          }
        } else {
          actMon.fainted = false;
          actMon.hp_num = Math.max(1, Math.min(topInfo.maxHp, topInfo.hp));
          actMon.hp_den = topInfo.maxHp;
          if (isMe) actMon.hp_exact = topInfo.hp;
          actMon.status = topInfo.status || '';
        }
      }
    }
  }

  // Populate active move PPs from `<select Name="MOVE">` W1..W4 options
  const myActiveSlot = spec.sides[mySide].active;
  if (!forceSwitch && myActiveSlot !== null && myMons[myActiveSlot] && battleTurn) {
    const moveOpts = battleTurn.options.filter(o => o.kind === 'move');
    const ownSet = ownSets[myActiveSlot];
    if (ownSet && Array.isArray(ownSet.moves)) {
      const ppRows = [];
      for (let mIdx = 0; mIdx < ownSet.moves.length; mIdx++) {
        const rawMove = ownSet.moves[mIdx];
        let mId = toId(rawMove);
        const baseMoveId = mId.startsWith('hiddenpower') ? 'hiddenpower' : mId;
        const dexMv = dexData.moves[mId] || dexData.moves[baseMoveId];
        const maxpp = dexMv ? Math.floor(dexMv.pp * 8 / 5) : 16;
        const wOpt = moveOpts.find(
          o => o.slotIndex === mIdx + 1 || o.moveId === baseMoveId || o.moveId === mId
        );
        const curPp = wOpt ? (wOpt.pp ?? maxpp) : 0;
        const disabled = !wOpt || curPp <= 0;
        ppRows.push({
          move: mId,
          pp: Math.max(0, curPp),
          maxpp,
          disabled,
        });
      }
      myMons[myActiveSlot].pp = ppRows;
    }
  }

  // Construct `sides[mySide].party` with active slot first (`party[0]`), followed by the 2 bench picks
  const basePicked = Array.isArray(pickedRosterSlots) && pickedRosterSlots.length === 3
    ? [...pickedRosterSlots]
    : [0, 1, 2];
  if (myActiveSlot !== null && !basePicked.includes(myActiveSlot)) {
    basePicked[0] = myActiveSlot;
  }
  const orderedParty = [];
  if (myActiveSlot !== null) orderedParty.push(myActiveSlot);
  for (const s of basePicked) {
    if (!orderedParty.includes(s)) orderedParty.push(s);
  }
  for (let i = 0; i < myMons.length && orderedParty.length < 3; i++) {
    if (!orderedParty.includes(i)) orderedParty.push(i);
  }
  spec.sides[mySide].party = orderedParty.slice(0, 3);

  return sanitizePositionSpec(spec);
}

function sanitizePositionSpec(spec) {
  for (let s = 0; s < 2; s++) {
    const side = spec.sides[s];
    if (spec.team_preview) {
      side.active = null;
      side.party = [];
      for (const m of side.mons) {
        m.active = false;
        m.fainted = false;
        m.appeared = false;
        m.hp_den = Math.max(1, m.hp_den || 100);
        m.hp_num = m.hp_den;
        m.hp_exact = null;
        m.status = '';
      }
      continue;
    }

    if (side.active === null || side.active < 0 || side.active >= side.mons.length) {
      side.active = 0;
    }
    for (let i = 0; i < side.mons.length; i++) {
      const m = side.mons[i];
      m.active = i === side.active;
      m.hp_den = Math.max(1, m.hp_den || 100);
      if (m.active && !spec.force_switch) {
        m.fainted = false;
      }
      if (m.fainted) {
        m.appeared = true;
        m.appear_count = Math.max(1, m.appear_count || 1);
        m.hp_num = 0;
        if (m.hp_exact !== null && m.hp_exact !== undefined) m.hp_exact = 0;
        m.status = '';
      } else {
        m.hp_num = Math.max(1, Math.min(m.hp_den, m.hp_num || m.hp_den));
        if (m.hp_exact !== null && m.hp_exact !== undefined) {
          m.hp_exact = Math.max(1, m.hp_exact);
        }
        if (m.status === 'fnt') m.status = '';
      }
      if (m.active) {
        m.appeared = true;
        m.appear_count = Math.max(1, m.appear_count || 1);
      }
    }
  }
  return spec;
}

// ------------------------------------------------------------------ Action Projection
/**
 * Map a searcher choice (`move <id>` or `switch <1..3>`) onto a concrete `<option Value="...">`
 * from `PS.cgi`'s `<select Name="MOVE">`.
 */
function mapChoiceToPbsOption(choiceStr, spec, pickedRosterSlots, options, ownSets) {
  if (!choiceStr || !Array.isArray(options) || options.length === 0) return null;
  const parts = String(choiceStr).trim().split(/\s+/);
  const mySide = spec.side;
  const activeRosterSlot = spec.sides[mySide].active ?? 0;
  const partyOrder = spec.sides[mySide].party || pickedRosterSlots || [0, 1, 2];

  if (parts[0] === 'move' && parts[1]) {
    const moveArg = toId(parts[1]);
    const baseMoveArg = moveArg.startsWith('hiddenpower') ? 'hiddenpower' : moveArg;
    const ownSet = ownSets[activeRosterSlot];

    // 1. Match by move slot index in ownSet
    if (ownSet && Array.isArray(ownSet.moves)) {
      const mIdx = ownSet.moves.findIndex(m => {
        const id = toId(m);
        return id === moveArg || (id.startsWith('hiddenpower') && baseMoveArg === 'hiddenpower');
      });
      if (mIdx >= 0) {
        const wantVal = `W${mIdx + 1}`;
        const found = options.find(o => o.value === wantVal && (o.pp === null || o.pp > 0));
        if (found) return found.value;
      }
    }
    // 2. Match by parsed Japanese move ID on the `<option>`
    const byMoveId = options.find(
      o => o.kind === 'move' && (o.moveId === moveArg || o.moveId === baseMoveArg) && (o.pp === null || o.pp > 0)
    );
    if (byMoveId) return byMoveId.value;
  }

  if (parts[0] === 'switch' && parts[1]) {
    const pos = parseInt(parts[1], 10); // 1-indexed into spec.sides[mySide].party
    const targetRosterSlot = partyOrder[pos - 1];
    if (targetRosterSlot !== undefined && ownSets[targetRosterSlot]) {
      const targetSpId = toId(ownSets[targetRosterSlot].species);
      // 1. Match by pickedRosterSlots -> C1..C3
      if (Array.isArray(pickedRosterSlots)) {
        const cIdx = pickedRosterSlots.indexOf(targetRosterSlot);
        if (cIdx >= 0) {
          const wantVal = `C${cIdx + 1}`;
          const found = options.find(o => o.value === wantVal && (o.pp === undefined || o.hp === null || o.hp > 0));
          if (found) return found.value;
        }
      }
      // 2. Match by speciesId on the `<option>`
      const bySpecies = options.find(
        o => o.kind === 'switch' && o.speciesId === targetSpId && (o.hp === null || o.hp > 0)
      );
      if (bySpecies) return bySpecies.value;
    }
  }

  return null;
}

/**
 * Select a guaranteed-valid `<option Value="...">` from `PS.cgi`'s `<select Name="MOVE">`.
 * First tries `bestChoice`, then walks `rootPolicy.actions` in descending visit order,
 * and finally falls back to the first legal move/switch option in `<select Name="MOVE">`.
 */
function selectLegalPbsMoveOption({
  bestChoice,
  rootPolicy,
  spec,
  pickedRosterSlots,
  options,
  ownSets,
  isRandom,
}) {
  if (!Array.isArray(options) || options.length === 0) {
    return 'W1';
  }

  const playableOptions = options.filter(o => {
    if (o.kind === 'move') return o.pp === null || o.pp > 0;
    if (o.kind === 'switch') return o.hp === null || o.hp > 0;
    return true;
  });
  const pool = playableOptions.length > 0 ? playableOptions : options;

  if (isRandom) {
    return pool[rngInt(pool.length)].value;
  }

  const direct = mapChoiceToPbsOption(bestChoice, spec, pickedRosterSlots, pool, ownSets);
  if (direct) return direct;

  if (rootPolicy && Array.isArray(rootPolicy.actions)) {
    for (const act of rootPolicy.actions) {
      const mapped = mapChoiceToPbsOption(act.input, spec, pickedRosterSlots, pool, ownSets);
      if (mapped) return mapped;
    }
  }

  return pool[0].value;
}

/**
 * Choose a legal 3-mon pick (`[leadSlot, benchSlotA, benchSlotB]`, 0-indexed into `ownSets`)
 * respecting the NC2000 total level cap (155).
 */
function parseOrFallbackPreviewPick(choiceStr, ownSets, isRandom) {
  const levels = ownSets.map(s => s.level || 55);
  const allValidTriples = [];
  for (let a = 0; a < ownSets.length; a++) {
    for (let b = 0; b < ownSets.length; b++) {
      if (b === a) continue;
      for (let c = b + 1; c < ownSets.length; c++) {
        if (c === a) continue;
        if ((levels[a] + levels[b] + levels[c]) <= 155) {
          allValidTriples.push([a, b, c]);
        }
      }
    }
  }

  if (!isRandom && choiceStr) {
    const m = String(choiceStr).match(/team\s+(\d)\s*,?\s*(\d)\s*,?\s*(\d)/i);
    if (m) {
      const a = parseInt(m[1], 10) - 1;
      const b = parseInt(m[2], 10) - 1;
      const c = parseInt(m[3], 10) - 1;
      const bench = [b, c].sort((x, y) => x - y);
      if (
        a >= 0 && a < 6 &&
        bench[0] >= 0 && bench[0] < 6 &&
        bench[1] >= 0 && bench[1] < 6 &&
        a !== bench[0] && a !== bench[1] && bench[0] !== bench[1] &&
        (levels[a] + levels[bench[0]] + levels[bench[1]]) <= 155
      ) {
        return [a, bench[0], bench[1]];
      }
    }
  }

  if (allValidTriples.length > 0) {
    return allValidTriples[rngInt(allValidTriples.length)];
  }
  return [0, 1, 2];
}

// ------------------------------------------------------------------ WASM Initialization
let wasm = null;
let wasmDex = null;
if (!RANDOM) {
  wasm = require(path.join(REPO, 'crates/wasm/pkg-node/nc2000_wasm.js'));
  wasmDex = new wasm.Dex();
}

function log(msg) {
  if (!QUIET) {
    console.log(`[${NAME}] ${msg}`);
  }
}

function appendDecisionLog(entry) {
  if (!DECISION_LOG) return;
  try {
    fs.mkdirSync(path.dirname(DECISION_LOG), { recursive: true });
    fs.appendFileSync(DECISION_LOG, JSON.stringify(entry) + '\n', { mode: 0o600 });
  } catch {
    // Non-fatal logging
  }
}


/**
 * Resolve the PBS result from the explicit winner name.
 */
function escapeRegexLiteral(s) {
  return String(s).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function resolveBotOutcomeFromFinishedHtml(
  html,
  botName,
  foeName
) {
  const text = String(html || '')
    .replace(/<[^>]*>/g, ' ')
    .normalize('NFKC')
    .replace(/\s+/g, ' ')
    .trim();

  if (/(?:^|\s)DRAW(?:!|\s|$)/u.test(text)) return 'DRAW';

  const wins = name => {
    const n = String(name || '')
      .normalize('NFKC')
      .replace(/\s+/g, ' ')
      .trim();

    if (!n) return false;

    return new RegExp(
      '(?:^|\\s)' +
        escapeRegexLiteral(n) +
        '\\s+WIN!(?=\\s|$)',
      'u'
    ).test(text);
  };

  const botWin = wins(botName);
  const foeWin = wins(foeName);

  if (botWin === foeWin) return 'END';
  return botWin ? 'WIN' : 'LOSE';
}
// ------------------------------------------------------------------ Single Game Runner
async function playOneGame(gameIdx) {
  const client = new PbsHttpClient();
  const team = pickTeam();
  const encodedPd = encodeTeamForPbsCookie(team);
  client.setCookie('PD0', encodedPd.cookieValue);
  client.setCookie('PBS', `${NAME}_${PASS}`);

  const opponentDb = loadOpponentDb(OPPONENT_DB_PATH);
  log(`Game #${gameIdx + 1}: selected team ${team.label} (iters=${ITERS}, c=${PROFILE.c})`);

  let mySide = null; // 0 = P1, 1 = P2
  let foeName = null;
  let previewSides = null;
  let pickedRosterSlots = null; // [leadIdx, bench1Idx, bench2Idx] (0-indexed)
  let seenFoeMons = null; // Persistent per-slot opponent state across turns
  let lastFoeActiveSlot = null;
  let lastLlogFoeMons = null;
  let searcher = null;
  let searcherBeliefSignature = '';
  let hasCustomShellMemory = false;
  let decisions = 0;
  let enteredRoom = false;
  let hostWaitStartedAt = null;
  let pdRegistered = false;
  let selectSubmitted = false;
  let lastTurnActedKey = '';
  const matchStartedAt = new Date().toISOString();

  const ensureSearcher = (side, forceRebuild = false) => {
    if (RANDOM) return null;
    const foeSide = 1 - side;
    const foePreviewMons = previewSides && previewSides[foeSide] ? previewSides[foeSide] : null;
    const sig = JSON.stringify({
      side,
      foeName,
      shell: computeShellKey(foePreviewMons),
      leaks: (seenFoeMons || []).map(m => ({
        maxHp: m?.maxHp || null,
        noLeftovers: Boolean(m?.noLeftovers),
        itemOriginal: m?.itemOriginal || null,
      })),
    });

    if (!searcher || mySide !== side || forceRebuild || sig !== searcherBeliefSignature) {
      if (searcher && typeof searcher.free === 'function') {
        try { searcher.free(); } catch {}
      }
      const adapted = buildDynamicBeliefPoolJson({
        foePreviewMons,
        foeName,
        seenFoeMons,
        opponentDb,
      });
      hasCustomShellMemory = adapted.hasCustomShellMemory;
      searcherBeliefSignature = sig;
      searcher = new wasm.ProtocolSearcher(
        wasmDex,
        side,
        adapted.json,
        SEED * 1000 + gameIdx * 10 + side,
        PROFILE.c
      );
    }
    return searcher;
  };

  const finalizeMatchRecord = outcome => {
    if (previewSides && mySide !== null) {
      const foeSide = 1 - mySide;
      const foePreviewMons = previewSides[foeSide];
      recordOpponentObservation({
        opponentDb,
        foeName,
        foePreviewMons,
        seenFoeMons,
        llogFoeMons: lastLlogFoeMons,
        outcome,
      });
      saveOpponentDb(opponentDb, OPPONENT_DB_PATH);
      appendMatchLog({
        startedAt: matchStartedAt,
        finishedAt: new Date().toISOString(),
        roomUrl: ROOM_URL,
        botName: NAME,
        foeName: foeName || null,
        mySide,
        tier: TIER,
        teamId: team.id,
        teamLabel: team.label,
        iters: ITERS,
        outcome,
        decisions,
        pickedRosterSlots,
        foeRoster: foePreviewMons,
        seenFoeMons,
      });
    }
  };

  try {
    let res = await client.get(ROOM_URL);

    while (true) {
      const page = classifyPage(res);

      if (page.kind === 'FINISHED') {
        if (!enteredRoom && page.reason === 'match_end_page') {
          res = await client.get(`${ROOM_URL}?ACTION=TITLE`);
          continue;
        }
        let resultHtml = page.html || '';
        try {
          const resultLog =
            await client.get(`${ROOM_URL}?ACTION=LLOG`);
          if (resultLog && resultLog.html) {
            resultHtml += '\n' + resultLog.html;
          }
        } catch {}

        const outcome =
          resolveBotOutcomeFromFinishedHtml(
            resultHtml,
            NAME,
            foeName
          );
        log(`Game #${gameIdx + 1} finished (${outcome}) after ${decisions} decisions.`);
        finalizeMatchRecord(outcome);
        // Touch PS.cgi?ACTION=END if link is present so the room cleans up promptly
        if (/PS\.cgi\?ACTION=END/i.test(page.html || '')) {
          try { await client.get(`${ROOM_URL}?ACTION=END`); } catch {}
        }
        return { outcome, decisions, team: team.id, foeName };
      }

      if (page.kind === 'REDIRECT_OTHER') {
        res = await client.get(page.location);
        continue;
      }

      if (page.kind === 'ERROR') {
        log(`PBS returned ERROR page: ${page.errorText}`);
        await sleep(POLL_MS);
        res = await client.get(ROOM_URL);
        continue;
      }

      if (page.kind === 'ENTORY') {
        // If we had already entered and the room returned to ENTORY, the game ended
        if (enteredRoom && decisions > 0) {
          log(`Game #${gameIdx + 1} finished (room reset to ENTORY) after ${decisions} decisions.`);
          finalizeMatchRecord('END');
          return { outcome: 'END', decisions, team: team.id, foeName };
        }

        if (page.p1Waiting) {
          if (MODE === 'host') {
            log(`P1 (${page.p1Name}) is already in room, waiting (--mode host)...`);
            await sleep(POLL_MS);
            res = await client.get(ROOM_URL);
            continue;
          }
          foeName = page.p1Name || foeName;
          log(`Joining against waiting P1 "${page.p1Name}" as P2...`);
          mySide = 1;
          enteredRoom = true;
          res = await client.postForm(ROOM_URL, {
            ACTION: 'ENTORY',
            NAME,
            PASS,
          });
          continue;
        } else {
          if (MODE === 'join') {
            if (!hostWaitStartedAt) hostWaitStartedAt = Date.now();
            if (HOST_TIMEOUT_SEC > 0 && (Date.now() - hostWaitStartedAt) > HOST_TIMEOUT_SEC * 1000) {
              log(`Join wait timeout (${HOST_TIMEOUT_SEC}s) reached with no P1 in room.`);
              return { outcome: 'TIMEOUT', decisions: 0, team: team.id, foeName: null };
            }
            log(`Waiting for a P1 player to enter room (--mode join)...`);
            await sleep(POLL_MS);
            res = await client.get(ROOM_URL);
            continue;
          }
          log(`Entering empty room as P1 (host)...`);
          mySide = 0;
          enteredRoom = true;
          hostWaitStartedAt = Date.now();
          res = await client.postForm(ROOM_URL, {
            ACTION: 'ENTORY',
            NAME,
            PASS,
            MSG,
            L_TURN: String(LTURN),
            TLIMIT: String(TLIMIT),
          });
          continue;
        }
      }

      if (page.kind === 'WAITING_ENTRY') {
        if (!hostWaitStartedAt) hostWaitStartedAt = Date.now();
        if (HOST_TIMEOUT_SEC > 0 && (Date.now() - hostWaitStartedAt) > HOST_TIMEOUT_SEC * 1000) {
          log(`Host wait timeout (${HOST_TIMEOUT_SEC}s) reached with no challenger.`);
          return { outcome: 'TIMEOUT', decisions: 0, team: team.id, foeName: null };
        }
        await sleep(POLL_MS);
        res = await client.get(ROOM_URL);
        continue;
      }

      if (page.kind === 'REDIRECT_PDINPUT') {
        if (!pdRegistered) {
          log(`Registering 6-mon party via PDINPUT.cgi (sim=${page.sim})...`);
        }
        pdRegistered = true;
        // Submit 1-POST party registration (`決定` = %B7%E8%C4%EA in EUC-JP)
        const formBody =
          `ACTION=RE&CBAN=1&PD=&sim=${encodeURIComponent(page.sim)}&CH=on&submit=%B7%E8%C4%EA`;
        const pdRes = await client.postForm('http://psense.lib.net/_/PDINPUT.cgi', formBody);
        if (pdRes.location) {
          res = await client.get(new URL(pdRes.location, ROOM_URL).toString());
        } else {
          res = await client.get(ROOM_URL);
        }
        continue;
      }

      if (page.kind === 'TEAM_PREVIEW_CHECK') {
        const parsedPreview = parseTeamPreviewHtml(page.html);
        // Determine whether we are P1 (0) or P2 (1)
        if (parsedPreview.p1Name === NAME && parsedPreview.p2Name !== NAME) {
          mySide = 0;
          foeName = parsedPreview.p2Name || foeName;
        } else if (parsedPreview.p2Name === NAME && parsedPreview.p1Name !== NAME) {
          mySide = 1;
          foeName = parsedPreview.p1Name || foeName;
        } else if (mySide === null) {
          // Match by own species
          const ownSpIds = team.sets.map(s => toId(s.species)).join(',');
          const p1SpIds = parsedPreview.p1Mons.map(m => m.speciesId).join(',');
          mySide = p1SpIds === ownSpIds ? 0 : 1;
          foeName = mySide === 0 ? parsedPreview.p2Name : parsedPreview.p1Name;
        } else {
          foeName = (mySide === 0 ? parsedPreview.p2Name : parsedPreview.p1Name) || foeName;
        }

        previewSides = [parsedPreview.p1Mons, parsedPreview.p2Mons];
        const foeSide = 1 - mySide;
        seenFoeMons = previewSides[foeSide].map(pm => ({
          species: pm.species,
          speciesId: pm.speciesId,
          level: pm.level || 50,
          gender: pm.gender || 'M',
          itemFlag: Boolean(pm.item_flag),
          appeared: false,
          fainted: false,
          hp: null,
          maxHp: null,
          status: '',
          itemOriginal: null,
          itemCurrent: null,
          noLeftovers: false,
          damagedTurnsCount: 0,
          revealedMoves: [],
        }));

        log(
          `Team Preview (side=P${mySide + 1}, foe="${foeName || '?'}"): Foe roster = ` +
          previewSides[foeSide].map(m => `${m.species}(L${m.level})`).join(', ')
        );

        let choiceStr = null;
        let previewSource = 'fallback';
        const spec = buildAndSanitizePositionSpec({
          mySide,
          turn: 0,
          ownSets: team.sets,
          previewSides,
          pickedRosterSlots: null,
          battleTurn: null,
          pdat: null,
          llog: null,
          seenFoeMons,
          teamPreview: true,
          forceSwitch: false,
        });

        if (!RANDOM) {
          try {
            const s = ensureSearcher(mySide, true);
            s.setPosition(JSON.stringify(spec));
            // If we have not seen this opponent's shell before, check compiled preview book first
            if (!hasCustomShellMemory && compiledPreview) {
              try {
                const cpKey = s.compiledPreviewKey();
                if (cpKey && compiledPreview[cpKey] && compiledPreview[cpKey].choice) {
                  choiceStr = compiledPreview[cpKey].choice;
                  previewSource = 'compiled-book';
                }
              } catch {}
            }
            if (!choiceStr && !hasCustomShellMemory) {
              choiceStr = s.bakedPreview();
              if (choiceStr) previewSource = 'baked';
            }
            if (!choiceStr) {
              s.step(ITERS);
              choiceStr = s.best();
              previewSource = hasCustomShellMemory ? 'adapted-search' : 'search';
            }
          } catch (err) {
            log(`Preview search warning (using fallback pick): ${err.message || err}`);
          }
        }

        pickedRosterSlots = parseOrFallbackPreviewPick(choiceStr, team.sets, RANDOM);
        const [leadIdx, b1Idx, b2Idx] = pickedRosterSlots;
        log(
          `Preview pick: Lead=#${leadIdx + 1} (${team.sets[leadIdx].species}), ` +
          `Bench=#${b1Idx + 1} (${team.sets[b1Idx].species}), #${b2Idx + 1} (${team.sets[b2Idx].species})` +
          (choiceStr ? ` [${previewSource}: ${choiceStr}]` : '')
        );

        decisions++;
        appendDecisionLog({
          gameIdx,
          phase: 'preview',
          side: mySide,
          foeName,
          previewSource,
          choiceStr,
          pickedRosterSlots,
        });

        const checkFields = {
          ACTION: 'CHECK',
          SENTE: String(leadIdx + 1),
          [`P${b1Idx + 1}`]: '1',
          [`P${b2Idx + 1}`]: '1',
        };
        res = await client.postForm(ROOM_URL, checkFields);
        continue;
      }

      if (page.kind === 'TEAM_PREVIEW_SELECT') {
        const [leadIdx, b1Idx, b2Idx] = pickedRosterSlots || [0, 1, 2];
        selectSubmitted = true;
        const selectFields = {
          ACTION: 'SELECT',
          SENTE: String(leadIdx + 1),
          [`P${b1Idx + 1}`]: '1',
          [`P${b2Idx + 1}`]: '1',
        };
        res = await client.postForm(ROOM_URL, selectFields);
        continue;
      }

      if (page.kind === 'WAITING_SELECT' || page.kind === 'WAITING_TURN') {
        await sleep(POLL_MS);
        res = await client.get(ROOM_URL);
        continue;
      }

      if (page.kind === 'SELECT_DONE') {
        res = await client.get(ROOM_URL);
        continue;
      }

      if (page.kind === 'BATTLE_INPUT' || page.kind === 'BATTLE_CHANGE') {
        const isForceSwitch = page.kind === 'BATTLE_CHANGE';
        const battleTurn = parseBattleTurnHtml(page.html);
        const turnNum = battleTurn.nowVal === '' ? 1 : (parseInt(battleTurn.nowVal, 10) + 1);
        const actionType = isForceSwitch ? 'CHANGE' : 'INPUT';
        const turnKey = `${actionType}:${battleTurn.nowVal}:${battleTurn.options.map(o => o.value).join(',')}`;

        if (mySide === null) {
          mySide = 0;
        }
        const foeSide = 1 - mySide;
        if (!foeName && battleTurn.sides && battleTurn.sides[foeSide]?.trainerName) {
          foeName = battleTurn.sides[foeSide].trainerName;
        }

        // Fetch PDAT and LLOG for richer board context (non-fatal if either fails)
        let pdat = null;
        let llog = null;
        try {
          const pdatRes = await client.get(`${ROOM_URL}?ACTION=PDAT`);
          pdat = parsePdatHtml(pdatRes.html);
        } catch {}
        try {
          const llogRes = await client.get(`${ROOM_URL}?ACTION=LLOG`);
          llog = parseLlogHtml(llogRes.html, previewSides || [[], []]);
          if (llog && llog.sides[foeSide]) {
            lastLlogFoeMons = llog.sides[foeSide].mons;
          }
        } catch {}

        // Update persistent `seenFoeMons` with Castle leaks (exact HP/maxHp, item reveals, Leftovers absence)
        if (seenFoeMons && previewSides && previewSides[foeSide]) {
          const foeTop = battleTurn.sides ? battleTurn.sides[foeSide] : null;
          let foeActiveSlot = null;
          if (foeTop && foeTop.speciesId) {
            foeActiveSlot = previewSides[foeSide].findIndex(m => m.speciesId === foeTop.speciesId);
          }
          if (foeActiveSlot === null || foeActiveSlot < 0) {
            foeActiveSlot = llog?.sides?.[foeSide]?.activeSlot ?? null;
          }

          for (let i = 0; i < 6; i++) {
            const seen = seenFoeMons[i];
            const llogMon = llog?.sides?.[foeSide]?.mons?.[i] || null;
            if (!seen) continue;
            if (llogMon) {
              if (llogMon.appeared) seen.appeared = true;
              if (llogMon.fainted) {
                seen.fainted = true;
                seen.hp = 0;
                seen.status = '';
              }
              if (llogMon.itemOriginal) {
                seen.itemOriginal = seen.itemOriginal || llogMon.itemOriginal;
                seen.itemCurrent = llogMon.itemCurrent;
                if (seen.itemOriginal === 'leftovers') seen.noLeftovers = false;
              }
              for (const mvId of llogMon.revealedMoves || []) {
                if (!seen.revealedMoves.includes(mvId)) {
                  seen.revealedMoves.push(mvId);
                }
              }
            }
          }

          if (foeActiveSlot !== null && foeActiveSlot >= 0 && seenFoeMons[foeActiveSlot] && foeTop) {
            const activeSeen = seenFoeMons[foeActiveSlot];
            const activeLlog = llog?.sides?.[foeSide]?.mons?.[foeActiveSlot] || null;
            activeSeen.appeared = true;
            if (foeTop.maxHp !== null && foeTop.maxHp > 0) {
              activeSeen.maxHp = foeTop.maxHp;
            }
            if (foeTop.hp !== null) {
              const prevHp = activeSeen.hp;
              activeSeen.hp = foeTop.hp;
              activeSeen.fainted = foeTop.hp <= 0;
              activeSeen.status = foeTop.hp <= 0 ? '' : (foeTop.status || '');

              // Detect Leftovers absence (`noLeftovers`):
              // At normal turn boundary (`BATTLE_INPUT`, `turnNum >= 2`), if the opponent's active
              // Pokemon has `0 < hp < maxHp`, has `itemFlag = true`, has never shown Leftovers,
              // and either was already damaged on a previous turn OR did not just switch in and has
              // no end-of-turn residual damage source (poison, burn, leech seed, curse, nightmare, sandstorm),
              // then it could not have been at 100% HP when Leftovers checked -> it does NOT hold Leftovers!
              if (
                !isForceSwitch &&
                turnNum >= 2 &&
                activeSeen.itemFlag &&
                !activeSeen.itemOriginal &&
                activeSeen.maxHp &&
                foeTop.hp > 0 &&
                foeTop.hp < activeSeen.maxHp
              ) {
                activeSeen.damagedTurnsCount = (activeSeen.damagedTurnsCount || 0) + 1;
                const hasResidualDamage =
                  activeSeen.status === 'psn' ||
                  activeSeen.status === 'tox' ||
                  activeSeen.status === 'brn' ||
                  activeLlog?.volatiles?.has('leechseed') ||
                  activeLlog?.volatiles?.has('nightmare') ||
                  activeLlog?.volatiles?.has('curse') ||
                  llog?.weather?.key === 'sandstorm';
                const stayedInFromLastTurn = lastFoeActiveSlot === foeActiveSlot;
                if (
                  activeSeen.damagedTurnsCount >= 2 ||
                  (stayedInFromLastTurn && !hasResidualDamage) ||
                  (!hasResidualDamage && prevHp !== null && prevHp < activeSeen.maxHp)
                ) {
                  if (!activeSeen.noLeftovers) {
                    activeSeen.noLeftovers = true;
                    log(`Leak inference: Foe ${activeSeen.species} (${foeTop.hp}/${activeSeen.maxHp}) lacks Leftovers.`);
                  }
                }
              }
            }
            lastFoeActiveSlot = foeActiveSlot;
          }
        }

        // If pickedRosterSlots was not set (e.g., resumed mid-game), infer from PDAT
        if (!pickedRosterSlots && pdat && pdat.mons.length > 0) {
          const inferred = [];
          for (const pm of pdat.mons) {
            const idx = team.sets.findIndex(
              (s, i) => !inferred.includes(i) && toId(s.species) === pm.speciesId
            );
            if (idx >= 0) inferred.push(idx);
          }
          for (let i = 0; i < 6 && inferred.length < 3; i++) {
            if (!inferred.includes(i)) inferred.push(i);
          }
          pickedRosterSlots = inferred.slice(0, 3);
        }

        let bestChoice = null;
        let rootPolicy = null;
        const spec = buildAndSanitizePositionSpec({
          mySide,
          turn: turnNum,
          ownSets: team.sets,
          previewSides,
          pickedRosterSlots,
          battleTurn,
          pdat,
          llog,
          seenFoeMons,
          teamPreview: false,
          forceSwitch: isForceSwitch,
        });

        if (!RANDOM && battleTurn.options.length > 1) {
          try {
            const s = ensureSearcher(mySide, false);
            s.setPosition(JSON.stringify(spec));
            s.step(ITERS);
            bestChoice = s.best();
            try {
              rootPolicy = JSON.parse(s.rootPolicy());
            } catch {}
          } catch (err) {
            log(`Turn ${turnNum} (${actionType}) search fallback: ${err.message || err}`);
          }
        }

        const chosenMoveValue = selectLegalPbsMoveOption({
          bestChoice,
          rootPolicy,
          spec,
          pickedRosterSlots,
          options: battleTurn.options,
          ownSets: team.sets,
          isRandom: RANDOM,
        });

        const chosenOptObj = battleTurn.options.find(o => o.value === chosenMoveValue);
        log(
          `Turn ${turnNum} (${actionType}, NOW="${battleTurn.nowVal}"): ` +
          `submitting MOVE=${chosenMoveValue} (${chosenOptObj ? chosenOptObj.label : ''})` +
          (bestChoice ? ` [search: ${bestChoice}]` : '')
        );

        decisions++;
        lastTurnActedKey = turnKey;
        appendDecisionLog({
          gameIdx,
          phase: actionType,
          turn: turnNum,
          nowVal: battleTurn.nowVal,
          side: mySide,
          foeName,
          bestChoice,
          chosenMoveValue,
          options: battleTurn.options,
          rootPolicy,
        });

        const postFields = {
          ACTION: actionType,
          MOVE: chosenMoveValue,
          NOW: battleTurn.nowVal,
        };
        res = await client.postForm(ROOM_URL, postFields);
        continue;
      }

      // Unknown or transitional page: wait and poll ROOM_URL
      log(`Unhandled page state (${page.kind}), polling ${ROOM_URL}...`);
      await sleep(POLL_MS);
      res = await client.get(ROOM_URL);
    }
  } finally {
    if (searcher && typeof searcher.free === 'function') {
      try { searcher.free(); } catch {}
    }
  }
}

async function main() {
  for (let g = 0; g < GAMES; g++) {
    await playOneGame(g);
    if (g + 1 < GAMES) {
      await sleep(POLL_MS);
    }
  }
}

if (require.main === module) {
  main().catch(err => {
    console.error(`[${NAME}] Fatal error:`, err);
    process.exit(1);
  });
}

module.exports = {
  PbsHttpClient,
  classifyPage,
  parseTeamPreviewHtml,
  parseBattleTurnHtml,
  parsePdatHtml,
  parseLlogHtml,
  calcSetMaxHp,
  inferHpTypeMaskFromMaxHp,
  buildDynamicBeliefPoolJson,
  recordOpponentObservation,
  buildAndSanitizePositionSpec,
  selectLegalPbsMoveOption,
  encodeTeamForPbsCookie,
  encodeFormEucJp,
  resolveBotOutcomeFromFinishedHtml,
  TIER_A_BOX_IDS,
};
