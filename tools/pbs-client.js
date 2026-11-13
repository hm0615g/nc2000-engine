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
 *   2. Battle decisions use the Castle observation adapter and unchanged WASM.
 *      Inconsistent input or an unmapped action stops the match with evidence.
 *      Action POSTs are attempted once after checking the current raw sources;
 *      uncertain responses are reconciled by reading, never by resending.
 *
 * Usage:
 *   node tools/pbs-client.js --room K --name nc2000bot --team pool:nash [options]
 */
'use strict';

const fs = require('fs');
const assert = require('node:assert/strict');
const path = require('path');
const net = require('net');
const { encodeTeamForPdinput, PDINPUT_ITEM_CODES } = require('./export-pdinput');
const {readDecisionForm} = require('./pbs-v32-decision-boundary');
const {captureStableBundle, createSubmissionGuard} = require('./pbs-castle-submission');

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
  --mode MODE          auto | host | join (default host)
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
  --trace-dir DIR      Directory for per-match raw HTML + state trace bundles
                       (default: data/pbs-traces; --no-trace to disable)
  --anomaly-log FILE   JSONL path for automated parity check anomalies
                       (default: data/pbs-anomalies.jsonl; --no-anomaly-log to disable)
  --no-human-delay     Disable human-like minimum pacing delays
  --random             Uniform legal driver (WASM input validation, no search)
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
  const s = String(raw || 'K').trim();
  if (/^https?:\/\//i.test(s)) return s.replace(/\?.*$/, '');
  const daiMatch = s.match(/^(?:第)?([1-6１-６一二三四五六])$/);
  if (daiMatch && DISCORD_DAI_TO_LETTER[daiMatch[1]]) {
    return `http://psense.lib.net/PBS/_/${DISCORD_DAI_TO_LETTER[daiMatch[1]]}/PS.cgi`;
  }
  const letter = s.toUpperCase();
  return `http://psense.lib.net/PBS/_/${letter}/PS.cgi`;
}

const ROOM_URL = resolveRoomUrl(args.room || 'K');
const NAME = String(args.name || 'nc2000bot');
const PASS = String(args.pass || 'nc2pass');
const MSG = String(args.msg || 'よろしくおねがいします');
const MODE = String(args.mode || 'host').toLowerCase();
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
const ENTRY_STATUS_IPC = Boolean(args['entry-status-ipc']);
let entryStatusSent = false;

function reportEntryStatus(status, reason) {
  if (!ENTRY_STATUS_IPC || entryStatusSent) return;
  if (typeof process.send !== 'function') return;
  entryStatusSent = true;
  process.send({
    type: 'PBS_ENTRY_STATUS',
    status,
    reason,
  });
}
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
const TRACE_DIR_PATH = args['no-trace']
  ? ''
  : path.resolve(
      REPO,
      args['trace-dir'] && args['trace-dir'] !== true
        ? String(args['trace-dir'])
        : 'data/pbs-traces'
    );
const ANOMALY_LOG_PATH = args['no-anomaly-log']
  ? ''
  : path.resolve(
      REPO,
      args['anomaly-log'] && args['anomaly-log'] !== true
        ? String(args['anomaly-log'])
        : 'data/pbs-anomalies.jsonl'
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
  if (/name="?ACTION"?\s+value="?BATONN"?/i.test(html)) {
    return { kind: 'BATTLE_BATONN', html };
  }
  if (/name="?ACTION"?\s+value="?CHANGE"?/i.test(html)) {
    return { kind: 'BATTLE_CHANGE', html };
  }
  if (/name="?ACTION"?\s+value="?INPUT"?/i.test(html)) {
    return { kind: 'BATTLE_INPUT', html };
  }

  // Castle-specific non-actionable waiting pages captured from room K.
  // Do not submit a form or consider these states unknown-page failures.
  const castleRefreshMeta = /<meta\b[^>]*\bhttp-equiv\s*=\s*["']?\s*refresh(?:["'\s>])/i.test(html);
  const castleRefreshForm = /<form\b[^>]*\baction\s*=\s*["']?(?:\.\/)?PS\.cgi(?:["'\s>])/i.test(html);
  const castleEntryWaiting = castleRefreshMeta
    && /<title>[^<]*\u30a8\u30f3\u30c8\u30ea\u30fc\u5f85\u3061[^<]*<\/title>/i.test(html)
    && /\u5bfe\u6226\u8005\u3092\u5f85\u3063\u3066\u3044\u307e\u3059/.test(html)
    && /Please\s+wait\s*!/i.test(html);
  if (castleEntryWaiting) return {kind: 'WAITING_ENTRY', html};
  const castleSelectWaitInputs = html.match(/<input\b[^>]*>/gi) || [];
  const castleSelectWaitForms = html.match(/<form\b[^>]*>/gi) || [];
  const castleSelectWaiting = castleRefreshMeta && castleRefreshForm
    && /\u4f7f\u7528\u30dd\u30b1\u30e2\u30f3\u9078\u629e/.test(html)
    && castleSelectWaitForms.length === 1 && castleSelectWaitInputs.length === 1
    && /\btype\s*=\s*["']?submit(?:["'\s>])/i.test(castleSelectWaitInputs[0])
    && /\bvalue\s*=\s*["']?\u66f4\u65b0(?:["'\s>])/i.test(castleSelectWaitInputs[0])
    && !/\bname\s*=/i.test(castleSelectWaitInputs[0])
    && !/<(?:input|button)\b[^>]*\bname\s*=\s*["']?ACTION\b/i.test(html);
  if (castleSelectWaiting) return {kind: 'WAITING_SELECT', html};
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

  const parts = html.split(/<td[^>]*VAlign="?middle"?[^>]*>/i);
  if (parts.length >= 3) {
    const p1Chunk = parts[1].split(/<\/td>|<td|<\/tr>/i)[0];
    const p2Chunk = parts[2].split(/<\/td>|<td|<\/tr>/i)[0];
    result.p1Mons = parsePreviewTtCell(p1Chunk);
    result.p2Mons = parsePreviewTtCell(p2Chunk);
  } else {
    const tdMatches = [...html.matchAll(/<td[^>]*VAlign="?middle"?[^>]*>([\s\S]*?)<\/td>/gi)];
    if (tdMatches.length >= 2) {
      result.p1Mons = parsePreviewTtCell(tdMatches[0][1]);
      result.p2Mons = parsePreviewTtCell(tdMatches[1][1]);
    }
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
    const mappedSpId = JP_TO_SPECIES_ID[jpName] || null;
    const speciesId = mappedSpId || 'pikachu';
    const spObj = dexData.species[speciesId];
    const monObj = {
      species: spObj ? spObj.name : 'Pikachu',
      speciesId,
      level,
      gender,
      item_flag: itemFlag,
    };
    if (!mappedSpId && jpName) {
      Object.defineProperty(monObj, '_unmappedJp', {
        value: jpName,
        enumerable: false,
        configurable: true,
      });
    }
    mons.push(monObj);
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
        const optObj = {
          value: value.toUpperCase(),
          label,
          kind: 'move',
          slotIndex,
          moveId,
          pp,
        };
        if (!moveId && moveNameJp) {
          Object.defineProperty(optObj, '_unmappedJp', {
            value: moveNameJp,
            enumerable: false,
            configurable: true,
          });
        }
        options.push(optObj);
      } else if (/^C(\d)$/i.test(value)) {
        const slotIndex = parseInt(value.slice(1), 10); // 1..3
        const hpMatch = label.match(/\[\s*(\d+)\s*\/\s*(\d+)\s*\]/);
        const hp = hpMatch ? parseInt(hpMatch[1], 10) : null;
        const maxHp = hpMatch ? parseInt(hpMatch[2], 10) : null;
        const spNameJp = normJp(label.replace(/\[\s*\d+\s*\/\s*\d+\s*\].*$/, ''));
        const speciesId = JP_TO_SPECIES_ID[spNameJp] || null;
        const optObj = {
          value: value.toUpperCase(),
          label,
          kind: 'switch',
          slotIndex,
          speciesId,
          hp,
          maxHp,
        };
        if (!speciesId && spNameJp) {
          Object.defineProperty(optObj, '_unmappedJp', {
            value: spNameJp,
            enumerable: false,
            configurable: true,
          });
        }
        options.push(optObj);
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
      const rawSpJp = normJp(beforeL);
      const spId = JP_TO_SPECIES_ID[rawSpJp] || null;
      sides[s].speciesId = spId;
      if (!spId && rawSpJp) {
        Object.defineProperty(sides[s], '_unmappedJp', {
          value: rawSpJp,
          enumerable: false,
          configurable: true,
        });
      }
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
    const rawSpJp = normJp(rawName);
    const speciesId = JP_TO_SPECIES_ID[rawSpJp] || null;
    const hp = parseInt(m[3], 10);
    const maxHp = parseInt(m[4], 10);
    const status = parseStatusText(m[5]);
    const moves = [];
    const unmappedMovesJp = [];
    for (let i = 6; i <= 9; i++) {
      const cell = m[i].replace(/<[^>]*>/g, '').trim();
      const mvMatch = cell.match(/^(.+?)\s*\(\s*(\d+)\s*\)$/);
      if (mvMatch) {
        const mvJp = normJp(mvMatch[1]);
        const mvId = JP_TO_MOVE_ID[mvJp] || null;
        const pp = parseInt(mvMatch[2], 10);
        if (mvId) {
          moves.push({ moveId: mvId, pp });
        } else if (mvJp) {
          unmappedMovesJp.push(mvJp);
        }
      }
    }
    const rawItem = normJp(m[10] || '');
    const mappedItem = rawItem && rawItem !== 'なし' ? (JP_TO_ITEM_ID[rawItem] || null) : '';
    const item = mappedItem || '';
    const monObj = {
      speciesId,
      level,
      gender,
      status: hp <= 0 ? 'fnt' : status,
      hp,
      maxHp,
      item,
      moves,
    };
    if (!speciesId && rawSpJp) {
      Object.defineProperty(monObj, '_unmappedJp', {
        value: rawSpJp,
        enumerable: false,
        configurable: true,
      });
    }
    if (unmappedMovesJp.length > 0) {
      Object.defineProperty(monObj, '_unmappedMovesJp', {
        value: unmappedMovesJp,
        enumerable: false,
        configurable: true,
      });
    }
    if (rawItem && rawItem !== 'なし' && !mappedItem) {
      Object.defineProperty(monObj, '_unmappedItemJp', {
        value: rawItem,
        enumerable: false,
        configurable: true,
      });
    }
    mons.push(monObj);
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

const TIMED_SIDE_CONDITIONS = new Set(['reflect', 'lightscreen', 'safeguard']);
const THRASH_CLASS_MOVES = new Set(['thrash', 'petaldance', 'outrage']);
const BIND_CLASS_MOVES = new Set(['wrap', 'bind', 'firespin', 'clamp', 'whirlpool']);
const TWO_TURN_CHARGE_MOVES = new Set([
  'fly',
  'dig',
  'solarbeam',
  'skullbash',
  'skyattack',
  'razorwind',
]);
const PROTECT_CLASS_MOVES = new Set(['protect', 'detect', 'endure']);
const NO_COPY_VOLATILE_KEYS = new Set([
  'attract',
  'defensecurl',
  'destinybond',
  'disable',
  'encore',
  'minimize',
  'nightmare',
  'partiallytrapped',
  'rage',
  'bide',
  'furycutter',
]);

/**
 * Advance a `parseLlogHtml` tracker state (`info`) up to `targetTurn`, executing
 * `ProtocolTracker` turn-boundary transitions (`|turn|N`) for each elapsed turn:
 * - resets `upkeepThisTurn`, `actedThisTurn`, `pendingBp`
 * - shifts `faintedThisTurn` -> `faintedLastTurn`
 * - updates `stallStreak` / `lastProtectTurn` / `protectedThisTurn`
 * - advances weather `upkeeps` and expires 5-turn weather (`upkeeps >= 5`)
 * - expires 5-turn side conditions (`reflect`, `lightscreen`, `safeguard` when `t - start >= 5`)
 * - ticks down active `perishsong` counters if not already ticked on the turn
 */
function advanceLlogTrackerToTurn(info, targetTurn) {
  if (!info || typeof targetTurn !== 'number' || Number.isNaN(targetTurn)) return info;
  const startTurn = Math.max(1, info.turn || 1);
  if (targetTurn <= startTurn) return info;

  for (let t = startTurn + 1; t <= targetTurn; t++) {
    const prevTurn = t - 1;
    info.upkeepThisTurn = false;

    if (info.weather) {
      const wStart = info.weather.start_turn || 1;
      if (wStart <= prevTurn && info.weather._lastUpkeepTurn !== prevTurn) {
        info.weather.upkeeps = (info.weather.upkeeps || 0) + 1;
        info.weather._lastUpkeepTurn = prevTurn;
      }
      if ((info.weather.upkeeps || 0) >= 5 || (t - wStart) >= 5) {
        info.weather = null;
      }
    }

    for (const s of [0, 1]) {
      const side = info.sides[s];
      if (!side) continue;
      side.actedThisTurn = false;
      side.pendingBp = false;
      side.faintedLastTurn = side.faintedThisTurn ?? null;
      side.faintedThisTurn = null;

      if (side.conditions) {
        for (const [condKey, condStart] of [...side.conditions.entries()]) {
          if (TIMED_SIDE_CONDITIONS.has(condKey) && (t - condStart) >= 5) {
            side.conditions.delete(condKey);
          }
        }
      }

      const aSlot = side.activeSlot;
      if (aSlot !== null && aSlot !== undefined && side.mons[aSlot]) {
        const m = side.mons[aSlot];
        if (m.protectedThisTurn) {
          m.stallStreak = (m.stallStreak || 0) + 1;
          m.lastProtectTurn = prevTurn;
          m.protectedThisTurn = false;
        } else if ((m.lastProtectTurn || 0) + 1 < Math.max(1, t)) {
          m.stallStreak = 0;
        }

        if (m.volatiles && m.volatiles.has('perishsong')) {
          const ps = m.volatiles.get('perishsong');
          if (ps && (ps.start_turn || 1) <= prevTurn && ps._lastTickTurn !== prevTurn) {
            ps.counter = Math.max(0, (ps.counter ?? 3) - 1);
            ps._lastTickTurn = prevTurn;
            if (ps.counter <= 0) {
              m.volatiles.delete('perishsong');
            }
          }
        }
      }
    }

    info.turn = t;
  }

  return info;
}

/**
 * Parse `?ACTION=LLOG` cumulative battle log HTML through an event-driven
 * `ProtocolTracker`-equivalent state machine to reconstruct full public state:
 * - `<HR>` turn progression (`turn = Math.max(1, hrCount - 1)` on standard PBS LLOG)
 * - per-side: `activeSlot`, `conditions` (`reflect`, `lightscreen`, `safeguard`, `spikes`),
 *   `pendingBp`, `actedThisTurn`, `faintedThisTurn`, `faintedLastTurn`, `lastMove`
 * - per-mon: `appeared`, `appearCount`, `switchInTurn`, `active`, `fainted`, `status`,
 *   `rest`, `slept`, `toxCounter`, `compBrn`, `compPar`, `boosts`, `volatiles`,
 *   `uses`, `revealedMoves`, `locked`, `charging`, `mustRecharge`, `transformedInto`,
 *   `mimicOverlay`, `lastMove`, `stallStreak`, `lastProtectTurn`, `protectedThisTurn`,
 *   `itemOriginal`, `itemCurrent`, `itemGained`
 * - field: `weather` (`{ key, upkeeps }`), `turn`, `upkeepThisTurn`
 */
function parseLlogHtml(html, previewSides = [[], []]) {
  const isEffectExpiryBold = function isEffectExpiryBold(block, bm) {
  const compact = text => String(text)
    .replace(/<[^>]*>/g, '')
    .normalize('NFKC')
    .replace(/[\u200b-\u200d\ufeff\u2060]/g, '')
    .replace(/\s+/g, '');
  const prefix = compact(block.slice(0, bm.index));
  const suffix = compact(block.slice(bm.index + bm[0].length));
  return prefix.endsWith('\u306b\u304b\u304b\u3063\u3066\u3044\u305f') &&
    /^\u306e\u52b9\u679c\u304c(?:\u304d\u308c\u305f|\u5207\u308c\u305f|\u304d\u3048\u305f|\u6d88\u3048\u305f|\u306a\u304f\u306a\u3063\u305f|\u3068\u3051\u305f|\u89e3\u3051\u305f)/u.test(suffix);
};
  const createMonTrack = () => ({
    appeared: false,
    appearCount: 0,
    switchInTurn: 0,
    active: false,
    fainted: false,
    status: '',
    rest: false,
    slept: 0,
    toxCounter: null,
    compBrn: false,
    compPar: false,
    boosts: [0, 0, 0, 0, 0, 0, 0],
    uses: {},
    revealedMoves: new Set(),
    itemOriginal: null,
    itemCurrent: null,
    itemGained: false,
    volatiles: new Map(),
    locked: null,
    charging: null,
    mustRecharge: false,
    transformedInto: null,
    mimicOverlay: null,
    lastMove: null,
    stallStreak: 0,
    lastProtectTurn: 0,
    protectedThisTurn: false,
  });

  const info = {
    sides: [
      {
        mons: ((previewSides && previewSides[0]) || []).map(() => createMonTrack()),
        activeSlot: null,
        conditions: new Map(),
        pendingBp: false,
        actedThisTurn: false,
        faintedThisTurn: null,
        faintedLastTurn: null,
        lastMove: null,
      },
      {
        mons: ((previewSides && previewSides[1]) || []).map(() => createMonTrack()),
        activeSlot: null,
        conditions: new Map(),
        pendingBp: false,
        actedThisTurn: false,
        faintedThisTurn: null,
        faintedLastTurn: null,
        lastMove: null,
      },
    ],
    weather: null,
    turn: 1,
    hrCount: 0,
    upkeepThisTurn: false,
    events: [],
    unrecognizedLines: [],
  };

  if (!html) return info;
  const pendingCalledMove = [null,null];
  const pendingTransform = [null,null];

  let p1Name = '';
  let p2Name = '';
  const nameRowMatch = html.match(
    /<tr[^>]*>\s*<t[dh][^>]*>\s*([^<]*?)\s*<\/t[dh]>\s*<td[^>]*ROWSPAN="?2"?[^>]*>[\s\S]*?<\/td>\s*<t[dh][^>]*>\s*([^<]*?)\s*<\/t[dh]>\s*<\/tr>/i
  );
  if (nameRowMatch) {
    p1Name = nameRowMatch[1].trim();
    p2Name = nameRowMatch[2].trim();
  }

  const findExplicitSlot = (sideIdx, text) => {
    const roster = (previewSides && previewSides[sideIdx]) || [];
    const normText = normJp(text);
    const sortedSlots = roster
      .map((m, idx) => {
        const spId = m.speciesId || toId(m.species);
        const jp = i18n.species[spId] || '';
        return { idx, jp, normJp: normJp(jp) };
      })
      .filter(e => e.jp)
      .sort((a, b) => b.normJp.length - a.normJp.length);

    for (const { idx, normJp: sJp } of sortedSlots) {
      if (normText.includes(sJp)) return idx;
    }
    return null;
  };

  const findSlot = (sideIdx, text) => {
    const explicit = findExplicitSlot(sideIdx, text);
    return explicit !== null ? explicit : info.sides[sideIdx].activeSlot;
  };

  const clearOnExit = mon => {
    mon.boosts = [0, 0, 0, 0, 0, 0, 0];
    mon.volatiles.clear();
    mon.locked = null;
    mon.charging = null;
    mon.mustRecharge = false;
    mon.transformedInto = null;
    mon.mimicOverlay = null;
    mon.lastMove = null;
    mon.stallStreak = 0;
    mon.protectedThisTurn = false;
    mon.toxCounter = null;
    mon.compBrn = false;
    mon.compPar = false;
    if (mon.status === 'tox') {
      mon.status = 'psn';
    }
  };

  const clearSourcedVols = (srcSide, srcSlot) => {
    if (srcSlot === null || srcSlot === undefined) return;
    for (const s of [0, 1]) {
      for (const m of info.sides[s].mons) {
        for (const [k, v] of [...m.volatiles.entries()]) {
          if (
            (k === 'trapped' || k === 'partiallytrapped') &&
            Array.isArray(v.source) &&
            v.source[0] === srcSide &&
            v.source[1] === srcSlot
          ) {
            m.volatiles.delete(k);
          }
        }
      }
    }
  };

  const cureMonStatus = mon => {
    mon.status = '';
    mon.rest = false;
    mon.slept = 0;
    mon.volatiles.delete('nightmare');
  };

  const applyBoostDelta = (mon, statIdx, delta) => {
    if (!mon || delta === 0 || statIdx < 0 || statIdx > 6) return;
    mon.boosts[statIdx] = Math.max(-6, Math.min(6, mon.boosts[statIdx] + delta));
    if (statIdx === 4 && mon.status === 'par') {
      mon.compPar = false;
    }
    if (statIdx === 0 && mon.status === 'brn') {
      mon.compBrn = false;
    }
  };

  const setActive = (sideIdx, slot, { isDrag = false, isInitialSendOut = false } = {}) => {
    if (slot === null || slot === undefined || slot < 0) return;
    const side = info.sides[sideIdx];
    const targetMon = side.mons[slot];
    if (side.activeSlot !== null && side.activeSlot !== slot) {
      pendingCalledMove[sideIdx] = null;
      pendingTransform[sideIdx] = null;
    }

    // If already active and unfainted, do not treat as a fresh switch
    if (side.activeSlot === slot && targetMon && targetMon.active && !targetMon.fainted) {
      targetMon.appeared = true;
      targetMon.appearCount = Math.max(1, targetMon.appearCount || 1);
      return;
    }

    const bp = Boolean(side.pendingBp && !isDrag);
    let inherit = null;

    if (side.activeSlot !== null && side.activeSlot !== slot && side.mons[side.activeSlot]) {
      const oldSlot = side.activeSlot;
      const oldMon = side.mons[oldSlot];
      oldMon.active = false;
      if (bp) {
        const copiedVols = new Map();
        for (const [vk, vv] of oldMon.volatiles.entries()) {
          const moveCondNoCopy = Boolean(dexData.moves[vk]?.condition?.noCopy);
          if (!NO_COPY_VOLATILE_KEYS.has(vk) && !moveCondNoCopy) {
            copiedVols.set(vk, { ...vv });
          }
        }
        inherit = {
          boosts: [...oldMon.boosts],
          volatiles: copiedVols,
          toxCounter: oldMon.toxCounter,
          compBrn: oldMon.compBrn,
          compPar: oldMon.compPar,
        };
      }
      clearOnExit(oldMon);
      clearSourcedVols(sideIdx, oldSlot);
    }

    side.pendingBp = false;
    if (!isInitialSendOut) {
      side.actedThisTurn = true;
    }
    side.activeSlot = slot;

    if (targetMon) {
      targetMon.appeared = true;
      targetMon.appearCount = (targetMon.appearCount || 0) + 1;
      targetMon.switchInTurn = isInitialSendOut ? 0 : info.turn;
      targetMon.active = true;
      if (inherit) {
        targetMon.boosts = inherit.boosts;
        targetMon.volatiles = inherit.volatiles;
        targetMon.toxCounter = inherit.toxCounter;
        targetMon.compBrn = inherit.compBrn;
        targetMon.compPar = inherit.compPar;
      }
      if (targetMon.status === 'brn') targetMon.compBrn = true;
      if (targetMon.status === 'par') targetMon.compPar = true;
    }

    info.events.push({
      type: isDrag ? 'drag' : 'switch',
      turn: info.turn,
      side: sideIdx,
      slot,
      initial: isInitialSendOut,
    });
  };

  const CONSUMED_BERRIES_JP = [
    ['おうごんのきのみ', 'goldberry'],
    ['おうごんのみ', 'goldberry'],
    ['きせきのきのみ', 'miracleberry'],
    ['きせきのみ', 'miracleberry'],
    ['はっかのきのみ', 'mintberry'],
    ['はっかのみ', 'mintberry'],
    ['まひなおしのきのみ', 'przcureberry'],
    ['まひなおしのみ', 'przcureberry'],
    ['こおりなおしのきのみ', 'iceberry'],
    ['こおりなおしのみ', 'iceberry'],
    ['やけどなおしのきのみ', 'burntberry'],
    ['やけどなおしのみ', 'burntberry'],
    ['どくけしのきのみ', 'psncureberry'],
    ['どくけしのみ', 'psncureberry'],
    ['にがきのみ', 'bitterberry'],
    ['ふしぎのきのみ', 'mysteryberry'],
    ['ふしぎなきのみ', 'mysteryberry'],
    ['きのみジュース', 'berryjuice'],
    ['きのみ', 'berry'],
  ];

  const isMoveFailureText = clean =>
    clean.includes('うまくきまらなかった') ||
    clean.includes('うまく決まらなかった') ||
    clean.includes('失敗した') ||
    clean.includes('しかし外れた') ||
    clean.includes('しかしはずれた') ||
    clean.includes('外れた！') ||
    clean.includes('はずれた！') ||
    clean.includes('攻撃ははずれた') ||
    clean.includes('攻撃は外れた') ||
    clean.includes('ははずれた') ||
    clean.includes('は外れた') ||
    clean.includes('当たらなかった') ||
    clean.includes('あたらなかった') ||
    clean.includes('効果がない') ||
    clean.includes('効果はない') ||
    clean.includes('こうかがない') ||
    clean.includes('にはきかない') ||
    clean.includes('には効かない') ||
    clean.includes('攻撃を守った') ||
    clean.includes('攻撃をまもった') ||
    clean.includes('攻撃を防いだ') ||
    clean.includes('まもりきった') ||
    clean.includes('守りきった');

  // Tokenize `<HR>` and `<P ALIGN="left|right|center">...</P>` in document order
  const tokenRegex = /(<HR[^>]*>)|<P\s+ALIGN="?(left|right|center)"?\s*>([\s\S]*?)(?=<P\s+ALIGN|<HR|<\/BODY|$)/gi;
  let hrCount = 0;
  let sawPBeforeFirstHr = false;
  let firstPWasSendOutOnly = false;
  let pCount = 0;
  let inInitialSendOutPhase = false;

  const computeTurnFromHr = () => {
    if (!sawPBeforeFirstHr) {
      return hrCount <= 1 ? 1 : Math.max(1, hrCount - 1);
    }
    if (firstPWasSendOutOnly) {
      return hrCount === 0 ? 1 : Math.max(1, hrCount);
    }
    return hrCount + 1;
  };

  for (const m of html.matchAll(tokenRegex)) {
    if (m[1]) {
      hrCount++;
      info.hrCount = hrCount;
      if (pCount > 0) {
        const nextTurn = computeTurnFromHr();
        if (inInitialSendOutPhase && ((!sawPBeforeFirstHr && hrCount >= 2) || (sawPBeforeFirstHr && hrCount >= 1))) {
          inInitialSendOutPhase = false;
          info.sides[0].actedThisTurn = false;
          info.sides[1].actedThisTurn = false;
        }
        if (nextTurn > info.turn) {
          pendingCalledMove[0] = pendingCalledMove[1] = null;
          pendingTransform[0] = pendingTransform[1] = null;
          advanceLlogTrackerToTurn(info, nextTurn);
        }
      }
      continue;
    }

    const align = m[2].toLowerCase();
    const block = m[3];
    const rawClean = block.replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ').trim();
    const clean = rawClean.replace(/\s+/g, '');
    const normClean = normJp(clean);

    if (align === 'center') {
      const turnNum = info.turn;
      let recognizedCenter = false;
      if (
        normClean.includes(normJp('雨が降り')) ||
        normClean.includes(normJp('あめがふり')) ||
        normClean.includes(normJp('日差しが強い')) ||
        normClean.includes(normJp('ひざしがつよい')) ||
        normClean.includes(normJp('砂嵐が')) ||
        normClean.includes(normJp('すなあらしが')) ||
        normClean.includes(normJp('砂嵐でダメージ'))
      ) {
        recognizedCenter = true;
        info.upkeepThisTurn = true;
        if (info.weather && info.weather._lastUpkeepTurn !== turnNum) {
          info.weather.upkeeps = (info.weather.upkeeps || 0) + 1;
          info.weather._lastUpkeepTurn = turnNum;
          if (info.weather.upkeeps >= 5) {
            info.weather = null;
          }
        }
      }
      if (
        normClean.includes(normJp('雨はやんだ')) ||
        normClean.includes(normJp('あめはやんだ')) ||
        normClean.includes(normJp('雨が降り止んだ')) ||
        normClean.includes(normJp('あめがふりやんだ')) ||
        normClean.includes(normJp('日差しが戻った')) ||
        normClean.includes(normJp('ひざしがもどった')) ||
        normClean.includes(normJp('日差しが弱まった')) ||
        normClean.includes(normJp('ひざしがよわまった')) ||
        normClean.includes(normJp('日差しが元に戻った')) ||
        normClean.includes(normJp('ひざしがもとにもどった')) ||
        normClean.includes(normJp('砂嵐はおさまった')) ||
        normClean.includes(normJp('すなあらしはおさまった')) ||
        normClean.includes(normJp('砂嵐がおさまった')) ||
        normClean.includes(normJp('すなあらしがおさまった'))
      ) {
        recognizedCenter = true;
        info.weather = null;
      }
      if (
        !recognizedCenter &&
        clean &&
        !/WIN!|ＷＩＮ！|LOSE|ＬＯＳＥ|DRAW|ＤＲＡＷ|ターン|対戦|バトル|引き分け/i.test(clean)
      ) {
        info.unrecognizedLines.push({
          turn: turnNum,
          side: null,
          align: 'center',
          line: rawClean,
        });
      }
      continue;
    }

    const sideIdx = align === 'left' ? 0 : 1;
    const foeIdx = 1 - sideIdx;

    const isSendOutBlock =
      clean.includes('を繰り出した') ||
      clean.includes('をくりだした') ||
      clean.includes('ゆけっ！') ||
      clean.includes('いけっ！') ||
      clean.includes('出てこい');
    const isDragBlock =
      clean.includes('吹き飛ばされた') ||
      clean.includes('ふきとばされた') ||
      clean.includes('引きずり出された') ||
      clean.includes('ひきずりだされた');
    const hasMoveTag = [...block.matchAll(/<B>\s*([^<]+?)\s*<\/B>/gi)].some(
      bm => Boolean(JP_TO_MOVE_ID[normJp(bm[1])])
    );

    if (pCount === 0 && hrCount === 0) {
      sawPBeforeFirstHr = true;
      firstPWasSendOutOnly = isSendOutBlock && !hasMoveTag;
    }
    pCount++;

    // Determine the turn number for this `<P>` block based on `<HR>` count:
    // Standard PBS LLOG has `HR #1` before initial send-out, `HR #2` before Turn 1,
    // `HR #3` before Turn 2, etc. (`turn = Math.max(1, hrCount - 1)`).
    let blockTurn = 1;
    let isInitialSendOut = false;
    if (!sawPBeforeFirstHr) {
      if (hrCount <= 1) {
        blockTurn = 1;
        isInitialSendOut = isSendOutBlock && !hasMoveTag;
        inInitialSendOutPhase = isInitialSendOut;
      } else {
        blockTurn = Math.max(1, hrCount - 1);
        if (inInitialSendOutPhase) {
          inInitialSendOutPhase = false;
          info.sides[0].actedThisTurn = false;
          info.sides[1].actedThisTurn = false;
        }
      }
    } else if (firstPWasSendOutOnly) {
      if (hrCount === 0) {
        blockTurn = 1;
        isInitialSendOut = isSendOutBlock && !hasMoveTag;
        inInitialSendOutPhase = isInitialSendOut;
      } else {
        blockTurn = Math.max(1, hrCount);
        if (inInitialSendOutPhase) {
          inInitialSendOutPhase = false;
          info.sides[0].actedThisTurn = false;
          info.sides[1].actedThisTurn = false;
        }
      }
    } else {
      blockTurn = hrCount + 1;
    }

    if (blockTurn > info.turn) {
      pendingCalledMove[0] = pendingCalledMove[1] = null;
      pendingTransform[0] = pendingTransform[1] = null;
      advanceLlogTrackerToTurn(info, blockTurn);
    }
    const turnNum = info.turn;

    // 1. Check voluntary switch-in / send-out cues
    if (isSendOutBlock && !isDragBlock) {
      const sendOutB = block.match(/<B>\s*([^<]+?)\s*<\/B>\s*を(?:繰り出した|くりだした)/i);
      const slot = sendOutB ? findSlot(sideIdx, sendOutB[1]) : findSlot(sideIdx, clean);
      if (slot !== null) {
        const initFlag = isInitialSendOut || (turnNum === 1 && info.sides[sideIdx].activeSlot === null);
        setActive(sideIdx, slot, { isDrag: false, isInitialSendOut: initFlag });
      }
    }

    // 2. Check `cant` cues (sleep, recharge, paralysis, freeze, flinch, attract, disable)
    const activeSlotBeforeMove = info.sides[sideIdx].activeSlot ?? findSlot(sideIdx, clean);
    const activeMonBeforeMove =
      activeSlotBeforeMove !== null ? info.sides[sideIdx].mons[activeSlotBeforeMove] : null;

    if (
      clean.includes('ぐうぐう眠っている') ||
      clean.includes('ぐうぐうねむっている') ||
      clean.includes('眠っていて動けない') ||
      clean.includes('ねむっていて動けない') ||
      (clean.includes('眠っている') && !clean.includes('眠ってしまった') && !clean.includes('眠りはじめた'))
    ) {
      info.sides[sideIdx].actedThisTurn = true;
      if (activeMonBeforeMove) {
        activeMonBeforeMove.status = 'slp';
        activeMonBeforeMove.slept = (activeMonBeforeMove.slept || 0) + 1;
        activeMonBeforeMove.locked = null;
        activeMonBeforeMove.charging = null;
      }
      info.events.push({ type: 'cant', turn: turnNum, side: sideIdx, reason: 'slp' });
    } else if (clean.includes('反動で動けない') || clean.includes('はんどうでうごけない')) {
      info.sides[sideIdx].actedThisTurn = true;
      if (activeMonBeforeMove) {
        activeMonBeforeMove.mustRecharge = false;
        activeMonBeforeMove.charging = null;
      }
      info.events.push({ type: 'cant', turn: turnNum, side: sideIdx, reason: 'recharge' });
    } else if (
      clean.includes('しびれて動けない') ||
      clean.includes('しびれてうごけない') ||
      clean.includes('痺れて動けない') ||
      clean.includes('痺れてうごけない') ||
      clean.includes('凍ってしまって動けない') ||
      clean.includes('こおってしまって') ||
      clean.includes('ひるんで動けない') ||
      clean.includes('ひるんで技が出せない') ||
      clean.includes('メロメロで技が出せない') ||
      clean.includes('かなしばりで出せない')
    ) {
      info.sides[sideIdx].actedThisTurn = true;
      if (activeMonBeforeMove) {
        activeMonBeforeMove.charging = null;
      }
      info.events.push({ type: 'cant', turn: turnNum, side: sideIdx, reason: 'cant' });
    }

    // Confusion self-hit aborts two-turn charging move
    if (
      clean.includes('わけもわからず自分を攻撃') ||
      clean.includes('わけもわからずじぶんをこうげき')
    ) {
      info.sides[sideIdx].actedThisTurn = true;
      if (activeMonBeforeMove) {
        activeMonBeforeMove.charging = null;
      }
    }

    // 3. Check move usage: `<B>MOVE</B>` (with plain-text fallback `の MOVE！`)
    const bMatches = [...block.matchAll(/<B>\s*([^<]+?)\s*<\/B>/gi)].filter(bm => !isEffectExpiryBold(block, bm));
    const blockMoveIds = [];
    for (const bm of bMatches) {
      const mvId = JP_TO_MOVE_ID[normJp(bm[1])];
      if (mvId) blockMoveIds.push(mvId);
    }
    if (blockMoveIds.length === 0) {
      for (const pm of rawClean.matchAll(/(?:の|は)\s*([^\s！!]+?)\s*[！!]/g)) {
        const mvId = JP_TO_MOVE_ID[normJp(pm[1])];
        if (mvId) blockMoveIds.push(mvId);
      }
    }

    let callerMoveId = null;
    for (let mIdx = 0; mIdx < blockMoveIds.length; mIdx++) {
      const mvId = blockMoveIds[mIdx];
      const previousCaller = pendingCalledMove[sideIdx];
      const inheritedCall = mIdx === 0 && previousCaller &&
        previousCaller.turn === turnNum &&
        previousCaller.slot === info.sides[sideIdx].activeSlot;
      const isCalledMove = Boolean(inheritedCall) || (mIdx > 0 && (
        callerMoveId === 'sleeptalk' ||
        callerMoveId === 'mirrormove' ||
        callerMoveId === 'metronome'
      ));
      if (mIdx === 0 && (mvId === 'sleeptalk' || mvId === 'mirrormove' || mvId === 'metronome')) {
        callerMoveId = mvId;
      }

      const moveFailed = isMoveFailureText(clean);
      const actorSlot = info.sides[sideIdx].activeSlot ?? findSlot(sideIdx, clean);
      if (actorSlot !== null && info.sides[sideIdx].mons[actorSlot]) {
        setActive(sideIdx, actorSlot);
      }
      const actorMon = actorSlot !== null ? info.sides[sideIdx].mons[actorSlot] : null;
      const releasingCharge = Boolean(actorMon &&
        actorMon.charging === mvId &&
        TWO_TURN_CHARGE_MOVES.has(mvId));
      if ((mvId === 'sleeptalk' ||
           mvId === 'mirrormove' ||
           mvId === 'metronome') &&
          !moveFailed && actorSlot !== null) {
        pendingCalledMove[sideIdx] = {
          turn: turnNum, slot: actorSlot, move: mvId
        };
      } else if (isCalledMove) {
        pendingCalledMove[sideIdx] = null;
      }
      const foeSlot = info.sides[foeIdx].activeSlot ?? findExplicitSlot(foeIdx, clean);
      const foeMon = foeSlot !== null ? info.sides[foeIdx].mons[foeSlot] : null;

      // Side-level move bookkeeping (always recorded regardless of actorSlot resolution)
      info.sides[sideIdx].actedThisTurn = true;
      info.sides[sideIdx].lastMove = mvId;
      if (mvId === 'batonpass' && !moveFailed) {
        info.sides[sideIdx].pendingBp = true;
      }

      // Per-mon move bookkeeping (`ProtocolTracker::on_move` + `Observer::scan_move`)
      if (actorMon) {
        actorMon.lastMove = mvId;
        actorMon.volatiles.delete('destinybond');
        if (mvId !== 'rage') actorMon.volatiles.delete('rage');
        if (mvId !== 'furycutter') actorMon.volatiles.delete('furycutter');

        if (!isCalledMove && mvId !== 'struggle' && mvId !== 'recharge') {
          if (actorMon.charging === mvId) {
            // Release turn of a 2-turn move: clear charging without double-deducting PP
            actorMon.charging = null;
          } else {
            actorMon.charging = null;
            if (actorMon.locked && actorMon.locked.move === mvId) {
              // Continuation of thrash/petaldance/outrage/rollout: no extra PP deduction
              const cap = mvId === 'rollout' ? 5 : 3;
              const nextN = (actorMon.locked.n || 1) + 1;
              actorMon.locked = ( moveFailed && mvId === 'rollout' ) || nextN >= cap
                ? null
                : { move: mvId, n: nextN };
            } else {
              actorMon.locked = null;
              if (!actorMon.transformedInto) {
                actorMon.uses[mvId] = (actorMon.uses[mvId] || 0) + 1;
              }
              if (THRASH_CLASS_MOVES.has(mvId) && !moveFailed) {
                actorMon.locked = { move: mvId, n: 1 };
              } else if (mvId === 'rollout' && !moveFailed) {
                actorMon.locked = { move: mvId, n: 1 };
              }
            }
          }
        }

        // Reveal move unless transformed, Mimic-copied, or foreign call (Metronome/Mirror Move)
        const effectiveCallerMoveId =
          inheritedCall ? previousCaller.move : callerMoveId;
        const isForeignCall =
          isCalledMove && effectiveCallerMoveId !== 'sleeptalk';
        if (
          !actorMon.transformedInto &&
          actorMon.mimicOverlay !== mvId &&
          !isForeignCall &&
          mvId !== 'struggle' &&
          mvId !== 'recharge'
        ) {
          actorMon.revealedMoves.add(mvId);
        }

        // Two-turn charging move initiation
        if (TWO_TURN_CHARGE_MOVES.has(mvId) &&
            !moveFailed && !releasingCharge) {
          const instantSolarBeam = mvId === 'solarbeam' && info.weather?.key === 'sunnyday';
          const isChargeText =
            clean.includes('空高く飛び上がった') ||
            clean.includes('穴を掘った') ||
            clean.includes('地下深く潜り込んだ') ||
            clean.includes('光を吸収') ||
            clean.includes('ひかりを吸収') ||
            clean.includes('頭を引っ込めた') ||
            clean.includes('頭をひっこめた') ||
            clean.includes('激しい風に包まれた') ||
            clean.includes('空気がうずまく') ||
            clean.includes('激しい光をまとった') ||
            clean.includes('激しい光が包む');
          const resolvedDamageInSameBlock =
            clean.includes('ダメージ') ||
            clean.includes('急所に当たった') ||
            clean.includes('効果はバツグン') ||
            clean.includes('効果はいまひとつ') ||
            clean.includes('たおれた') ||
            clean.includes('倒れた');
          if (!instantSolarBeam && (isChargeText || !resolvedDamageInSameBlock)) {
            actorMon.charging = mvId;
          }
        }

        // Hyper Beam recharge
        if (mvId === 'hyperbeam' && !moveFailed) {
          actorMon.mustRecharge = true;
        }

        // Protect / Detect / Endure
        if (PROTECT_CLASS_MOVES.has(mvId) && !moveFailed) {
          actorMon.protectedThisTurn = true;
        }

        // Self-targeted move effects
        if (!moveFailed) {
          if (mvId === 'rest') {
            actorMon.rest = true;
            actorMon.status = 'slp';
            actorMon.slept = 0;
            actorMon.volatiles.delete('nightmare');
          } else if (mvId === 'substitute') {
            actorMon.volatiles.set('substitute', { key: 'substitute', start_turn: turnNum });
          } else if (mvId === 'mist') {
            actorMon.volatiles.set('mist', { key: 'mist', start_turn: turnNum });
          } else if (mvId === 'focusenergy') {
            actorMon.volatiles.set('focusenergy', { key: 'focusenergy', start_turn: turnNum });
          } else if (mvId === 'defensecurl') {
            actorMon.volatiles.set('defensecurl', { key: 'defensecurl', start_turn: turnNum });
          } else if (mvId === 'minimize') {
            actorMon.volatiles.set('minimize', { key: 'minimize', start_turn: turnNum });
          } else if (mvId === 'bide') {
            actorMon.volatiles.set('bide', { key: 'bide', start_turn: turnNum });
          } else if (mvId === 'rage') {
            actorMon.volatiles.set('rage', { key: 'rage', start_turn: turnNum });
          } else if (mvId === 'destinybond') {
            actorMon.volatiles.set('destinybond', { key: 'destinybond', start_turn: turnNum });
          } else if (mvId === 'furycutter') {
            const prevFc = actorMon.volatiles.get('furycutter');
            const nextCount = prevFc ? Math.min(5, (prevFc.counter || 1) + 1) : 1;
            actorMon.volatiles.set('furycutter', {
              key: 'furycutter',
              start_turn: prevFc ? prevFc.start_turn : turnNum,
              counter: nextCount,
            });
          } else if (mvId === 'transform' && foeSlot !== null && foeMon) {
            pendingTransform[sideIdx] = {
              turn: turnNum,
              actorSlot,
              foeSide: foeIdx,
              foeSlot
            };
          } else if (mvId === 'psychup' && foeMon) {
            actorMon.boosts = [...foeMon.boosts];
          } else if (mvId === 'mimic' && foeMon && foeMon.lastMove) {
            actorMon.mimicOverlay = foeMon.lastMove;
          } else if (mvId === 'curse') {
            const rosterEntry = (previewSides && previewSides[sideIdx] && previewSides[sideIdx][actorSlot]) || null;
            const spId = rosterEntry ? (rosterEntry.speciesId || toId(rosterEntry.species)) : null;
            const isGhost = Boolean(spId && dexData.species[spId]?.types?.includes('Ghost'));
            if (
              (isGhost || clean.includes('のろいをかけた') || clean.includes('呪いをかけた')) &&
              foeMon
            ) {
              foeMon.volatiles.set('curse', { key: 'curse', start_turn: turnNum });
            }
          }
        } else if (mvId === 'furycutter') {
          actorMon.volatiles.delete('furycutter');
        }
      }

      // Side & Field effects (decoupled from actorSlot resolution!)
      if (!moveFailed) {
        if (mvId === 'raindance') {
          info.weather = { key: 'raindance', upkeeps: 0, start_turn: turnNum, _lastUpkeepTurn: 0 };
        } else if (mvId === 'sunnyday') {
          info.weather = { key: 'sunnyday', upkeeps: 0, start_turn: turnNum, _lastUpkeepTurn: 0 };
        } else if (mvId === 'sandstorm') {
          info.weather = { key: 'sandstorm', upkeeps: 0, start_turn: turnNum, _lastUpkeepTurn: 0 };
        } else if (mvId === 'reflect') {
          if (!info.sides[sideIdx].conditions.has('reflect')) {
            info.sides[sideIdx].conditions.set('reflect', turnNum);
          }
        } else if (mvId === 'lightscreen') {
          if (!info.sides[sideIdx].conditions.has('lightscreen')) {
            info.sides[sideIdx].conditions.set('lightscreen', turnNum);
          }
        } else if (mvId === 'safeguard') {
          if (!info.sides[sideIdx].conditions.has('safeguard')) {
            info.sides[sideIdx].conditions.set('safeguard', turnNum);
          }
        } else if (mvId === 'spikes') {
          if (!info.sides[foeIdx].conditions.has('spikes')) {
            info.sides[foeIdx].conditions.set('spikes', turnNum);
          }
        } else if (mvId === 'rapidspin') {
          info.sides[sideIdx].conditions.delete('spikes');
          if (actorMon) {
            actorMon.volatiles.delete('leechseed');
            actorMon.volatiles.delete('partiallytrapped');
          }
        } else if (mvId === 'haze') {
          for (const s of [0, 1]) {
            const a = info.sides[s].activeSlot;
            if (a !== null && info.sides[s].mons[a]) {
              info.sides[s].mons[a].boosts = [0, 0, 0, 0, 0, 0, 0];
              info.sides[s].mons[a].compBrn = false;
              info.sides[s].mons[a].compPar = false;
            }
          }
        } else if (mvId === 'healbell') {
          for (const m of info.sides[sideIdx].mons) {
            if (!m.fainted) cureMonStatus(m);
          }
        } else if (mvId === 'perishsong') {
          for (const s of [0, 1]) {
            const a = info.sides[s].activeSlot;
            if (a !== null && info.sides[s].mons[a] && !info.sides[s].mons[a].volatiles.has('perishsong')) {
              info.sides[s].mons[a].volatiles.set('perishsong', {
                key: 'perishsong',
                start_turn: turnNum,
                counter: 3,
                _lastTickTurn: turnNum,
              });
            }
          }
        }

        // Foe-targeted move volatiles & status effects
        if (foeMon) {
          const sourcePair = actorSlot !== null ? [sideIdx, actorSlot] : undefined;
          if (mvId === 'leechseed') {
            foeMon.volatiles.set('leechseed', {
              key: 'leechseed',
              start_turn: turnNum,
              ...(sourcePair ? { source: sourcePair } : {}),
            });
          } else if (mvId === 'nightmare') {
            foeMon.volatiles.set('nightmare', { key: 'nightmare', start_turn: turnNum });
          } else if (mvId === 'attract') {
            foeMon.volatiles.set('attract', {
              key: 'attract',
              start_turn: turnNum,
              ...(sourcePair ? { source: sourcePair } : {}),
            });
          } else if (mvId === 'foresight') {
            foeMon.volatiles.set('foresight', { key: 'foresight', start_turn: turnNum });
          } else if (mvId === 'encore' && foeMon.lastMove) {
            foeMon.volatiles.set('encore', {
              key: 'encore',
              start_turn: turnNum,
              move: foeMon.lastMove,
            });
          } else if (mvId === 'disable') {
            const disabledMv = blockMoveIds.find(id => id !== 'disable') || foeMon.lastMove;
            if (disabledMv) {
              foeMon.volatiles.set('disable', {
                key: 'disable',
                start_turn: turnNum,
                move: disabledMv,
              });
            }
          } else if (mvId === 'meanlook' || mvId === 'spiderweb') {
            foeMon.volatiles.set('trapped', {
              key: 'trapped',
              start_turn: turnNum,
              ...(sourcePair ? { source: sourcePair } : {}),
            });
          } else if (BIND_CLASS_MOVES.has(mvId)) {
            foeMon.volatiles.set('partiallytrapped', {
              key: 'partiallytrapped',
              start_turn: turnNum,
              move: mvId,
              ...(sourcePair ? { source: sourcePair } : {}),
            });
          } else if (
            mvId === 'confuseray' ||
            mvId === 'supersonic' ||
            mvId === 'sweetkiss' ||
            mvId === 'swagger'
          ) {
            foeMon.volatiles.set('confusion', { key: 'confusion', start_turn: turnNum });
            foeMon.locked = null;
          } else if (mvId === 'toxic' && !foeMon.volatiles.has('substitute')) {
            foeMon.status = 'tox';
            foeMon.toxCounter = 0;
          } else if (mvId === 'spite' && foeMon.lastMove) {
            const spiteMatch = clean.match(/(\d+)\s*(?:減らした|へらした|減った|へった)/);
            const spiteAmt = spiteMatch ? parseInt(spiteMatch[1], 10) : 4;
            foeMon.uses[foeMon.lastMove] = (foeMon.uses[foeMon.lastMove] || 0) + spiteAmt;
          }
        }
      } else if (
        (clean.includes('守った') || clean.includes('まもった') || clean.includes('防いだ') || clean.includes('にはきかない')) &&
        actorMon &&
        actorMon.locked &&
        actorMon.locked.move === 'rollout'
      ) {
        actorMon.locked = null;
      }

      info.events.push({
        type: 'move',
        turn: turnNum,
        side: sideIdx,
        slot: actorSlot,
        move: mvId,
        failed: moveFailed,
        called: isCalledMove,
      });
    }

    const awaitingTransform = pendingTransform[sideIdx];
    if (awaitingTransform) {
      if (isMoveFailureText(clean)) {
        pendingTransform[sideIdx] = null;
      } else if (/(?:\u3078\u3093\u3057\u3093\u3057\u305f|\u5909\u8eab\u3057\u305f)/.test(clean)) {
        const srcMon =
          info.sides[sideIdx].mons[awaitingTransform.actorSlot];
        const targetMon =
          info.sides[awaitingTransform.foeSide].mons[awaitingTransform.foeSlot];
        if (srcMon && targetMon &&
            info.sides[sideIdx].activeSlot === awaitingTransform.actorSlot) {
          srcMon.transformedInto = [
            awaitingTransform.foeSide,
            awaitingTransform.foeSlot
          ];
          srcMon.boosts = [...targetMon.boosts];
        }
        pendingTransform[sideIdx] = null;
      }
    }
    if (blockMoveIds.length === 0 && isMoveFailureText(clean)) {
      pendingCalledMove[sideIdx] = null;
    }
    // 3b. Phaze drag (`ほえる` / `ふきとばし` -> `ひきずりだされた` / `ふきとばされた`)
    if (isDragBlock) {
      const dragClauseMatch = rawClean.match(
        /([^\s！!<>]+?)\s*(?:が|は)\s*(?:(?:戦闘に)?(?:引きずり出された|ひきずりだされた)|吹き飛ばされた|ふきとばされた)/
      );
      const dragTargetText = dragClauseMatch ? dragClauseMatch[1] : clean;
      const hasPhazeMove = blockMoveIds.includes('roar') || blockMoveIds.includes('whirlwind');
      const primarySide = hasPhazeMove ? foeIdx : sideIdx;
      const secondarySide = 1 - primarySide;
      let dragSide = primarySide;
      let dragSlot = findExplicitSlot(primarySide, dragTargetText);
      if (dragSlot === null) {
        const altSlot = findExplicitSlot(secondarySide, dragTargetText);
        if (altSlot !== null) {
          dragSide = secondarySide;
          dragSlot = altSlot;
        }
      }
      if (dragSlot !== null) {
        setActive(dragSide, dragSlot, { isDrag: true, isInitialSendOut: false });
      }
    }

    const resolveLineSubject = (lineText, altSide = null) => {
      if (altSide !== null) {
        const exp = findExplicitSlot(altSide, lineText);
        const slot = exp !== null ? exp : info.sides[altSide].activeSlot;
        return {
          side: altSide,
          slot,
          mon: slot !== null ? (info.sides[altSide].mons[slot] || null) : null,
        };
      }
      const foeExp = findExplicitSlot(foeIdx, lineText);
      const ownExp = findExplicitSlot(sideIdx, lineText);
      if (foeExp !== null && ownExp === null) {
        return {
          side: foeIdx,
          slot: foeExp,
          mon: info.sides[foeIdx].mons[foeExp] || null,
        };
      }
      const sSlot = ownExp ?? info.sides[sideIdx].activeSlot;
      return {
        side: sideIdx,
        slot: sSlot,
        mon: sSlot !== null ? (info.sides[sideIdx].mons[sSlot] || null) : null,
      };
    };

    // 4. Stat boost comments `<!--actorSide targetSide slot statIdx delta ...//-->`
    let parsedCommentBoost = false;
    for (const cm of block.matchAll(/<!--([\d\s\-]+)\/\/-->/g)) {
      const nums = cm[1].trim().split(/\s+/).map(Number).filter(n => !Number.isNaN(n));
      if (nums.length >= 5 && (nums[1] === 0 || nums[1] === 1) && nums[3] >= 0 && nums[3] <= 6) {
        const targetSide = nums[1];
        const statIdx = nums[3];
        const delta = nums[4];
        const aSlot = info.sides[targetSide].activeSlot;
        if (aSlot !== null && info.sides[targetSide].mons[aSlot] && delta !== 0 && Math.abs(delta) <= 6) {
          applyBoostDelta(info.sides[targetSide].mons[aSlot], statIdx, delta);
          parsedCommentBoost = true;
        }
      }
    }

    // 5. Line-by-line inspection (`<BR>` clauses) for items, stat-change text, status, volatiles, and faints
    const rawLines = block.split(/<BR\s*\/?>|\n/i);
    for (const rawLine of rawLines) {
      const lineClean = rawLine.replace(/<[^>]*>/g, '').replace(/\s+/g, '').trim();
      if (!lineClean) continue;
      const altM = rawLine.match(/alt="([^"]*)"/i);
      const alt = altM ? altM[1].replace(/&nbsp;/g, ' ').trim() : '';
      let altSide = null;
      if (alt) {
        const altNorm = normJp(alt).replace(/の$/, '');
        if (p1Name && altNorm === normJp(p1Name)) altSide = 0;
        else if (p2Name && altNorm === normJp(p2Name)) altSide = 1;
      }
      const subj = resolveLineSubject(lineClean, altSide);
      const subjMon = subj.mon;

      // 5a. Item reveal & consumption cues (`たべのこし`, berries, Focus Band, Quick Claw)
      if (subjMon) {
        if (lineClean.includes('たべのこし') || lineClean.includes('食べ残し')) {
          if (!subjMon.itemGained) subjMon.itemOriginal = subjMon.itemOriginal || 'leftovers';
          subjMon.itemCurrent = 'leftovers';
        } else if (lineClean.includes('きあいのハチマキ')) {
          if (!subjMon.itemGained) subjMon.itemOriginal = subjMon.itemOriginal || 'focusband';
          subjMon.itemCurrent = 'focusband';
        } else if (lineClean.includes('せんせいのツメ')) {
          if (!subjMon.itemGained) subjMon.itemOriginal = subjMon.itemOriginal || 'quickclaw';
          subjMon.itemCurrent = 'quickclaw';
        } else {
          for (const [jpBerry, berryId] of CONSUMED_BERRIES_JP) {
            if (lineClean.includes(jpBerry)) {
              if (!subjMon.itemGained) subjMon.itemOriginal = subjMon.itemOriginal || berryId;
              subjMon.itemCurrent = '';
              if (berryId === 'bitterberry') {
                subjMon.volatiles.delete('confusion');
              } else if (berryId === 'miracleberry') {
                subjMon.volatiles.delete('confusion');
                cureMonStatus(subjMon);
              } else if (
                berryId === 'mintberry' ||
                berryId === 'przcureberry' ||
                berryId === 'iceberry' ||
                berryId === 'burntberry' ||
                berryId === 'psncureberry'
              ) {
                cureMonStatus(subjMon);
              } else if (berryId === 'mysteryberry') {
                for (const [uMv, uCnt] of Object.entries(subjMon.uses)) {
                  if (uCnt > 0) {
                    subjMon.uses[uMv] = Math.max(0, uCnt - 5);
                    break;
                  }
                }
              }
              break;
            }
          }
        }
      }

      // 5b. Thief (`どろぼう`) item steal cues
      if (lineClean.includes('を奪った') || lineClean.includes('をうばった')) {
        const actorSlot = info.sides[sideIdx].activeSlot;
        const foeSlot = info.sides[foeIdx].activeSlot;
        for (const [jpName, itemId] of Object.entries(JP_TO_ITEM_ID)) {
          if (jpName && lineClean.includes(jpName)) {
            if (foeSlot !== null && info.sides[foeIdx].mons[foeSlot]) {
              const victim = info.sides[foeIdx].mons[foeSlot];
              if (!victim.itemGained) victim.itemOriginal = victim.itemOriginal || itemId;
              victim.itemCurrent = '';
            }
            if (actorSlot !== null && info.sides[sideIdx].mons[actorSlot]) {
              const thief = info.sides[sideIdx].mons[actorSlot];
              thief.itemCurrent = itemId;
              thief.itemGained = true;
            }
            break;
          }
        }
      }

      // 5c. Text fallback for stat boosts/drops when HTML comments are absent
      if (
        !parsedCommentBoost &&
        subjMon &&
        !blockMoveIds.includes('bellydrum') &&
        (lineClean.includes('あがった') ||
          lineClean.includes('さがった') ||
          lineClean.includes('上がった') ||
          lineClean.includes('下がった'))
      ) {
        const statNames = [
          ['こうげき', 0],
          ['攻撃', 0],
          ['ぼうぎょ', 1],
          ['防御', 1],
          ['とくこう', 2],
          ['特攻', 2],
          ['とくぼう', 3],
          ['特防', 3],
          ['すばやさ', 4],
          ['素早さ', 4],
          ['めいちゅう', 5],
          ['命中', 5],
          ['かいひ', 6],
          ['回避', 6],
        ];
        // Classify magnitude from the battle message, not the move name.
        const stageLine = normJp(lineClean).replace(/[\u301c\uff5e]/g, '\u30fc');
        const delta =
          /(?:\u3055\u3044\u3060\u3044\u307e\u3067|\u6700\u5927\u307e\u3067)/.test(stageLine) ? 12
          : /\u3050\u3050?\u30fc\u3093\u3068(?:\u3042\u304c\u3063\u305f|\u4e0a\u304c\u3063\u305f)/.test(stageLine) ? 2
          : /(?:\u3042\u304c\u3063\u305f|\u4e0a\u304c\u3063\u305f)/.test(stageLine) ? 1
          : /\u304c\u304f(?:\u3063|\u30fc\u3093)\u3068(?:\u3055\u304c\u3063\u305f|\u4e0b\u304c\u3063\u305f)/.test(stageLine) ? -2
          : -1;
        const appliedStats = new Set();
        for (const [jpStat, idx] of statNames) {
          if (lineClean.includes(jpStat) && !appliedStats.has(idx)) {
            appliedStats.add(idx);
            applyBoostDelta(subjMon, idx, delta);
          }
        }
      }

      // 5d. Status infliction, cure, & residual damage cues
      if (subjMon && !subjMon.fainted) {
        if (
          lineClean.includes('もうどくをあびた') ||
          lineClean.includes('猛毒をあびた') ||
          lineClean.includes('\u731b\u6bd2\u306b\u304a\u304b\u3055\u308c\u305f') ||
          lineClean.includes('\u3082\u3046\u3069\u304f\u306b\u304a\u304b\u3055\u308c\u305f') ||
          lineClean.includes('もうどく状態')
        ) {
          subjMon.status = 'tox';
          subjMon.toxCounter = 0;
        } else if (
          lineClean.includes('どくをあびた') ||
          lineClean.includes('毒をあびた') ||
          lineClean.includes('どく状態になった') ||
          lineClean.includes('毒におかされた') ||
          lineClean.includes('どくにおかされた')
        ) {
          subjMon.status = 'psn';
        } else if (
          lineClean.includes('やけどを負った') ||
          lineClean.includes('火傷を負った') ||
          lineClean.includes('やけどをおった') ||
          lineClean.includes('火傷をおった') ||
          lineClean.includes('\u706b\u50b7\u3057\u305f') ||
          lineClean.includes('\u3084\u3051\u3069\u3057\u305f') ||
          lineClean.includes('やけど状態になった')
        ) {
          subjMon.status = 'brn';
          subjMon.compBrn = true;
        } else if (
          lineClean.includes('まひして技が出にくくなった') ||
          lineClean.includes('麻痺して技が出にくくなった') ||
          lineClean.includes('まひ状態になった') ||
          lineClean.includes('まひした') ||
          lineClean.includes('麻痺した')
        ) {
          subjMon.status = 'par';
          subjMon.compPar = true;
        } else if (
          (lineClean.includes('\u7720\u3063\u3066\u3057\u307e\u3063\u305f') ||
           lineClean.includes('\u306d\u3080\u3063\u3066\u3057\u307e\u3063\u305f') ||
           lineClean.includes('\u7720\u308a\u306f\u3058\u3081\u305f') ||
           lineClean.includes('\u306d\u3080\u308a\u306f\u3058\u3081\u305f')) &&
          !blockMoveIds.includes('rest')
        ) {
          subjMon.status = 'slp';
          subjMon.rest = false;
          subjMon.slept = 0;
        } else if (
          lineClean.includes('凍りついた') ||
          lineClean.includes('こおりついた') ||
          lineClean.includes('凍ってしまった') ||
          lineClean.includes('こおりづけになった') ||
          lineClean.includes('凍りづけになった')
        ) {
          subjMon.status = 'frz';
        }

        // Wake / Thaw cures
        if (
          lineClean.includes('目を覚ました') ||
          lineClean.includes('めをさました') ||
          lineClean.includes('氷が溶けた') ||
          lineClean.includes('氷がとけた') ||
          lineClean.includes('こおりがとけた') ||
          lineClean.includes('こおりが溶けた')
        ) {
          cureMonStatus(subjMon);
        }

        // Residual poison/burn damage tick increments Stadium `toxCounter` (`comp_res`)
        if (
          lineClean.includes('どくのダメージ') ||
          lineClean.includes('毒のダメージ') ||
          lineClean.includes('もうどくのダメージ') ||
          lineClean.includes('猛毒のダメージ') ||
          lineClean.includes('やけどのダメージ') ||
          lineClean.includes('火傷のダメージ')
        ) {
          info.upkeepThisTurn = true;
          if (subjMon.toxCounter !== null && subjMon.toxCounter !== undefined) {
            subjMon.toxCounter += 1;
          }
        }
      }

      // 5e. Volatile start/end cues & Perish Song countdown
      if (subjMon && !subjMon.fainted) {
        if (lineClean.includes('こんらんした') || lineClean.includes('混乱した')) {
          subjMon.volatiles.set('confusion', { key: 'confusion', start_turn: turnNum });
          subjMon.locked = null;
        } else if (
          lineClean.includes('こんらんがとけた') ||
          lineClean.includes('こんらんがなおった') ||
          lineClean.includes('混乱が解けた') ||
          lineClean.includes('混乱が治った') ||
          lineClean.includes('正気に戻った') ||
          lineClean.includes('しょうきにもどった')
        ) {
          subjMon.volatiles.delete('confusion');
        }
        if (
          lineClean.includes('みがわりが消えた') ||
          lineClean.includes('みがわりは消えてしまった') ||
          lineClean.includes('みがわりが壊れた') ||
          lineClean.includes('身代わりが消えた') ||
          lineClean.includes('分身はきえてしまった') ||
          lineClean.includes('分身は消えてしまった')
        ) {
          subjMon.volatiles.delete('substitute');
        }
        // The target may receive Encore without a recognized preceding move.
        if (/\u30a2\u30f3\u30b3\u30fc\u30eb\u3092(?:\u3046\u3051\u305f|\u53d7\u3051\u305f)/.test(lineClean) &&
            !subjMon.volatiles.has('encore')) {
          subjMon.volatiles.set('encore', {
            key: 'encore',
            start_turn: turnNum,
            ...(subjMon.lastMove ? { move: subjMon.lastMove } : {}),
          });
        }
        if (lineClean.includes('\u30a2\u30f3\u30b3\u30fc\u30eb\u304c\u3068\u3051\u305f') ||
            lineClean.includes('\u30a2\u30f3\u30b3\u30fc\u30eb\u304c\u7d42\u308f\u3063\u305f') ||
            lineClean.includes('\u30a2\u30f3\u30b3\u30fc\u30eb\u72b6\u614b\u304c\u3068\u3051\u305f')) {
          subjMon.volatiles.delete('encore');
        }
        if (lineClean.includes('かなしばりがとけた') || lineClean.includes('かなしばりから解放された')) {
          subjMon.volatiles.delete('disable');
        }
        if (
          lineClean.includes('締め付けから解放された') ||
          lineClean.includes('しめつけから解放された') ||
          lineClean.includes('から解放された') ||
          lineClean.includes('渦から解放された')
        ) {
          subjMon.volatiles.delete('partiallytrapped');
        }

        const perishMatch = lineClean.match(/(?:ほろびのカウント|滅びのカウント|カウント)が([0-3])/);
        if (perishMatch) {
          info.upkeepThisTurn = true;
          const cnt = parseInt(perishMatch[1], 10);
          if (cnt <= 0) {
            subjMon.volatiles.delete('perishsong');
          } else {
            const prevPs = subjMon.volatiles.get('perishsong');
            subjMon.volatiles.set('perishsong', {
              key: 'perishsong',
              start_turn: prevPs ? prevPs.start_turn : turnNum,
              counter: cnt,
              _lastTickTurn: turnNum,
            });
          }
        }
      }

      // 5f. Faint detection (`|faint|`) across either side mentioned on this line
      const isHpZero = /(?:HP|ＨＰ)\s*\d+\s*→\s*0\b/i.test(lineClean);
      const isFaintLine = lineClean.includes('たおれた') || lineClean.includes('倒れた');
      if (
        (isFaintLine || isHpZero) &&
        subj.slot !== null &&
        subjMon &&
        !subjMon.fainted
      ) {
        subjMon.appeared = true;
        subjMon.appearCount = Math.max(1, subjMon.appearCount || 1);
        subjMon.fainted = true;
        subjMon.status = '';
        subjMon.rest = false;
        subjMon.slept = 0;
        clearOnExit(subjMon);
        clearSourcedVols(subj.side, subj.slot);
        info.sides[subj.side].faintedThisTurn = subj.slot;
        info.events.push({ type: 'faint', turn: turnNum, side: subj.side, slot: subj.slot });
      }

      // 5g. Check whether this `<BR>` line was recognized by any state rule or known battle narrative
      const isMoveLine =
        blockMoveIds.length > 0 &&
        Object.keys(JP_TO_MOVE_ID).some(jpMv => jpMv && normJp(lineClean).includes(jpMv));
      const isBerryLine = CONSUMED_BERRIES_JP.some(([jpBerry]) => lineClean.includes(jpBerry));
      const isKnownStateOrNarrative =
        isMoveLine ||
        isBerryLine ||
        isMoveFailureText(lineClean) ||
        /を繰り出した|をくりだした|ゆけっ|いけっ|出てこい|引っ込めた|ひっこめた|戻れ|もどれ|交代/.test(lineClean) ||
        /引きずり出された|ひきずりだされた|吹き飛ばされた|ふきとばされた/.test(lineClean) ||
        /眠っている|ねむっている|眠りはじめた|ねむりはじめた|眠っていて|ねむっていて|反動で動けない|はんどうでうごけない|しびれて動けない|しびれてうごけない|痺れて動けない|痺れてうごけない|凍ってしまって|こおってしまって|凍っていて|こおっていて|ひるんで|メロメロ|かなしばり|わけもわからず/.test(lineClean) ||
        /空高く飛び上がった|穴を掘った|地下深く潜り込んだ|光を吸収|ひかりを吸収|頭を引っ込めた|頭をひっこめた|激しい風に包まれた|空気がうずまく|激しい光をまとった|激しい光が包む|力をためて/.test(lineClean) ||
        /たべのこし|食べ残し|きあいのハチマキ|せんせいのツメ|を奪った|をうばった/.test(lineClean) ||
        /あがった|さがった|上がった|下がった|あがらない|さがらない|上がらない|下がらない/.test(lineClean) ||
        /もうどく|猛毒|どくをあびた|毒をあびた|毒におかされた|どくにおかされた|どく状態|やけど|火傷|まひして|麻痺して|まひした|麻痺した|まひ状態|眠ってしまった|ねむってしまった|けんこうになって|健康になって|凍りついた|こおりついた|こおりづけになった|凍ってしまった|目を覚ました|めをさました|氷が溶けた|こおりがとけた|氷がとけた|正気に戻った|しょうきにもどった|どくのダメージ|毒のダメージ/.test(lineClean) ||
        /こんらん|混乱|みがわり|身代わり|分身|アンコール|解放された|カウントが|しめつけ|締め付け|渦の中に|うずのなかに|炎の渦|ほのおのうず|からみついて|はさまれた/.test(lineClean) ||
        /たおれた|倒れた|みちづれ|道連れ/.test(lineClean) ||
        /リフレクター|ひかりのかべ|光の壁|しんぴのまもり|神秘の守り|しんぴのヴェール|神秘のヴェール|しろいきり|白い霧|くろいきり|黒い霧|すべての能力が元に戻った|まきびし|すずの音|鈴の音|ほろびのうた|滅びの歌|タネが|種が|あくむ|悪夢|みやぶった|見破った|を封じた|にげられなくなった|逃げられなくなった|もうにげられない|うらみ|ＰＰを|PPを|へんしん|変身|暗示|あんじ|ものまね|のろい|呪い|のろわれている|いかりのボルテージ|がまん|まるくなった|ちいさくなった|はりきって|張り切って/.test(lineClean) ||
        /雨が降り|あめがふり|大雨になった|雨はやんだ|あめはやんだ|雨が降り止んだ|日差しが|ひざしが|砂嵐|すなあらし|すなあらしがふきあれる|すなあらしがおさまった|砂嵐がおさまった/.test(lineClean) ||
        /ダメージ|急所に当たった|きゅうしょにあたった|急所にあたった|効果はバツグン|こうかはばつぐん|効果抜群|こうかばつぐん|効果はいまひとつ|こうかはいまひとつ|効果はイマイチ|こうかはイマイチ|回当たった|かいあたった|体力を回復|たいりょくをかいふく|回復した|体力を吸い取った|たいりょくをすいとった|たいりょくをうばう|わかちあった|体力がいっぱい|たいりょくがいっぱい|反動を受けた|はんどうをうけた|反動をうけた|はんどうをうけた|は守っている|はまもっている|まもりの体勢|こらえた|こらえる体勢|こらえている|はねている|ひるんだ|何も起こらない|なにもおこらない|しかし決まらなかった|すでに|強くなった|最大まであげた|たねをうえつけた|はかいのいでんし|たりなかった|交換できるポケモンがいなかった|タイプを自分に貼り付けた|激突した|一撃必殺|WIN!|ＷＩＮ！|LOSE|ＬＯＳＥ|DRAW|ＤＲＡＷ|戻る|HP\s*\d+\s*→|ＨＰ\s*\d+\s*→|ほろんでしまう/.test(lineClean);

      if (!isKnownStateOrNarrative && lineClean.length > 0) {
        info.unrecognizedLines.push({
          turn: turnNum,
          side: sideIdx,
          align,
          line: lineClean,
        });
      }
    }

    // 6. Side condition end cues (Reflect, Light Screen, Safeguard)
    if (
      clean.includes('リフレクター') &&
      (clean.includes('消えた') || clean.includes('きえた') || clean.includes('なくなった') || clean.includes('とけた') || (clean.includes('切れた') || clean.includes('\u304d\u308c\u305f')))
    ) {
      info.sides[sideIdx].conditions.delete('reflect');
    }
    if (
      (clean.includes('ひかりのかべ') || clean.includes('光の壁')) &&
      (clean.includes('消えた') || clean.includes('きえた') || clean.includes('なくなった') || clean.includes('とけた') || (clean.includes('切れた') || clean.includes('\u304d\u308c\u305f')))
    ) {
      info.sides[sideIdx].conditions.delete('lightscreen');
    }
    if (
      (clean.includes('しんぴのまもり') || clean.includes('神秘の守り')) &&
      (clean.includes('消えた') || clean.includes('きえた') || clean.includes('なくなった') || clean.includes('とけた') || (clean.includes('切れた') || clean.includes('\u304d\u308c\u305f')) || clean.includes('解けた'))
    ) {
      info.sides[sideIdx].conditions.delete('safeguard');
    }

    // 7. Weather upkeep & end cues
    if (
      clean.includes('雨が降り続いている') ||
      clean.includes('あめがふりつづいている') ||
      clean.includes('日差しが強い') ||
      clean.includes('ひざしがつよい') ||
      clean.includes('砂嵐が吹き荒れている') ||
      clean.includes('すなあらしがふきあれている') ||
      clean.includes('砂嵐でダメージ')
    ) {
      info.upkeepThisTurn = true;
      if (info.weather && info.weather._lastUpkeepTurn !== turnNum) {
        info.weather.upkeeps = (info.weather.upkeeps || 0) + 1;
        info.weather._lastUpkeepTurn = turnNum;
        if (info.weather.upkeeps >= 5) {
          info.weather = null;
        }
      }
    }
    if (
      clean.includes('雨はやんだ') ||
      clean.includes('あめはやんだ') ||
      clean.includes('日差しが戻った') ||
      clean.includes('ひざしがもどった') ||
      clean.includes('砂嵐はおさまった') ||
      clean.includes('すなあらしはおさまった')
    ) {
      info.weather = null;
    }
  }

  for (const side of [0,1]) {
    if (pendingTransform[side]) {
      info.unrecognizedLines.push({
        turn: info.turn,
        side,
        align: side === 0 ? 'left' : 'right',
        line: 'Transform outcome not yet confirmed by LLOG'
      });
    }
  }
  return info;
}

/**
 * Convert a `parseLlogHtml` tracker object (containing `Map` and `Set` instances)
 * into a plain JSON-serializable object for raw trace persistence and parity auditing.
 */
function serializeLlogTracker(llog) {
  if (!llog) return null;
  return {
    turn: llog.turn,
    hrCount: llog.hrCount,
    upkeepThisTurn: Boolean(llog.upkeepThisTurn),
    weather: llog.weather ? { ...llog.weather } : null,
    events: Array.isArray(llog.events) ? llog.events.map(e => ({ ...e })) : [],
    unrecognizedLines: Array.isArray(llog.unrecognizedLines)
      ? llog.unrecognizedLines.map(u => ({ ...u }))
      : [],
    sides: (llog.sides || []).map(side => ({
      activeSlot: side.activeSlot,
      conditions:
        side.conditions instanceof Map
          ? Object.fromEntries(side.conditions.entries())
          : { ...(side.conditions || {}) },
      pendingBp: Boolean(side.pendingBp),
      actedThisTurn: Boolean(side.actedThisTurn),
      faintedThisTurn: side.faintedThisTurn ?? null,
      faintedLastTurn: side.faintedLastTurn ?? null,
      lastMove: side.lastMove ?? null,
      mons: (side.mons || []).map(m => ({
        appeared: Boolean(m.appeared),
        appearCount: m.appearCount || 0,
        switchInTurn: m.switchInTurn || 0,
        active: Boolean(m.active),
        fainted: Boolean(m.fainted),
        status: m.status || '',
        rest: Boolean(m.rest),
        slept: m.slept || 0,
        toxCounter: m.toxCounter ?? null,
        compBrn: Boolean(m.compBrn),
        compPar: Boolean(m.compPar),
        boosts: [...(m.boosts || [0, 0, 0, 0, 0, 0, 0])],
        uses: { ...(m.uses || {}) },
        revealedMoves:
          m.revealedMoves instanceof Set
            ? [...m.revealedMoves]
            : Array.isArray(m.revealedMoves)
              ? [...m.revealedMoves]
              : [],
        itemOriginal: m.itemOriginal ?? null,
        itemCurrent: m.itemCurrent ?? null,
        itemGained: Boolean(m.itemGained),
        volatiles:
          m.volatiles instanceof Map
            ? [...m.volatiles.values()].map(v => ({ ...v }))
            : Array.isArray(m.volatiles)
              ? m.volatiles.map(v => ({ ...v }))
              : [],
        locked: m.locked ? { ...m.locked } : null,
        charging: m.charging ?? null,
        mustRecharge: Boolean(m.mustRecharge),
        transformedInto: m.transformedInto ? [...m.transformedInto] : null,
        mimicOverlay: m.mimicOverlay ?? null,
        lastMove: m.lastMove ?? null,
        stallStreak: m.stallStreak || 0,
        lastProtectTurn: m.lastProtectTurn || 0,
        protectedThisTurn: Boolean(m.protectedThisTurn),
      })),
    })),
  };
}

/**
 * Merge parsed LLOG observations (and optional top status table) into persistent
 * `seenFoeMons` so mid-game and end-of-game observations (`Selfdestruct`, `fainted`,
 * `uses`, `revealedMoves`, `itemOriginal`, `noLeftovers`) are never lost.
 */
function mergeLlogIntoSeenFoeMons({
  llog,
  foeSide,
  seenFoeMons,
  previewSides,
  battleTurn = null,
  turnNum = 1,
  isForceSwitch = false,
  lastFoeActiveSlot = null,
  onLeakLog = null,
}) {
  if (!Array.isArray(seenFoeMons) || seenFoeMons.length === 0) {
    return { lastFoeActiveSlot, llogFoeMons: llog?.sides?.[foeSide]?.mons || null };
  }

  const foePreview = (previewSides && previewSides[foeSide]) || [];
  const foeTop = battleTurn && battleTurn.sides ? battleTurn.sides[foeSide] : null;
  let foeActiveSlot = null;
  if (foeTop && foeTop.speciesId) {
    foeActiveSlot = foePreview.findIndex(m => m.speciesId === foeTop.speciesId);
  }
  if (foeActiveSlot === null || foeActiveSlot < 0) {
    foeActiveSlot = llog?.sides?.[foeSide]?.activeSlot ?? null;
  }

  for (let i = 0; i < seenFoeMons.length; i++) {
    const seen = seenFoeMons[i];
    const llogMon = llog?.sides?.[foeSide]?.mons?.[i] || null;
    if (!seen || !llogMon) continue;

    if (llogMon.appeared) seen.appeared = true;
    if (llogMon.fainted) {
      seen.fainted = true;
      seen.hp = 0;
      seen.status = '';
    } else if (!foeTop && llogMon.status) {
      seen.status = llogMon.status;
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

    if (llogMon.uses && Object.keys(llogMon.uses).length > 0) {
      seen.uses = { ...(seen.uses || {}) };
      for (const [mvId, cnt] of Object.entries(llogMon.uses)) {
        seen.uses[mvId] = Math.max(seen.uses[mvId] || 0, cnt);
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
      activeSeen.hp = activeLlog?.fainted ? 0 : foeTop.hp;
      activeSeen.fainted = Boolean(activeLlog?.fainted || foeTop.hp <= 0);
      const rawTopStatus = foeTop.status || '';
      activeSeen.status = activeSeen.fainted
        ? ''
        : (rawTopStatus === 'psn' && activeLlog?.status === 'tox' ? 'tox' : rawTopStatus);

      if (
        !isForceSwitch &&
        turnNum >= 2 &&
        activeSeen.itemFlag &&
        !activeSeen.itemOriginal &&
        activeSeen.maxHp &&
        activeSeen.hp > 0 &&
        activeSeen.hp < activeSeen.maxHp
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
            if (typeof onLeakLog === 'function') {
              onLeakLog(
                `Leak inference: Foe ${activeSeen.species} (${activeSeen.hp}/${activeSeen.maxHp}) lacks Leftovers.`
              );
            }
          }
        }
      }
    }
    lastFoeActiveSlot = foeActiveSlot;
  }

  return {
    lastFoeActiveSlot,
    llogFoeMons: llog?.sides?.[foeSide]?.mons || null,
  };
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
        gender: pm.gender ?? 'M',
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
        gender: pm.gender ?? bestTemplate?.gender ?? 'M',
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

  // Synchronize LLOG tracker turn state with the decision point:
  // - On a normal move request (`!forceSwitch`), `turn` is the new turn about to be played,
  //   so if `llog` finished on an earlier turn, advance `llog` across the turn boundary.
  // - On a forced switch (`forceSwitch`), if `llog` has `<HR>` boundaries, we are still inside
  //   `llog.turn` (either mid-turn Baton Pass or end-of-turn faint replacement before next `|turn|`).
  let effectiveTurn = teamPreview ? 0 : Math.max(1, turn || 1);
  if (!teamPreview && llog) {
    if (forceSwitch) {
      if ((llog.hrCount || 0) >= 1) {
        effectiveTurn = Math.max(1, llog.turn || 1);
      } else if (effectiveTurn > (llog.turn || 1)) {
        advanceLlogTrackerToTurn(llog, effectiveTurn);
      }
    } else {
      effectiveTurn = Math.max(effectiveTurn, llog.turn || 1);
      if (effectiveTurn > (llog.turn || 1)) {
        advanceLlogTrackerToTurn(llog, effectiveTurn);
      }
    }
  }

  // Expire any timed side conditions or weather that have reached 5 turns at `effectiveTurn`
  if (!teamPreview && llog) {
    if (llog.weather) {
      const wStart = llog.weather.start_turn || 1;
      if ((llog.weather.upkeeps || 0) >= 5 || (effectiveTurn - wStart) >= 5) {
        llog.weather = null;
      }
    }
    for (const s of [0, 1]) {
      const conds = llog.sides?.[s]?.conditions;
      if (conds) {
        for (const [ck, cStart] of [...conds.entries()]) {
          if (TIMED_SIDE_CONDITIONS.has(ck) && (effectiveTurn - cStart) >= 5) {
            conds.delete(ck);
          }
        }
      }
    }
  }

  const cleanWeather =
    !teamPreview && llog && llog.weather
      ? {
          key: llog.weather.key,
          upkeeps: Math.max(0, Math.min(4, llog.weather.upkeeps || 0)),
        }
      : null;

  const spec = {
    schema: 'nc2000-position-v1',
    side: mySide,
    turn: effectiveTurn,
    upkeep_this_turn: Boolean(!teamPreview && llog?.upkeepThisTurn),
    own_sets: ownSets,
    sides: [
      {
        mons: [],
        active: null,
        conditions: [],
        party: [],
        pending_bp: false,
        acted_this_turn: false,
        fainted_this_turn: null,
        fainted_last_turn: null,
        last_move: null,
      },
      {
        mons: [],
        active: null,
        conditions: [],
        party: [],
        pending_bp: false,
        acted_this_turn: false,
        fainted_this_turn: null,
        fainted_last_turn: null,
        last_move: null,
      },
    ],
    weather: cleanWeather,
    team_preview: Boolean(teamPreview),
    force_switch: Boolean(forceSwitch),
    trapped: false,
  };

  // Populate both sides' 6-mon rosters
  for (const s of [0, 1]) {
    const isMe = s === mySide;
    const llogSide = llog && llog.sides ? llog.sides[s] : null;
    if (!teamPreview && llogSide) {
      spec.sides[s].pending_bp = Boolean(llogSide.pendingBp);
      spec.sides[s].acted_this_turn = Boolean(llogSide.actedThisTurn);
      spec.sides[s].fainted_this_turn =
        llogSide.faintedThisTurn !== undefined ? llogSide.faintedThisTurn : null;
      spec.sides[s].fainted_last_turn =
        llogSide.faintedLastTurn !== undefined ? llogSide.faintedLastTurn : null;
      spec.sides[s].last_move =
        llogSide.lastMove && dexData.moves[llogSide.lastMove] ? llogSide.lastMove : null;
    }

    const previewMons = (previewSides && previewSides[s] && previewSides[s].length === 6)
      ? previewSides[s]
      : ownSets.map(x => ({
          species: x.species,
          speciesId: toId(x.species),
          level: x.level || 55,
          gender: x.gender ?? 'M',
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
      const llogMon = llogSide && llogSide.mons[i] ? llogSide.mons[i] : null;
      const seenFoe = !isMe && seenFoeMons && seenFoeMons[i] ? seenFoeMons[i] : null;

      const speciesName = ownSet
        ? ownSet.species
        : (dexData.species[pm.speciesId]?.name || pm.species || 'Pikachu');
      const level = ownSet ? (ownSet.level || 55) : (pm.level || 55);
      const gender = pm.gender ?? (ownSet?.gender ?? 'M');
      const itemFlag = isMe ? Boolean(ownSet?.item) : Boolean(pm.item_flag);

      const usesMap = new Map();
      const revealedSet = new Set();
      if (llogMon) {
        for (const [mvId, count] of Object.entries(llogMon.uses || {})) {
          if (dexData.moves[mvId] && count > 0) {
            usesMap.set(mvId, count);
          }
        }
        for (const mvId of llogMon.revealedMoves || []) {
          if (dexData.moves[mvId]) {
            revealedSet.add(mvId);
          }
        }
      }
      if (seenFoe) {
        if (seenFoe.uses) {
          for (const [mvId, count] of Object.entries(seenFoe.uses)) {
            if (dexData.moves[mvId] && count > 0) {
              usesMap.set(mvId, Math.max(usesMap.get(mvId) || 0, count));
            }
          }
        }
        if (seenFoe.revealedMoves) {
          for (const mvId of seenFoe.revealedMoves) {
            if (dexData.moves[mvId]) {
              revealedSet.add(mvId);
            }
          }
        }
      }
      const uses = [...usesMap.entries()].map(([move, n]) => ({ move, n }));
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

      const cleanVolatiles = [];
      if (!teamPreview && !fainted && llogMon?.volatiles) {
        for (const v of llogMon.volatiles.values()) {
          if (!v || !v.key) continue;
          const cv = {
            key: v.key,
            start_turn: Math.max(0, v.start_turn || 0),
          };
          if (v.move && dexData.moves[v.move]) cv.move = v.move;
          if (Array.isArray(v.source) && v.source.length === 2) cv.source = v.source;
          if (v.counter !== undefined && v.counter !== null) cv.counter = v.counter;
          cleanVolatiles.push(cv);
        }
      }

      const monEntry = {
        species: speciesName,
        level,
        gender,
        name: '',
        item_flag: itemFlag,
        appeared,
        appear_count: !teamPreview ? Math.max(appeared ? 1 : 0, llogMon?.appearCount || 0) : 0,
        switch_in_turn: !teamPreview ? Math.max(0, llogMon?.switchInTurn || 0) : 0,
        active: false,
        fainted,
        hp_num: hpNum,
        hp_den: hpDen,
        hp_exact: null,
        status: fainted ? '' : status,
        rest: Boolean(!teamPreview && !fainted && llogMon?.rest),
        slept: !teamPreview && !fainted ? Math.max(0, llogMon?.slept || 0) : 0,
        tox_counter:
          !teamPreview && !fainted && llogMon?.toxCounter !== null && llogMon?.toxCounter !== undefined
            ? llogMon.toxCounter
            : null,
        comp_brn: Boolean(!teamPreview && !fainted && llogMon?.compBrn),
        comp_par: Boolean(!teamPreview && !fainted && llogMon?.compPar),
        boosts: !teamPreview && !fainted && llogMon?.boosts ? [...llogMon.boosts] : [0, 0, 0, 0, 0, 0, 0],
        volatiles: cleanVolatiles,
        uses,
        locked:
          !teamPreview && !fainted && llogMon?.locked && dexData.moves[llogMon.locked.move]
            ? { move: llogMon.locked.move, n: llogMon.locked.n || 1 }
            : null,
        charging:
          !teamPreview && !fainted && llogMon?.charging && dexData.moves[llogMon.charging]
            ? llogMon.charging
            : null,
        must_recharge: Boolean(!teamPreview && !fainted && llogMon?.mustRecharge),
        transformed_into:
          !teamPreview && !fainted && Array.isArray(llogMon?.transformedInto)
            ? llogMon.transformedInto
            : null,
        mimic_overlay:
          !teamPreview && !fainted && llogMon?.mimicOverlay && dexData.moves[llogMon.mimicOverlay]
            ? llogMon.mimicOverlay
            : null,
        last_move:
          !teamPreview && !fainted && llogMon?.lastMove && dexData.moves[llogMon.lastMove]
            ? llogMon.lastMove
            : null,
        stall_streak: !teamPreview && !fainted ? Math.max(0, llogMon?.stallStreak || 0) : 0,
        last_protect_turn: !teamPreview && !fainted ? Math.max(0, llogMon?.lastProtectTurn || 0) : 0,
        protected_this_turn: Boolean(!teamPreview && !fainted && llogMon?.protectedThisTurn),
        revealed_moves: isMe ? [] : revealedMoves,
      };

      // Attach Castle item reveal & Hidden Power DV mask on opponent mons
      if (!teamPreview && !isMe) {
        const itemOrig = seenFoe?.itemOriginal || llogMon?.itemOriginal || null;
        const itemCur = seenFoe?.itemCurrent !== undefined && seenFoe?.itemCurrent !== null
          ? seenFoe.itemCurrent
          : (llogMon?.itemCurrent !== undefined && llogMon?.itemCurrent !== null ? llogMon.itemCurrent : null);
        if (llogMon?.itemGained) {
          monEntry.item_gained = true;
        }
        if (itemOrig && itemFlag) {
          monEntry.item_original = { known: true, item: itemOrig };
          const curStr = itemCur !== null ? itemCur : itemOrig;
          monEntry.item_current = { known: true, item: curStr ? curStr : null };
        } else if (itemCur !== null && itemFlag) {
          monEntry.item_current = { known: true, item: itemCur ? itemCur : null };
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

    if (!teamPreview && llogSide && llogSide.conditions) {
      for (const [key, startTurn] of llogSide.conditions.entries()) {
        spec.sides[s].conditions.push({ key, start_turn: Math.max(1, startTurn || 1) });
      }
    }
  }

  const sanitizeDiffs = [];
  if (teamPreview) {
    return sanitizePositionSpec(spec, sanitizeDiffs);
  }

  // Apply PDAT authority for our own 3 picked mons
  const myMons = spec.sides[mySide].mons;
  const pdatMonByRosterSlot = new Map();
  if (pdat && Array.isArray(pdat.mons)) {
    for (let cIdx = 0; cIdx < pdat.mons.length; cIdx++) {
      const pm = pdat.mons[cIdx];
      let rSlot = Array.isArray(pickedRosterSlots) && pickedRosterSlots[cIdx] !== undefined
        ? pickedRosterSlots[cIdx]
        : myMons.findIndex(m => toId(m.species) === pm.speciesId);
      if (rSlot < 0 || rSlot >= myMons.length) continue;
      pdatMonByRosterSlot.set(rSlot, pm);
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
        const pdatSt = pm.status || '';
        target.status = pdatSt === 'psn' && target.status === 'tox' ? 'tox' : pdatSt;
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
            sanitizeDiffs.push({
              path: `sides[${s}].mons[${activeSlot}].hp`,
              from: topInfo.hp,
              to: 1,
              reason: 'fainted_active_on_move_turn',
            });
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
          const topSt = topInfo.status || '';
          actMon.status = topSt === 'psn' && actMon.status === 'tox' ? 'tox' : topSt;
        }
      }
    }
  }

  // Reconcile `upkeep_this_turn`, `fainted_this_turn`, and `trapped` with active state
  const myActiveSlot = spec.sides[mySide].active;
  const myActMon = myActiveSlot !== null ? myMons[myActiveSlot] : null;
  if (forceSwitch) {
    const isMidTurnBatonPass = Boolean(
      spec.sides[mySide].pending_bp && myActMon && !myActMon.fainted
    );
    if (isMidTurnBatonPass) {
      spec.upkeep_this_turn = Boolean(llog?.upkeepThisTurn);
      spec.sides[mySide].acted_this_turn = true;
    } else {
      // Post-faint replacement at end of turn: residual has already run
      spec.upkeep_this_turn = true;
      spec.sides[mySide].acted_this_turn = true;
      if (myActMon && myActMon.fainted && spec.sides[mySide].fainted_this_turn === null) {
        spec.sides[mySide].fainted_this_turn = myActiveSlot;
      }
    }
  } else {
    spec.upkeep_this_turn = false;
    const hasTrapMechanics = Boolean(
      myActMon &&
      (
        myActMon.locked ||
        myActMon.charging ||
        myActMon.must_recharge ||
        (Array.isArray(myActMon.volatiles) &&
          myActMon.volatiles.some(v => v.key === 'trapped' || v.key === 'partiallytrapped'))
      )
    );
    spec.trapped = Boolean(battleTurn?.trapped || hasTrapMechanics);
  }

  // Populate active move PPs from `<select Name="MOVE">` W1..W4 options (with PDAT / uses fallback and Encore/Disable flags)
  if (!forceSwitch && myActiveSlot !== null && myActMon && battleTurn) {
    const moveOpts = battleTurn.options.filter(o => o.kind === 'move');
    const ownSet = ownSets[myActiveSlot];
    const pdatMon = pdatMonByRosterSlot.get(myActiveSlot) || null;
    const encoreVol = Array.isArray(myActMon.volatiles)
      ? myActMon.volatiles.find(v => v.key === 'encore' && v.move)
      : null;
    const disableVol = Array.isArray(myActMon.volatiles)
      ? myActMon.volatiles.find(v => v.key === 'disable' && v.move)
      : null;
    const encoreBase = encoreVol ? (toId(encoreVol.move).startsWith('hiddenpower') ? 'hiddenpower' : toId(encoreVol.move)) : null;
    const disableBase = disableVol ? (toId(disableVol.move).startsWith('hiddenpower') ? 'hiddenpower' : toId(disableVol.move)) : null;

    if (ownSet && Array.isArray(ownSet.moves)) {
      const ppRows = [];
      for (let mIdx = 0; mIdx < ownSet.moves.length; mIdx++) {
        const rawMove = ownSet.moves[mIdx];
        const mId = toId(rawMove);
        const baseMoveId = mId.startsWith('hiddenpower') ? 'hiddenpower' : mId;
        const dexMv = dexData.moves[mId] || dexData.moves[baseMoveId];
        const maxpp = dexMv ? Math.floor(dexMv.pp * 8 / 5) : 16;
        const wOpt = moveOpts.find(
          o => o.slotIndex === mIdx + 1 || o.moveId === baseMoveId || o.moveId === mId
        );
        const pdatMv = pdatMon && Array.isArray(pdatMon.moves)
          ? pdatMon.moves.find(pm => pm.moveId === baseMoveId || pm.moveId === mId)
          : null;
        const usedEntry = Array.isArray(myActMon.uses)
          ? myActMon.uses.find(u => u.move === baseMoveId || u.move === mId)
          : null;
        const fallbackPp = pdatMv
          ? pdatMv.pp
          : Math.max(0, maxpp - (usedEntry ? usedEntry.n : 0));
        const curPp = wOpt ? (wOpt.pp ?? fallbackPp) : fallbackPp;
        const disabledByVol = Boolean(
          (encoreBase && encoreBase !== baseMoveId) ||
          (!encoreBase && disableBase && disableBase === baseMoveId)
        );
        const disabled = !wOpt || curPp <= 0 || disabledByVol;
        ppRows.push({
          move: mId,
          pp: Math.max(0, curPp),
          maxpp,
          disabled,
        });
      }
      myActMon.pp = ppRows;
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

  return sanitizePositionSpec(spec, sanitizeDiffs);
}

function sanitizePositionSpec(spec, diffOut = null) {
  const diffs = Array.isArray(diffOut) ? diffOut : [];
  for (let s = 0; s < 2; s++) {
    const side = spec.sides[s];
    if (spec.team_preview) {
      side.active = null;
      side.party = [];
      side.conditions = [];
      side.pending_bp = false;
      side.acted_this_turn = false;
      side.fainted_this_turn = null;
      side.fainted_last_turn = null;
      side.last_move = null;
      for (const m of side.mons) {
        m.active = false;
        m.fainted = false;
        m.appeared = false;
        m.appear_count = 0;
        m.switch_in_turn = 0;
        m.hp_den = Math.max(1, m.hp_den || 100);
        m.hp_num = m.hp_den;
        m.hp_exact = null;
        m.status = '';
        m.rest = false;
        m.slept = 0;
        m.tox_counter = null;
        m.comp_brn = false;
        m.comp_par = false;
        m.volatiles = [];
        m.locked = null;
        m.charging = null;
        m.must_recharge = false;
        m.transformed_into = null;
        m.mimic_overlay = null;
        m.last_move = null;
        m.stall_streak = 0;
        m.last_protect_turn = 0;
        m.protected_this_turn = false;
      }
      continue;
    }

    if (side.active === null || side.active < 0 || side.active >= side.mons.length) {
      diffs.push({
        path: `sides[${s}].active`,
        from: side.active,
        to: 0,
        reason: 'active_out_of_range',
      });
      side.active = 0;
    }
    for (let i = 0; i < side.mons.length; i++) {
      const m = side.mons[i];
      m.active = i === side.active;
      m.hp_den = Math.max(1, m.hp_den || 100);
      if (m.active && !spec.force_switch) {
        if (m.fainted) {
          diffs.push({
            path: `sides[${s}].mons[${i}].fainted`,
            from: true,
            to: false,
            reason: 'fainted_active_outside_force_switch',
          });
        }
        m.fainted = false;
      }
      if (m.fainted) {
        m.appeared = true;
        m.appear_count = Math.max(1, m.appear_count || 1);
        m.hp_num = 0;
        if (m.hp_exact !== null && m.hp_exact !== undefined) m.hp_exact = 0;
        m.status = '';
        m.rest = false;
        m.slept = 0;
        m.tox_counter = null;
        m.comp_brn = false;
        m.comp_par = false;
        m.boosts = [0, 0, 0, 0, 0, 0, 0];
        m.volatiles = [];
        m.locked = null;
        m.charging = null;
        m.must_recharge = false;
        m.transformed_into = null;
        m.mimic_overlay = null;
        m.last_move = null;
        m.stall_streak = 0;
        m.protected_this_turn = false;
      } else {
        const clampedHpNum = Math.max(1, Math.min(m.hp_den, m.hp_num || m.hp_den));
        if (m.hp_num !== clampedHpNum) {
          diffs.push({
            path: `sides[${s}].mons[${i}].hp_num`,
            from: m.hp_num,
            to: clampedHpNum,
            reason: 'hp_num_out_of_range',
          });
        }
        m.hp_num = clampedHpNum;
        if (m.hp_exact !== null && m.hp_exact !== undefined) {
          if (m.hp_exact < 1) {
            diffs.push({
              path: `sides[${s}].mons[${i}].hp_exact`,
              from: m.hp_exact,
              to: 1,
              reason: 'hp_exact_nonpositive_unfainted',
            });
          }
          m.hp_exact = Math.max(1, m.hp_exact);
        }
        if (m.status === 'fnt') {
          diffs.push({
            path: `sides[${s}].mons[${i}].status`,
            from: 'fnt',
            to: '',
            reason: 'fnt_status_on_unfainted',
          });
          m.status = '';
        }
      }
      if (m.active) {
        m.appeared = true;
        m.appear_count = Math.max(1, m.appear_count || 1);
        if (!m.fainted) {
          if (m.status === 'tox' && (m.tox_counter === null || m.tox_counter === undefined)) {
            m.tox_counter = 0;
          }
          if (m.status === 'brn' && !m.comp_brn && m.boosts[0] === 0) {
            m.comp_brn = true;
          }
          if (m.status === 'par' && !m.comp_par && m.boosts[4] === 0) {
            m.comp_par = true;
          }
        }
      }
    }
  }
  Object.defineProperty(spec, '_sanitizeDiffs', {
    value: diffs,
    enumerable: false,
    configurable: true,
    writable: true,
  });
  return spec;
}

// ------------------------------------------------------------------ Action Projection
/**
 * Map a searcher choice (`move <id>` or `switch <1..3>`) onto a concrete `<option Value="...">`
 * from `PS.cgi`'s `<select Name="MOVE">`, returning `{ value, path }` or `null`.
 */
function mapChoiceToPbsOptionWithTrace(choiceStr, spec, pickedRosterSlots, options, ownSets) {
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
        if (found) return { value: found.value, path: 'move-slot' };
      }
    }
    // 2. Match by parsed Japanese move ID on the `<option>`
    const byMoveId = options.find(
      o => o.kind === 'move' && (o.moveId === moveArg || o.moveId === baseMoveArg) && (o.pp === null || o.pp > 0)
    );
    if (byMoveId) return { value: byMoveId.value, path: 'move-id' };
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
          if (found) return { value: found.value, path: 'switch-slot' };
        }
      }
      // 2. Match by speciesId on the `<option>`
      const bySpecies = options.find(
        o => o.kind === 'switch' && o.speciesId === targetSpId && (o.hp === null || o.hp > 0)
      );
      if (bySpecies) return { value: bySpecies.value, path: 'switch-species' };
    }
  }

  return null;
}

function mapChoiceToPbsOption(choiceStr, spec, pickedRosterSlots, options, ownSets) {
  const res = mapChoiceToPbsOptionWithTrace(choiceStr, spec, pickedRosterSlots, options, ownSets);
  return res ? res.value : null;
}

/**
 * Resolve a guaranteed-valid `<option Value="...">` from `PS.cgi`'s `<select Name="MOVE">`,
 * returning `{ value, mappingPath, matchedAction }` for diagnostic logging.
 */
function resolveLegalPbsMoveOption({
  bestChoice,
  rootPolicy,
  spec,
  pickedRosterSlots,
  options,
  ownSets,
  isRandom,
}) {
  if (!Array.isArray(options) || options.length === 0) {
    return { value: 'W1', mappingPath: 'empty-default-w1', matchedAction: null };
  }

  const playableOptions = options.filter(o => {
    if (o.kind === 'move') return o.pp === null || o.pp > 0;
    if (o.kind === 'switch') return o.hp === null || o.hp > 0;
    return true;
  });
  const pool = playableOptions.length > 0 ? playableOptions : options;

  if (isRandom) {
    const picked = pool[rngInt(pool.length)];
    return { value: picked.value, mappingPath: 'random', matchedAction: null };
  }

  const direct = mapChoiceToPbsOptionWithTrace(bestChoice, spec, pickedRosterSlots, pool, ownSets);
  if (direct) {
    return {
      value: direct.value,
      mappingPath: `direct:${direct.path}`,
      matchedAction: bestChoice,
    };
  }

  if (rootPolicy && Array.isArray(rootPolicy.actions)) {
    for (const act of rootPolicy.actions) {
      const mapped = mapChoiceToPbsOptionWithTrace(act.input, spec, pickedRosterSlots, pool, ownSets);
      if (mapped) {
        return {
          value: mapped.value,
          mappingPath: `policy-fallback:${mapped.path}`,
          matchedAction: act.input,
        };
      }
    }
  }

  return {
    value: pool[0].value,
    mappingPath: 'first-playable-fallback',
    matchedAction: null,
  };
}

/**
 * Select a guaranteed-valid `<option Value="...">` from `PS.cgi`'s `<select Name="MOVE">`.
 * First tries `bestChoice`, then walks `rootPolicy.actions` in descending visit order,
 * and finally falls back to the first legal move/switch option in `<select Name="MOVE">`.
 */
function selectLegalPbsMoveOption(params) {
  return resolveLegalPbsMoveOption(params).value;
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

/**
 * Interpret a Castle input request without sending a network request.
 */
function interpretBattleRequest(page) {
  const kind = page && page.kind;

  if (![
    'BATTLE_INPUT',
    'BATTLE_CHANGE',
    'BATTLE_BATONN'
  ].includes(kind)) {
    throw new Error(
      'Not a supported battle input page: ' + kind
    );
  }

  const battleTurn = parseBattleTurnHtml(page.html || '');

  const isBatonPass = kind === 'BATTLE_BATONN';
  const isForceSwitch = kind !== 'BATTLE_INPUT';

  const actionType = isBatonPass
    ? 'BATONN'
    : (isForceSwitch ? 'CHANGE' : 'INPUT');

  const rawNowInt = battleTurn.nowVal === ''
    ? 0
    : (parseInt(battleTurn.nowVal, 10) || 0);

  const turnNum = isForceSwitch
    ? Math.max(1, rawNowInt)
    : rawNowInt + 1;

  // BATONN is a switch request, but its ACTION must remain BATONN.
  // Never invent an option if the form is unrecognized.
  if (isBatonPass) {
    if (!/^[0-9]+$/.test(battleTurn.nowVal)) {
      throw new Error(
        'BATONN page has no valid NOW field'
      );
    }

    if (
      !battleTurn.options.length ||
      battleTurn.options.some(
        o => o.kind !== 'switch' ||
             !/^C[1-3]$/.test(o.value)
      )
    ) {
      throw new Error(
        'BATONN page has missing or unrecognized switch options'
      );
    }

    if (!battleTurn.options.some(
      o => o.hp === null || o.hp > 0
    )) {
      throw new Error(
        'BATONN page has no selectable living switch target'
      );
    }
  }

  return {
    battleTurn,
    isForceSwitch,
    actionType,
    turnNum
  };
}

// ------------------------------------------------------------------ WASM Initialization
let wasm = null;
let wasmDex = null;
function ensureObservationRuntime() {
  if (wasm && wasmDex) return;
  wasm = require(path.join(REPO, 'crates/wasm/pkg-node/nc2000_wasm.js'));
  wasmDex = new wasm.Dex();
}
if (!RANDOM) ensureObservationRuntime();

function assertRepositoryOutput(filename) {
  if (!filename) return;
  const target = path.resolve(filename);
  const root = fs.realpathSync(REPO);
  const relative = path.relative(root, target);
  assert.ok(relative && relative !== '..' && !relative.startsWith('..' + path.sep) &&
    !path.isAbsolute(relative), 'OUTPUT_OUTSIDE_REPOSITORY:' + target);
  let existing = target;
  while (true) {
    try { fs.lstatSync(existing); break; }
    catch (error) {
      if (error.code !== 'ENOENT') throw error;
      existing = path.dirname(existing);
    }
  }
  const resolved = fs.realpathSync(existing);
  const actualRelative = path.relative(root, resolved);
  assert.ok(actualRelative === '' || (actualRelative !== '..' &&
    !actualRelative.startsWith('..' + path.sep) && !path.isAbsolute(actualRelative)),
  'OUTPUT_SYMLINK_OUTSIDE_REPOSITORY:' + target);
}

function chooseObservedAction(prepared, {isRandom, iterations, chooseIndex = rngInt}) {
  assert.equal(prepared?.status, 'READY', 'OBSERVATION_NOT_READY');
  const {searcher: current, built} = prepared;
  assert.ok(current && built && Array.isArray(built.expectedNativeInputs) &&
    built.expectedNativeInputs.length > 0, 'NO_OBSERVED_LEGAL_ACTION');
  assert.ok(Number.isSafeInteger(iterations) && iterations > 0, 'INVALID_SEARCH_BUDGET');
  let bestChoice, mappingPath;
  if (isRandom) {
    const index = chooseIndex(built.expectedNativeInputs.length);
    assert.ok(Number.isSafeInteger(index) && index >= 0 && index < built.expectedNativeInputs.length);
    bestChoice = built.expectedNativeInputs[index];
    mappingPath = 'observed:random';
  } else if (built.expectedNativeInputs.length === 1) {
    bestChoice = built.expectedNativeInputs[0];
    mappingPath = 'observed:only-option';
  } else {
    current.step(iterations);
    bestChoice = current.best();
    mappingPath = 'observed:search';
  }
  assert.ok(built.expectedNativeInputs.includes(bestChoice), 'UNMAPPED_SEARCH_ACTION');
  const mapping = built.mapNativeInput(bestChoice);
  const options = built.ui.battleTurn.options.filter(row => row.value === mapping.value);
  assert.equal(options.length, 1, 'CASTLE_OPTION_NOT_UNIQUE');
  assert.equal(options[0].kind, mapping.kind, 'CASTLE_OPTION_KIND_MISMATCH');
  const rootPolicy = JSON.parse(current.rootPolicy());
  const metrics = JSON.parse(current.metrics());
  assert.equal(metrics.projections, 0, 'ACTION_PROJECTION_BLOCKED');
  assert.equal(metrics.legalityDrift, 0, 'ACTION_LEGALITY_DRIFT');
  return {bestChoice, mapping, mappingPath, rootPolicy, metrics,
    iterations: rootPolicy.iterations};
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
async function playOneGame(gameIdx, dependencies = {}) {
  for (const filename of [DECISION_LOG, OPPONENT_DB_PATH, MATCH_LOG_PATH, TRACE_DIR_PATH, ANOMALY_LOG_PATH]) {
    assertRepositoryOutput(filename);
  }
  const client = dependencies.httpClient || new PbsHttpClient();
  const team = dependencies.team || pickTeam();
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
  let observationSession = null;
  let unknownPageStreak = 0;
  const matchStartedAt = new Date().toISOString();
  const roomTagMatch = ROOM_URL.match(/\/PBS\/_\/([^/]+)\//i);
  const roomTag = roomTagMatch ? roomTagMatch[1].toUpperCase() : 'ROOM';
  const matchId = `${matchStartedAt.replace(/[:.]/g, '-')}-${roomTag}-g${gameIdx + 1}`;
  const traceFilePath = TRACE_DIR_PATH ? path.join(TRACE_DIR_PATH, `${matchId}.jsonl`) : '';
  const traceEvents = [];

  const appendTraceEvent = event => {
    const rec = {
      ts: new Date().toISOString(),
      matchId,
      ...event,
    };
    traceEvents.push(rec);
    if (dependencies.onTraceEvent) dependencies.onTraceEvent(rec);
    if (!traceFilePath) return;
    try {
      fs.mkdirSync(path.dirname(traceFilePath), { recursive: true });
      fs.appendFileSync(traceFilePath, JSON.stringify(rec) + '\n', { mode: 0o600 });
    } catch {
      // Non-fatal trace persistence
    }
  };

  const submissionGuard = createSubmissionGuard({
    matchId, classifyPage, readDecisionForm,
    getBundle: () => captureStableBundle({http: client, roomUrl: ROOM_URL, classifyPage}),
    getPage: () => client.get(ROOM_URL),
    getFinishedPage: async () => {
      const terminal = await client.get(`${ROOM_URL}?ACTION=LLOG`);
      assert.ok(terminal.statusCode >= 200 && terminal.statusCode < 300 &&
        typeof terminal.html === 'string' && terminal.html.length, 'FINAL_LLOG_HTTP_FAILED');
      assert.ok(previewSides && (mySide === 0 || mySide === 1) && foeName,
        'FINAL_LLOG_MATCH_IDENTITY_REQUIRED');
      const lastPreview = parseTeamPreviewHtml(terminal.html);
      const names = [lastPreview.p1Name, lastPreview.p2Name];
      assert.equal(names[mySide], NAME, 'FINAL_LLOG_OWN_NAME_MISMATCH');
      assert.equal(names[1 - mySide], foeName, 'FINAL_LLOG_FOE_NAME_MISMATCH');
      const identities = mons => mons.map(mon => ({species: mon.speciesId,
        level: mon.level, gender: mon.gender, item: mon.item_flag}));
      assert.deepEqual([lastPreview.p1Mons, lastPreview.p2Mons].map(identities),
        previewSides.map(identities), 'FINAL_LLOG_PREVIEW_MISMATCH');
      assert.ok(['WIN', 'LOSE', 'DRAW'].includes(
        resolveBotOutcomeFromFinishedHtml(terminal.html, NAME, foeName)), 'FINAL_LLOG_OUTCOME_REQUIRED');
      return terminal;
    },
    postOnce: fields => client.postForm(ROOM_URL, fields, {maxAttempts: 1}),
    onEvent: appendTraceEvent,
  });

  appendTraceEvent({
    type: 'match_start',
    gameIdx,
    roomUrl: ROOM_URL,
    roomTag,
    botName: NAME,
    tier: TIER,
    iters: ITERS,
    c: PROFILE.c,
    seed: SEED,
    random: RANDOM,
    team: {
      id: team.id,
      label: team.label,
      tier: team.tier,
      sets: team.sets,
    },
  });

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
        revealedMoves: m?.revealedMoves || [],
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

  const finalizeMatchRecord = (outcome, finishDetails = {}) => {
    appendTraceEvent({
      type: 'finish',
      finishTrigger: finishDetails.finishTrigger || 'FINISHED',
      rawFinishHtml: finishDetails.rawFinishHtml ?? null,
      rawFinalLlogHtml: finishDetails.rawFinalLlogHtml ?? null,
      finalLlogSummary: finishDetails.finalLlogSummary ?? null,
      outcome,
      decisions,
      mySide,
      foeName: foeName || null,
      pickedRosterSlots,
      previewSides,
      seenFoeMonsFinal: seenFoeMons ? JSON.parse(JSON.stringify(seenFoeMons)) : null,
      errors: finishDetails.errors || [],
    });

    let parityReport = null;
    try {
      const { auditMatchTrace, appendAnomalyReport } = require('./pbs-parity-audit.js');
      parityReport = auditMatchTrace(traceEvents, {
        filePath: traceFilePath || null,
        reparse: false,
      });
      if (parityReport && parityReport.anomalies && parityReport.anomalies.length > 0) {
        appendAnomalyReport(parityReport, ANOMALY_LOG_PATH);
        log(
          `[PARITY_WARN] Match ${matchId}: ${parityReport.errorCount} error(s), ${parityReport.warnCount} warning(s) ` +
          `(${parityReport.anomalies.map(a => a.code).join(', ')})`
        );
      }
    } catch (err) {
      log(`Parity audit execution warning: ${err.message || err}`);
    }

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
        matchId,
        traceFile: traceFilePath ? path.relative(REPO, traceFilePath) : null,
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
        parityOk: parityReport ? parityReport.ok : null,
        parityErrorCount: parityReport ? parityReport.errorCount : null,
        parityWarnCount: parityReport ? parityReport.warnCount : null,
      });
    }
  };

  try {
    let res = await client.get(ROOM_URL);

    while (true) {
      const page = classifyPage(res);
      if (page.kind !== 'UNKNOWN') unknownPageStreak = 0;

      // Confirm entry only after PBS returns an entered-room state.
      if (enteredRoom && !entryStatusSent && [
        'WAITING_ENTRY',
        'REDIRECT_PDINPUT',
        'TEAM_PREVIEW_CHECK',
        'TEAM_PREVIEW_SELECT',
        'WAITING_SELECT',
        'SELECT_DONE',
        'WAITING_TURN',
        'BATTLE_INPUT',
        'BATTLE_CHANGE',
      ].includes(page.kind)) {
        reportEntryStatus('success', page.kind);
      }

      if (page.kind === 'FINISHED') {
        if (!enteredRoom && page.reason === 'match_end_page') {
          res = await client.get(`${ROOM_URL}?ACTION=TITLE`);
          continue;
        }
        let resultHtml = page.html || '';
        let finalLlogHtml = '';
        let finalLlogObj = null;
        const finishErrors = [];
        try {
          const resultLog =
            await client.get(`${ROOM_URL}?ACTION=LLOG`);
          if (resultLog && resultLog.html) {
            finalLlogHtml = resultLog.html;
            resultHtml += '\n' + resultLog.html;
          }
        } catch (err) {
          finishErrors.push({ stage: 'fetch_final_llog', message: String(err.message || err) });
        }

        // Merge final LLOG into persistent `seenFoeMons` & `lastLlogFoeMons` before recording match
        const llogHtmlToParse = finalLlogHtml || page.html || '';
        if (llogHtmlToParse && mySide !== null) {
          try {
            finalLlogObj = parseLlogHtml(llogHtmlToParse, previewSides || [[], []]);
            const foeSide = 1 - mySide;
            if (!observationSession && finalLlogObj && finalLlogObj.sides[foeSide]) {
              const merged = mergeLlogIntoSeenFoeMons({
                llog: finalLlogObj,
                foeSide,
                seenFoeMons,
                previewSides,
                battleTurn: null,
                turnNum: finalLlogObj.turn || 1,
                isForceSwitch: false,
                lastFoeActiveSlot,
              });
              lastFoeActiveSlot = merged.lastFoeActiveSlot;
              lastLlogFoeMons = merged.llogFoeMons || lastLlogFoeMons;
            }
          } catch (err) {
            finishErrors.push({ stage: 'parse_final_llog', message: String(err.message || err) });
          }
        }

        const outcome =
          resolveBotOutcomeFromFinishedHtml(
            resultHtml,
            NAME,
            foeName
          );
        log(`Game #${gameIdx + 1} finished (${outcome}) after ${decisions} decisions.`);
        finalizeMatchRecord(outcome, {
          finishTrigger: 'FINISHED',
          rawFinishHtml: page.html || '',
          rawFinalLlogHtml: finalLlogHtml || null,
          finalLlogSummary: serializeLlogTracker(finalLlogObj),
          errors: finishErrors,
        });
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
        if (ENTRY_STATUS_IPC && enteredRoom && !entryStatusSent) {
          reportEntryStatus('failed', 'pbs_error');
          log(`PBS entry failed: ${page.errorText}`);
          return {
            outcome: 'ENTRY_FAILED',
            decisions: 0,
            team: team.id,
            foeName: null,
          };
        }
        log(`PBS returned ERROR page: ${page.errorText}`);
        appendTraceEvent({
          type: 'error_page',
          errorText: page.errorText,
          rawErrorHtml: page.html || '',
        });
        await sleep(POLL_MS);
        res = await client.get(ROOM_URL);
        continue;
      }

      if (page.kind === 'ENTORY') {
        // If we had already entered and the room returned to ENTORY, the game ended
        if (enteredRoom && decisions > 0) {
          let finalLlogHtml = '';
          let finalLlogObj = null;
          const finishErrors = [];
          if (mySide !== null) {
            try {
              const finalLogRes = await client.get(`${ROOM_URL}?ACTION=LLOG`);
              if (finalLogRes && finalLogRes.html) {
                finalLlogHtml = finalLogRes.html;
                finalLlogObj = parseLlogHtml(finalLogRes.html, previewSides || [[], []]);
                const foeSide = 1 - mySide;
                if (!observationSession && finalLlogObj && finalLlogObj.sides[foeSide]) {
                  const merged = mergeLlogIntoSeenFoeMons({
                    llog: finalLlogObj,
                    foeSide,
                    seenFoeMons,
                    previewSides,
                    battleTurn: null,
                    turnNum: finalLlogObj.turn || 1,
                    isForceSwitch: false,
                    lastFoeActiveSlot,
                  });
                  lastFoeActiveSlot = merged.lastFoeActiveSlot;
                  lastLlogFoeMons = merged.llogFoeMons || lastLlogFoeMons;
                }
              }
            } catch (err) {
              finishErrors.push({ stage: 'entory_reset_final_llog', message: String(err.message || err) });
            }
          }
          const inferredOutcome = finalLlogHtml
            ? resolveBotOutcomeFromFinishedHtml(finalLlogHtml, NAME, foeName)
            : 'END';
          log(`Game #${gameIdx + 1} finished (room reset to ENTORY, outcome=${inferredOutcome}) after ${decisions} decisions.`);
          finalizeMatchRecord(inferredOutcome, {
            finishTrigger: 'ENTORY_RESET',
            rawFinishHtml: page.html || '',
            rawFinalLlogHtml: finalLlogHtml || null,
            finalLlogSummary: serializeLlogTracker(finalLlogObj),
            errors: finishErrors,
          });
          return { outcome: inferredOutcome, decisions, team: team.id, foeName };
        }

        if (page.p1Waiting) {
          if (MODE === 'host') {
            if (ENTRY_STATUS_IPC) {
              reportEntryStatus('failed', 'room_busy');
              log('PBS room is already occupied; entry rejected.');
              return {
                outcome: 'ROOM_BUSY',
                decisions: 0,
                team: team.id,
                foeName: page.p1Name || null,
              };
            }
            log(`P1 (${page.p1Name}) is already in room, waiting (--mode host)...`);
            await sleep(POLL_MS);
            res = await client.get(ROOM_URL);
            continue;
          }
          foeName = page.p1Name || foeName;
          log(`Joining against waiting P1 "${page.p1Name}" as P2...`);
          mySide = 1;
          enteredRoom = true;
          appendTraceEvent({
            type: 'entry',
            entrySide: 1,
            p1WaitingName: page.p1Name || null,
            rawEntryHtml: page.html || '',
          });
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
          appendTraceEvent({
            type: 'entry',
            entrySide: 0,
            p1WaitingName: null,
            rawEntryHtml: page.html || '',
          });
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
        appendTraceEvent({
          type: 'pdinput',
          sim: page.sim,
          formBody,
          redirectLocation: pdRes.location || null,
          rawPdinputHtml: pdRes.html || '',
        });
        if (pdRes.location) {
          res = await client.get(new URL(pdRes.location, ROOM_URL).toString());
        } else {
          res = await client.get(ROOM_URL);
        }
        continue;
      }

      if (page.kind === 'TEAM_PREVIEW_CHECK') {
        const previewErrors = [];
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
          gender: pm.gender ?? 'M',
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
          uses: {},
        }));

        log(
          `Team Preview (side=P${mySide + 1}, foe="${foeName || '?'}"): Foe roster = ` +
          previewSides[foeSide].map(m => `${m.species}(L${m.level})`).join(', ')
        );

        let choiceStr = null;
        let previewSource = 'fallback';
        let previewPolicy = null;
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
              } catch (err) {
                previewErrors.push({ stage: 'compiled_preview_key', message: String(err.message || err) });
              }
            }
            if (!choiceStr && !hasCustomShellMemory) {
              choiceStr = s.bakedPreview();
              if (choiceStr) previewSource = 'baked';
            }
            if (!choiceStr) {
              s.step(ITERS);
              choiceStr = s.best();
              previewSource = hasCustomShellMemory ? 'adapted-search' : 'search';
              try {
                previewPolicy = JSON.parse(s.rootPolicy());
              } catch {}
            }
          } catch (err) {
            previewErrors.push({ stage: 'preview_search', message: String(err.message || err) });
            log(`Preview search warning (using fallback pick): ${err.message || err}`);
          }
        }

        pickedRosterSlots = dependencies.previewPick
          ? dependencies.previewPick({choiceStr, ownSets: team.sets, isRandom: RANDOM})
          : parseOrFallbackPreviewPick(choiceStr, team.sets, RANDOM);
        const [leadIdx, b1Idx, b2Idx] = pickedRosterSlots;
        log(
          `Preview pick: Lead=#${leadIdx + 1} (${team.sets[leadIdx].species}), ` +
          `Bench=#${b1Idx + 1} (${team.sets[b1Idx].species}), #${b2Idx + 1} (${team.sets[b2Idx].species})` +
          (choiceStr ? ` [${previewSource}: ${choiceStr}]` : '')
        );

        decisions++;
        const gameSeed = SEED * 1000 + gameIdx * 10 + mySide;
        const checkFields = {
          ACTION: 'CHECK',
          SENTE: String(leadIdx + 1),
          [`P${b1Idx + 1}`]: '1',
          [`P${b2Idx + 1}`]: '1',
        };

        appendTraceEvent({
          type: 'preview',
          decision: decisions,
          mySide,
          foeName: foeName || null,
          seed: gameSeed,
          rawCheckHtml: page.html || '',
          parsedPreview,
          spec,
          sanitizeDiffs: spec._sanitizeDiffs || [],
          previewSource,
          choiceStr,
          pickedRosterSlots,
          rootPolicy: previewPolicy,
          checkFields,
          errors: previewErrors,
        });

        appendDecisionLog({
          gameIdx,
          decision: decisions,
          phase: 'preview',
          turn: 0,
          side: mySide,
          foeName,
          seed: gameSeed,
          iterations: previewSource === 'search' || previewSource === 'adapted-search' ? ITERS : 0,
          c: PROFILE.c,
          previewSource,
          bestChoice: choiceStr,
          choiceStr,
          pickedRosterSlots,
          mappingPath: `preview:${previewSource}`,
          rootPolicy: previewPolicy,
          spec,
        });

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

      if (page.kind === 'BATTLE_INPUT' || page.kind === 'BATTLE_CHANGE' || page.kind === 'BATTLE_BATONN') {
        let bundle = null, prepared = null;
        try {
          assert.ok((mySide === 0 || mySide === 1) && previewSides && pickedRosterSlots,
            'OBSERVATION_INITIAL_CONTEXT_REQUIRED');
          bundle = await captureStableBundle({http: client, roomUrl: ROOM_URL,
            classifyPage, initialResponse: res});
          ensureObservationRuntime();
          if (!observationSession) {
            const workDir = path.join(REPO, 'pbs-work', 'observations', matchId);
            assertRepositoryOutput(workDir);
            const createSession = dependencies.createObservationSession ||
              require('./pbs-castle-observation').createObservationSession;
            observationSession = createSession({
              repoRoot: REPO, workDir, client: module.exports, wasm, dex: wasmDex, dexData,
              dictionaryPath: path.join(REPO, 'tools/pbs-castle-dictionary.js'),
              side: mySide, matchId, ownSets: team.sets, previewSides, pickedRosterSlots,
              seed: SEED * 1000 + gameIdx * 10 + mySide, c: PROFILE.c,
              poolJson: beliefPoolJson,
              poolProvider: ({seenFoeMons: observed, foeSide}) => buildDynamicBeliefPoolJson({
                foePreviewMons: previewSides[foeSide], foeName,
                seenFoeMons: observed, opponentDb,
              }),
            });
            // Preview's searcher is no longer a battle input authority.
            if (searcher) {
              searcher.free();
              searcher = null;
              searcherBeliefSignature = '';
            }
          }
          const seenFoeMonsBefore = seenFoeMons ? JSON.parse(JSON.stringify(seenFoeMons)) : null;
          const lastFoeActiveSlotBefore = lastFoeActiveSlot;
          prepared = observationSession.prepare(bundle);
          assert.equal(prepared.status, 'READY', 'OBSERVATION_NOT_READY');
          const {built, spec, report} = prepared;
          const {battleTurn, actionType, isForceSwitch} = built.ui;
          const decisionPolicy = dependencies.decisionPolicy || {isRandom: RANDOM, iterations: ITERS};
          const selected = chooseObservedAction(prepared, decisionPolicy);
          const {bestChoice, rootPolicy, mappingPath, mapping} = selected;
          const chosenMoveValue = mapping.value;
          const postFields = {ACTION: actionType, MOVE: chosenMoveValue, NOW: battleTurn.nowVal};
          const form = readDecisionForm(bundle.rawTurnHtml);
          assert.equal(postFields.ACTION, form.action, 'PREPARED_ACTION_FORM_MISMATCH');
          assert.equal(postFields.NOW, form.now, 'PREPARED_NOW_FORM_MISMATCH');
          seenFoeMons = JSON.parse(JSON.stringify(prepared.seenFoeMons));
          lastFoeActiveSlot = prepared.lastFoeActiveSlot;
          // No union with the legacy parser's possibly corrected-away events.
          lastLlogFoeMons = null;
          decisions++;
          const gameSeed = SEED * 1000 + gameIdx * 10 + mySide;
          log(`Turn ${spec.turn} (${actionType}, NOW="${battleTurn.nowVal}"): ` +
            `selected MOVE=${chosenMoveValue} [${mappingPath}: ${bestChoice}]`);
          appendTraceEvent({
            type: 'decision', decision: decisions, phase: actionType,
            forceSwitch: isForceSwitch, turn: spec.turn, rawNowVal: battleTurn.nowVal,
            mySide, foeName: foeName || null, seed: gameSeed,
            iterations: selected.iterations,
            requestedIterations: decisionPolicy.isRandom ? 0 : decisionPolicy.iterations,
            c: PROFILE.c, rawTurnHtml: bundle.rawTurnHtml, rawPdatHtml: bundle.rawPdatHtml,
            rawLlogHtml: bundle.rawLlogHtml, battleTurn, pdat: built.pdat,
            llogSummary: null, pickedRosterSlots, lastFoeActiveSlotBefore,
            lastFoeActiveSlotAfter: lastFoeActiveSlot, seenFoeMonsBefore,
            seenFoeMonsAfter: JSON.parse(JSON.stringify(seenFoeMons)), spec,
            sanitizeDiffs: [], bestChoice, chosenMoveValue, mappingPath,
            matchedAction: bestChoice, rootPolicy, postFields, errors: [],
            observationReport: report, observationSourceHashes: prepared.sourceHashes,
            observationRoute: 'CASTLE_OBSERVATION_UNCHANGED_CORE',
            sentToCastle: false,
          });
          appendDecisionLog({
            gameIdx, decision: decisions, phase: actionType, turn: spec.turn,
            nowVal: battleTurn.nowVal, side: mySide, foeName, seed: gameSeed,
            iterations: selected.iterations,
            requestedIterations: decisionPolicy.isRandom ? 0 : decisionPolicy.iterations,
            c: PROFILE.c, bestChoice, chosenMoveValue, mappingPath, matchedAction: bestChoice,
            options: battleTurn.options, rootPolicy, spec,
            observationReport: report, observationSourceHashes: prepared.sourceHashes,
          });
          const submitted = await submissionGuard.submit({bundle, fields: postFields});
          res = submitted.response;
        } catch (error) {
          appendTraceEvent({
            type: 'observation_stopped', stage: 'battle_observation_or_submission',
            message: String(error?.message || error),
            rawTurnHtml: bundle?.rawTurnHtml ?? page.html ?? null,
            rawPdatHtml: bundle?.rawPdatHtml ?? null,
            rawLlogHtml: bundle?.rawLlogHtml ?? null,
            observationReport: prepared?.report ?? null,
            submission: error?.submission ?? submissionGuard.snapshot(),
            automaticFallback: false, automaticInitialization: false,
          });
          throw error;
        } finally {
          if (prepared?.searcher) prepared.searcher.free();
        }
        continue;
      }

      // Unknown/transitional pages: preserve evidence and fail
      // after repeated failures instead of polling indefinitely.
      unknownPageStreak++;

      if (
        unknownPageStreak === 1 ||
        unknownPageStreak === 12
      ) {
        appendTraceEvent({
          type: 'unhandled_page',
          pageKind: page.kind,
          streak: unknownPageStreak,
          statusCode: res.statusCode,
          location: res.location || null,
          rawUnknownHtml: page.html || ''
        });
      }

      if (unknownPageStreak >= 12) {
        throw new Error(
          `Unknown PBS page state persisted for ${unknownPageStreak} polls; raw HTML preserved in trace`
        );
      }

      log(
        `Unhandled page state (${page.kind}), polling ${ROOM_URL}... (${unknownPageStreak}/12)`
      );

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
  playOneGame,
  chooseObservedAction,
  assertRepositoryOutput,
  classifyPage,
  parseTeamPreviewHtml,
  parseBattleTurnHtml,
  interpretBattleRequest,
  parsePdatHtml,
  parseLlogHtml,
  serializeLlogTracker,
  advanceLlogTrackerToTurn,
  mergeLlogIntoSeenFoeMons,
  calcSetMaxHp,
  inferHpTypeMaskFromMaxHp,
  buildDynamicBeliefPoolJson,
  loadOpponentDb,
  saveOpponentDb,
  recordOpponentObservation,
  buildAndSanitizePositionSpec,
  sanitizePositionSpec,
  resolveLegalPbsMoveOption,
  selectLegalPbsMoveOption,
  parseOrFallbackPreviewPick,
  encodeTeamForPbsCookie,
  encodeFormEucJp,
  resolveBotOutcomeFromFinishedHtml,
  TIER_A_BOX_IDS,
};
