#!/usr/bin/env node
/**
 * Resident Discord Bot for Gym Leader's Castle (`ジムリーダーの城`, `#2000-一撃なし`).
 *
 * Official Castle Discord room mapping (`http://psense.lib.net/PBS/chat/`):
 * - `第1` / `第一` -> Room `C` (`http://psense.lib.net/PBS/_/C/PS.cgi`)
 * - `第2` / `第二` -> Room `D` (`http://psense.lib.net/PBS/_/D/PS.cgi`)
 * - `第3` / `第三` -> Room `E` (`http://psense.lib.net/PBS/_/E/PS.cgi`)
 * - `第4` / `第四` -> Room `F` (`http://psense.lib.net/PBS/_/F/PS.cgi`)
 * - `第5` / `第五` -> Room `G` (`http://psense.lib.net/PBS/_/G/PS.cgi`)
 * - `第6` / `第六` -> Room `H` (`http://psense.lib.net/PBS/_/H/PS.cgi`)
 * (Trainer House `L`..`O` and direct room letters `C`..`O` are also supported.)
 *
 * Verified Castle Discord workflow (`#2000-一撃なし`):
 * 1. Idles with zero traffic to `psense.lib.net` until a user `@mentions` the bot.
 * 2. Determines strength/party tier from the `@mention` text:
 *    - Default (`tier = "default"`): `81,000` iterations (`c = 0.4`) + `pool:nash` (APEX4 equilibrium mixture).
 *    - Mercy / Handicap (`tier = "mercy"`, triggered by `手加減`, `別パ`, `サブ`, `Tier A`, etc.):
 *      `81,000` iterations (`c = 0.4`) + `pool:tier-a` (8 iconic human-origin Tier A teams).
 * 3. Replies `"応戦します"` (via Discord Reply) to the challenger's message:
 *    - When the challenger replies `第1へ` / `第2にて` / `では第三でお願いします` (or the `アナウンス [APP]`
 *      bot posts the `[第1]` through `[第6]` link), our bot posts `"よろしくお願いします"` and
 *      immediately launches `pbs-client.js` in the corresponding room (`C` through `H`).
 *    - If the challenger already wrote `第1` through `第6` in the initial `@mention`, our bot replies
 *      `"応戦します\nよろしくお願いします"` and launches `pbs-client.js` immediately.
 *    - If `autoPickRoomAfterSec` (default 15s) elapses after `"応戦します"` without the challenger naming
 *      a room, our bot checks `第1 (C)` through `第6 (H)` on `psense.lib.net`, picks the room where
 *      the challenger is waiting or the first empty room, posts `"第1にて\nよろしくお願いします"` (which
 *      also triggers `アナウンス [APP]` to post the link), and enters that room.
 * 4. Upon match completion, posts `"ありがとうございました"`.
 */
'use strict';

const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');
const WebSocket = require('ws');
const {
  PbsHttpClient,
  classifyPage,
  resolveBotOutcomeFromFinishedHtml,
  parseTeamPreviewHtml,
  parseBattleTurnHtml,
  parsePdatHtml,
  parseLlogHtml,
  serializeLlogTracker,
  advanceLlogTrackerToTurn,
  mergeLlogIntoSeenFoeMons,
  buildAndSanitizePositionSpec,
  resolveLegalPbsMoveOption,
  recordOpponentObservation,
} = require('./pbs-client.js');
const {
  auditMatchTrace,
} = require('./pbs-parity-audit.js');

const REPO = path.resolve(__dirname, '..');

const K_ROOM_URL = 'http://psense.lib.net/PBS/_/K/PS.cgi';
const K_ROOM_LINK = 'http://psense.lib.net/PBS/_/K/PS.cgi?ACTION=TITLE';


// Official Castle Discord (`http://psense.lib.net/PBS/chat/`) room table:
const CASTLE_DISCORD_ROOMS = [
  { num: 1, label: '第1', letter: 'C' },
  { num: 2, label: '第2', letter: 'D' },
  { num: 3, label: '第3', letter: 'E' },
  { num: 4, label: '第4', letter: 'F' },
  { num: 5, label: '第5', letter: 'G' },
  { num: 6, label: '第6', letter: 'H' },
];

const LETTER_TO_LABEL = {
  C: '第1',
  D: '第2',
  E: '第3',
  F: '第4',
  G: '第5',
  H: '第6',
};

const DAI_TOKEN_TO_ROOM = {
  '1': { letter: 'C', label: '第1' },
  '１': { letter: 'C', label: '第1' },
  '一': { letter: 'C', label: '第1' },
  '2': { letter: 'D', label: '第2' },
  '２': { letter: 'D', label: '第2' },
  '二': { letter: 'D', label: '第2' },
  '3': { letter: 'E', label: '第3' },
  '３': { letter: 'E', label: '第3' },
  '三': { letter: 'E', label: '第3' },
  '4': { letter: 'F', label: '第4' },
  '４': { letter: 'F', label: '第4' },
  '四': { letter: 'F', label: '第4' },
  '5': { letter: 'G', label: '第5' },
  '５': { letter: 'G', label: '第5' },
  '五': { letter: 'G', label: '第5' },
  '6': { letter: 'H', label: '第6' },
  '６': { letter: 'H', label: '第6' },
  '六': { letter: 'H', label: '第6' },
};

function parseArgs(argv) {
  const out = {};
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith('--')) {
      const key = a.slice(2);
      const next = argv[i + 1];
      if (next !== undefined && !next.startsWith('--')) {
        out[key] = next;
        i++;
      } else {
        out[key] = true;
      }
    }
  }
  return out;
}

function loadConfig(cliArgs = {}) {
  const candidates = [
    cliArgs.config ? path.resolve(String(cliArgs.config)) : null,
    path.resolve(process.cwd(), 'pbs-bot.config.json'),
    path.resolve(REPO, 'pbs-bot.config.json'),
  ].filter(Boolean);

  let fileCfg = {};
  for (const p of candidates) {
    if (fs.existsSync(p)) {
      try {
        fileCfg = JSON.parse(fs.readFileSync(p, 'utf8'));
        console.log(`[discord-bot] Loaded configuration from ${p}`);
        break;
      } catch (err) {
        console.error(`[discord-bot] Failed to parse config file ${p}:`, err.message);
      }
    }
  }

  const splitList = val => {
    if (!val) return [];
    if (Array.isArray(val)) return val.map(String).map(s => s.trim()).filter(Boolean);
    return String(val).split(',').map(s => s.trim()).filter(Boolean);
  };

  return {
    discordToken: cliArgs.token || process.env.DISCORD_TOKEN || fileCfg.discordToken || '',
    pbsName: cliArgs.name || process.env.PBS_NAME || fileCfg.pbsName || 'NC2000Bot',
    pbsPass: cliArgs.pass || process.env.PBS_PASS || fileCfg.pbsPass || 'nc2000pass',
    pbsMsg: cliArgs.msg || process.env.PBS_MSG || fileCfg.pbsMsg || 'よろしくお願いします',
    channelIds: splitList(cliArgs.channels || process.env.CHANNEL_IDS || fileCfg.channelIds),
    roomBotIds: splitList(cliArgs['room-bots'] || process.env.ROOM_BOT_IDS || fileCfg.roomBotIds),
    replyTriggerText:
      cliArgs['reply-text'] ??
      process.env.REPLY_TRIGGER_TEXT ??
      fileCfg.replyTriggerText ??
      '応戦します',
    replyMode:
      cliArgs['reply-mode'] ||
      process.env.REPLY_MODE ||
      fileCfg.replyMode ||
      'reply',
    roomEnterGreetText:
      cliArgs['greet-text'] ??
      process.env.ROOM_ENTER_GREET_TEXT ??
      fileCfg.roomEnterGreetText ??
      'よろしくお願いします',
    postMatchThanksText:
      cliArgs['thanks-text'] ??
      process.env.POST_MATCH_THANKS_TEXT ??
      fileCfg.postMatchThanksText ??
      'ありがとうございました',
    iters: parseInt(cliArgs.iters || process.env.PBS_ITERS || fileCfg.iters || '81000', 10),
    hostTimeoutSec: parseInt(
      cliArgs['host-timeout-sec'] || process.env.HOST_TIMEOUT_SEC || fileCfg.hostTimeoutSec || '240',
      10
    ),
    roomWaitTimeoutSec: parseInt(
      cliArgs['room-wait-sec'] || process.env.ROOM_WAIT_TIMEOUT_SEC || fileCfg.roomWaitTimeoutSec || '90',
      10
    ),
    autoPickRoomAfterSec: parseInt(
      cliArgs['auto-pick-room-sec'] ??
      process.env.AUTO_PICK_ROOM_AFTER_SEC ??
      fileCfg.autoPickRoomAfterSec ??
      '15',
      10
    ),
    pbsBaseUrl:
      cliArgs['pbs-base-url'] ||
      process.env.PBS_BASE_URL ||
      fileCfg.pbsBaseUrl ||
      'http://psense.lib.net/PBS/_/{ROOM}/PS.cgi',
    observeOnly: Boolean(cliArgs.observe || cliArgs['observe-only'] || fileCfg.observeOnly),
    dryRunPbs: Boolean(cliArgs['dry-run-pbs'] || fileCfg.dryRunPbs),
    logAllChannelEvents: Boolean(
      cliArgs.observe ||
      cliArgs['log-events'] ||
      fileCfg.logAllChannelEvents !== false
    ),
    eventLogPath: path.resolve(
      REPO,
      cliArgs['event-log'] !== undefined
        ? String(cliArgs['event-log'])
        : (fileCfg.eventLogPath || 'data/discord-events.jsonl')
    ),
    traceDir:
      cliArgs['trace-dir'] !== undefined
        ? String(cliArgs['trace-dir'])
        : (fileCfg.traceDir !== undefined ? String(fileCfg.traceDir) : 'data/pbs-traces'),
    anomalyLogPath:
      cliArgs['anomaly-log'] !== undefined
        ? String(cliArgs['anomaly-log'])
        : (fileCfg.anomalyLogPath !== undefined ? String(fileCfg.anomalyLogPath) : 'data/pbs-anomalies.jsonl'),
  };
}

/**
 * Normalize fullwidth ASCII letters/digits (`Ｃ`–`Ｏ`, `１`–`９`) to halfwidth.
 */
function normalizeFullwidthAscii(str) {
  return String(str || '').replace(/[Ａ-Ｚａ-ｚ０-９]/g, ch =>
    String.fromCharCode(ch.charCodeAt(0) - 0xfee0)
  );
}

/**
 * Extract a Castle room (`第1`=`C` through `第6`=`H`, or direct `C`..`O` / `PS.cgi` URL)
 * from a Discord message or `アナウンス [APP]` link/embed.
 *
 * Matches all real Castle Discord patterns seen in `#2000-一撃なし`:
 * - `第1へ`, `第2にて`, `第2`, `では第三でお願いします🙌`, `第一でお願いします`
 * - `アナウンス [APP]` markdown links: `[第1](http://psense.lib.net/PBS/_/C/PS.cgi)` or `第1` through `第6`
 * - Direct URLs: `http://psense.lib.net/PBS/_/C/PS.cgi`
 * - Explicit letter rooms: `C部屋`, `D部屋`, `E部屋`, `【C】`, etc.
 */
function parseRoomFromText(rawText, pbsBaseUrl = 'http://psense.lib.net/PBS/_/{ROOM}/PS.cgi') {
  if (!rawText) return null;
  const text = normalizeFullwidthAscii(String(rawText));

  // 1. Direct psense.lib.net URL (e.g. inside アナウンス [APP] markdown link `[第2](http://psense.lib.net/PBS/_/D/PS.cgi)`)
  const urlMatch = text.match(/https?:\/\/psense\.lib\.net\/PBS\/_\/([C-O])(?:\/PS\.cgi|\/|\b)/i);
  if (urlMatch) {
    const letter = urlMatch[1].toUpperCase();
    return {
      roomLetter: letter,
      roomLabel: LETTER_TO_LABEL[letter] || `${letter}部屋`,
      roomUrl: pbsBaseUrl.replace('{ROOM}', letter),
    };
  }

  // Strip `<@...>` and `<#...>` Discord tags and custom emoji `<:name:id>` before matching
  const stripped = text
    .replace(/<a?:[A-Za-z0-9_]+:\d+>/g, ' ')
    .replace(/<[@#!&][^>]+>/g, ' ');

  // 2. Castle Discord standard notation: `第1` through `第6` (including kanji numerals)
  const daiMatch = stripped.match(/第\s*([1-6一二三四五六])(?![0-9])/);
  if (daiMatch && DAI_TOKEN_TO_ROOM[daiMatch[1]]) {
    const info = DAI_TOKEN_TO_ROOM[daiMatch[1]];
    return {
      roomLetter: info.letter,
      roomLabel: info.label,
      roomUrl: pbsBaseUrl.replace('{ROOM}', info.letter),
    };
  }

  // 3. Explicit letter rooms (`C部屋`, `部屋: D`, `【E】`, etc.)
  const patterns = [
    /(?:^|[^A-Za-z0-9])([C-O])\s*(?:部屋|ルーム|room|鯖|サーバー)(?![A-Za-z])/i,
    /(?:部屋|ルーム|room|割当|割り当て|会場|場所|城)\s*(?:は|が|：|:|->|⇒|=|\s)\s*([C-O])(?![A-Za-z0-9])/i,
    /[【\[「『（(]\s*([C-O])\s*(?:部屋|ルーム|room)?\s*[】\]」』）)]/i,
  ];

  for (const re of patterns) {
    const m = stripped.match(re);
    if (m && m[1]) {
      const letter = m[1].toUpperCase();
      return {
        roomLetter: letter,
        roomLabel: LETTER_TO_LABEL[letter] || `${letter}部屋`,
        roomUrl: pbsBaseUrl.replace('{ROOM}', letter),
      };
    }
  }

  return null;
}

/**
 * Detect whether a message requests mercy / handicap mode (`tier = "mercy"`, 81k + Tier A box)
 * or default mode (`tier = "default"`, 81k + Nash equilibrium mixture).
 */
function detectRequestedTier(rawText) {
  const norm = normalizeFullwidthAscii(String(rawText || '')).toLowerCase();
  const mercyKeywords = [
    '手加減',
    'てかげん',
    'ハンデ',
    '別パ',
    'サブパ',
    'サブ',
    'tier a',
    'tiera',
    'tier-a',
    'tier1',
    'tier 1',
    '弱め',
    'やさしく',
    '優しく',
    'mercy',
    'handicap',
  ];
  for (const kw of mercyKeywords) {
    if (norm.includes(kw)) {
      return 'mercy';
    }
  }
  return 'default';
}

/**
 * Parse a Discord `MESSAGE_CREATE` payload to determine if the bot is being `@mentioned`
 * for a match, and extract `{ tier, room }`.
 */
function parseChallengeMention(msg, botUserId, pbsBaseUrl) {
  if (!msg || !botUserId) return null;
  if (msg.author?.id === botUserId || msg.author?.bot) return null;

  const content = String(msg.content || '');
  const mentionedInArray =
    Array.isArray(msg.mentions) && msg.mentions.some(u => u && u.id === botUserId);
  const mentionedInContent =
    content.includes(`<@${botUserId}>`) || content.includes(`<@!${botUserId}>`);

  if (!mentionedInArray && !mentionedInContent) {
    return null;
  }

  const tier = detectRequestedTier(content);
  const room = parseRoomFromText(content, pbsBaseUrl);
  return {
    tier,
    room,
    challengerId: msg.author?.id || null,
    challengerName:
      msg.member?.nick || msg.author?.global_name || msg.author?.username || 'Challenger',
  };
}

/**
 * Extract all human-readable text and URLs from a Discord message payload, including
 * embeds and components (e.g. `アナウンス [APP]` hyperlink `[第2](http://psense.lib.net/PBS/_/D/PS.cgi)`).
 */
function extractFullMessageText(msg) {
  if (!msg) return '';
  const parts = [msg.content || '', msg.name || ''];
  if (Array.isArray(msg.embeds)) {
    for (const emb of msg.embeds) {
      if (emb.title) parts.push(emb.title);
      if (emb.description) parts.push(emb.description);
      if (emb.url) parts.push(emb.url);
      if (emb.footer?.text) parts.push(emb.footer.text);
      if (Array.isArray(emb.fields)) {
        for (const f of emb.fields) {
          if (f.name) parts.push(f.name);
          if (f.value) parts.push(f.value);
        }
      }
    }
  }
  if (Array.isArray(msg.components)) {
    for (const row of msg.components) {
      for (const comp of row.components || []) {
        if (comp.label) parts.push(comp.label);
        if (comp.url) parts.push(comp.url);
      }
    }
  }
  return parts.filter(Boolean).join('\n');
}

/**
 * Probe Castle Discord rooms `第1 (C)` through `第6 (H)` on `psense.lib.net` to find:
 * 1. A room where a player is already waiting for an opponent (`p1Waiting === true`), OR
 * 2. The first unoccupied room (`page.kind === 'ENTORY' && !page.p1Waiting`).
 */
async function probeCastleDiscordRooms(pbsBaseUrl = 'http://psense.lib.net/PBS/_/{ROOM}/PS.cgi') {
  const client = new PbsHttpClient();
  let firstEmpty = null;

  for (const r of CASTLE_DISCORD_ROOMS) {
    const roomUrl = pbsBaseUrl.replace('{ROOM}', r.letter);
    try {
      const res = await client.get(roomUrl);
      const page = classifyPage(res);
      if (page.kind === 'ENTORY' && page.p1Waiting) {
        return {
          roomLetter: r.letter,
          roomLabel: r.label,
          roomUrl,
          reason: `p1_waiting(${page.p1Name || '?'})`,
        };
      }
      if (!firstEmpty && (page.kind === 'ENTORY' || page.kind === 'FINISHED')) {
        firstEmpty = {
          roomLetter: r.letter,
          roomLabel: r.label,
          roomUrl,
          reason: 'empty_room',
        };
      }
    } catch {
      // Ignore transient probe error on one room
    }
  }

  return firstEmpty || {
    roomLetter: 'C',
    roomLabel: '第1',
    roomUrl: pbsBaseUrl.replace('{ROOM}', 'C'),
    reason: 'fallback_default',
  };
}

// ------------------------------------------------------------------ Discord REST & Gateway Client
class CastleDiscordBot {
  constructor(config) {
    this.config = config;
    this.botUserId = null;
    this.botUsername = null;
    this.ws = null;
    this.heartbeatTimer = null;
    this.lastSeq = null;
    this.sessionId = null;
    this.resumeGatewayUrl = null;
    this.activeMatch = null;      // { roomLetter, roomLabel, roomUrl, challengerName, tier, child }
    this.pendingChallenge = null; // { channelId, messageId, challengerId, challengerName, tier, timer, autoPickTimer }
  }

  log(msg) {
    console.log(`[discord-bot ${new Date().toISOString()}] ${msg}`);
  }

  appendEventLog(eventType, payload, extra = {}) {
    if (!this.config.eventLogPath) return;
    try {
      fs.mkdirSync(path.dirname(this.config.eventLogPath), { recursive: true });
      const record = {
        ts: new Date().toISOString(),
        eventType,
        channelId: payload?.channel_id || payload?.parent_id || payload?.id || null,
        messageId: payload?.id || null,
        author: payload?.author
          ? {
              id: payload.author.id,
              username: payload.author.username,
              global_name: payload.author.global_name,
              bot: Boolean(payload.author.bot),
            }
          : null,
        content: payload?.content || null,
        embeds: payload?.embeds || [],
        components: payload?.components || [],
        referencedMessageId: payload?.message_reference?.message_id || null,
        ...extra,
      };
      fs.appendFileSync(this.config.eventLogPath, JSON.stringify(record) + '\n', { mode: 0o600 });
    } catch {
      // Non-fatal
    }
  }

  async discordRest(method, endpoint, body) {
    const url = `https://discord.com/api/v10${endpoint}`;
    const res = await fetch(url, {
      method,
      headers: {
        Authorization: `Bot ${this.config.discordToken}`,
        'Content-Type': 'application/json',
        'User-Agent': 'NC2000CastleBot/1.0',
      },
      body: body ? JSON.stringify(body) : undefined,
    });
    if (!res.ok) {
      const errText = await res.text().catch(() => '');
      throw new Error(`Discord REST ${method} ${endpoint} failed (${res.status}): ${errText}`);
    }
    if (res.status === 204) return null;
    return res.json();
  }

  async sendChannelMessage(channelId, content, replyToMessageId = null) {
    if (!content) return null;
    const payload = {
      content,
      allowed_mentions: {
        parse: ['users'],
        replied_user: true,
      },
    };
    if (replyToMessageId) {
      payload.message_reference = {
        message_id: replyToMessageId,
        fail_if_not_exists: false,
      };
    }
    try {
      return await this.discordRest('POST', `/channels/${channelId}/messages`, payload);
    } catch (err) {
      this.log(`Warning: failed to send message to channel ${channelId}: ${err.message}`);
      return null;
    }
  }

  clearPendingChallenge() {
    if (this.pendingChallenge) {
      if (this.pendingChallenge.timer) clearTimeout(this.pendingChallenge.timer);
      if (this.pendingChallenge.autoPickTimer) clearTimeout(this.pendingChallenge.autoPickTimer);
    }
    this.pendingChallenge = null;
  }

  async handleDiscordMessageEvent(eventType, msg) {
    if (!msg) return;
    const channelId = msg.channel_id || msg.parent_id || msg.id;
    if (!channelId) return;
    if (msg.author?.id && msg.author.id === this.botUserId) return;

    // Only accept messages in the approved bot channel.
    // The human-only Castle channel must never trigger PBS matches.
    if (String(channelId) !== '989231786288611388') {
      return;
    }

    const fullText = extractFullMessageText(msg);
    const parsedRoom = parseRoomFromText(fullText, this.config.pbsBaseUrl);

    if (this.config.logAllChannelEvents || this.pendingChallenge) {
      this.appendEventLog(eventType, msg, {
        extractedText: fullText,
        parsedRoom,
        pendingChallenge: Boolean(this.pendingChallenge),
      });
    }

    if (this.config.observeOnly) {
      this.log(
        `[OBSERVE ${eventType}] ch=${channelId} author=${msg.author?.username || '?'}(bot=${Boolean(msg.author?.bot)}) ` +
        `text=${JSON.stringify(fullText.replace(/\s+/g, ' ').trim())} ` +
        `-> parsedRoom=${parsedRoom ? `${parsedRoom.roomLabel}/${parsedRoom.roomLetter} (${parsedRoom.roomUrl})` : 'none'}, ` +
        `tier=${detectRequestedTier(fullText)}`
      );
      return;
    }

    if (eventType !== 'MESSAGE_CREATE') return;

    const challenge = parseChallengeMention(
      msg, this.botUserId, this.config.pbsBaseUrl
    );
    if (!challenge) return;

    if (this.activeMatch) {
      this.log(
        `Ignored challenge from "${challenge.challengerName}" because a match is already active in ${this.activeMatch.roomLabel} (${this.activeMatch.roomLetter}).`
      );
      await this.sendChannelMessage(
        channelId,
        `現在 ${this.activeMatch.roomLabel}で対戦中です。終了後にまたお声がけください！`,
        msg.id
      );
      return;
    }

    this.log(
      `Received challenge from "${challenge.challengerName}" ` +
      `in channel ${channelId}; using dedicated room K.`
    );

    if (this.config.replyTriggerText) {
      if (this.config.replyMode === 'mention' && challenge.challengerId) {
        await this.sendChannelMessage(
          channelId,
          `<@${challenge.challengerId}> ${this.config.replyTriggerText}`,
          null
        );
      } else if (this.config.replyMode === 'plain') {
        await this.sendChannelMessage(channelId, this.config.replyTriggerText, null);
      } else {
        await this.sendChannelMessage(channelId, this.config.replyTriggerText, msg.id);
      }
    }

    // Always use room K, including when another room is requested.
    // Do not send the greeting before entry is confirmed.
    this.launchPbsMatch({
      channelId,
      challengerName: challenge.challengerName,
      tier: challenge.tier,
      roomLetter: 'K',
      roomLabel: '\u7b2cK',
      roomUrl: K_ROOM_URL,
    });
  }

  launchPbsMatch({ channelId, challengerName, tier, roomLetter, roomLabel, roomUrl }) {
    if (this.activeMatch) return;
    const displayRoom = roomLabel || LETTER_TO_LABEL[roomLetter] || `${roomLetter}部屋`;

    if (this.config.dryRunPbs) {
      this.log(
        `[DRY-RUN-PBS] Would launch pbs-client.js in ${displayRoom} (${roomLetter}: ${roomUrl}) ` +
        `for challenger "${challengerName}" [tier=${tier}, iters=${this.config.iters}].`
      );
      return;
    }

    const clientScript = path.join(__dirname, 'pbs-client.js');
    const cliArgs = [
      clientScript,
      '--room', roomUrl,
      '--name', this.config.pbsName,
      '--pass', this.config.pbsPass,
      '--msg', this.config.pbsMsg,
      '--tier', tier,
      '--iters', String(this.config.iters),
      '--games', '1',
      '--mode', 'host',
      '--entry-status-ipc',
      '--host-timeout-sec', String(this.config.hostTimeoutSec),
      '--seed', String(Date.now() % 1000000),
    ];
    if (this.config.traceDir) {
      cliArgs.push('--trace-dir', this.config.traceDir);
    }
    if (this.config.anomalyLogPath) {
      cliArgs.push('--anomaly-log', this.config.anomalyLogPath);
    }

    this.log(
      `Spawning pbs-client.js in ${displayRoom} (${roomLetter}: ${roomUrl}) [tier=${tier}, iters=${this.config.iters}]...`
    );

    const child = (this.spawnProcess || spawn)(process.execPath, cliArgs, {
      cwd: REPO,
      stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
    });

    let lastOutput = '';
    child.stdout.on('data', chunk => {
      const text = chunk.toString('utf8');
      lastOutput += text;
      process.stdout.write(text);
    });
    child.stderr.on('data', chunk => {
      const text = chunk.toString('utf8');
      process.stderr.write(text);
    });

    this.activeMatch = {
      channelId,
      challengerName,
      tier,
      roomLetter,
      roomLabel: displayRoom,
      roomUrl,
      child,
    };

    // Receive verified entry status from pbs-client.js.
    let entryResultReceived = false;

    const respondToEntry = async (status, reason) => {
      if (entryResultReceived) return;
      entryResultReceived = true;

      this.log(`PBS entry status=${status}, reason=${reason || 'none'}`);

      if (status === 'success') {
        const greeting =
          '\u3088\u308d\u3057\u304f\u304a\u9858\u3044\u3057\u307e\u3059';
        await this.sendChannelMessage(
          channelId,
          `${greeting}\n${K_ROOM_LINK}`
        );
      } else {
        const unavailable =
          '\u73fe\u5728\u90e8\u5c4b\u304c\u5229\u7528\u3067\u304d\u307e\u305b\u3093';
        await this.sendChannelMessage(channelId, unavailable);
      }
    };

    child.on('message', message => {
      if (!message || message.type !== 'PBS_ENTRY_STATUS') return;
      void respondToEntry(message.status, message.reason);
    });

    child.on('error', error => {
      this.log(`PBS client process error: ${error.message}`);
      void respondToEntry('failed', 'process_error');
    });

    child.on('close', async code => {
      this.activeMatch = null;

      if (!entryResultReceived) {
        await respondToEntry('failed', 'process_closed_before_entry');
      }
      const timedOut = lastOutput.includes('wait timeout') || lastOutput.includes('(TIMEOUT)');
      const playedTurns = /after\s+([1-9]\d*)\s+decisions/.test(lastOutput);
      this.log(
        `pbs-client.js for ${displayRoom} (${roomLetter}) exited with code ${code} (playedTurns=${playedTurns}, timedOut=${timedOut}).`
      );

      if (playedTurns && !timedOut && this.config.postMatchThanksText) {
        await this.sendChannelMessage(channelId, this.config.postMatchThanksText);
      }
    });
  }

  startGateway() {
    const gatewayUrl = 'wss://gateway.discord.gg/?v=10&encoding=json';
    this.log(
      `Connecting to Discord Gateway (${gatewayUrl}) [observeOnly=${this.config.observeOnly}, dryRunPbs=${this.config.dryRunPbs}, replyMode=${this.config.replyMode}]...`
    );
    const ws = new WebSocket(gatewayUrl);
    this.ws = ws;

    ws.on('open', () => {
      this.log('WebSocket connected to Discord Gateway.');
    });

    ws.on('message', raw => {
      let pkt;
      try {
        pkt = JSON.parse(raw.toString('utf8'));
      } catch {
        return;
      }

      if (pkt.s !== null && pkt.s !== undefined) {
        this.lastSeq = pkt.s;
      }

      switch (pkt.op) {
        case 10: {
          const intervalMs = pkt.d?.heartbeat_interval || 41250;
          if (this.heartbeatTimer) clearInterval(this.heartbeatTimer);
          this.heartbeatTimer = setInterval(() => {
            if (this.ws && this.ws.readyState === WebSocket.OPEN) {
              this.ws.send(JSON.stringify({ op: 1, d: this.lastSeq }));
            }
          }, intervalMs);

          const intents = (1 << 0) | (1 << 9) | (1 << 12) | (1 << 15);
          ws.send(
            JSON.stringify({
              op: 2,
              d: {
                token: this.config.discordToken,
                intents,
                properties: {
                  os: process.platform,
                  browser: 'nc2000-castle-bot',
                  device: 'nc2000-castle-bot',
                },
              },
            })
          );
          break;
        }
        case 1: {
          ws.send(JSON.stringify({ op: 1, d: this.lastSeq }));
          break;
        }
        case 7: {
          this.log('Discord Gateway requested reconnect (op 7).');
          ws.close();
          break;
        }
        case 9: {
          this.log('Discord Gateway reported invalid session (op 9), reconnecting in 5s...');
          this.sessionId = null;
          ws.close();
          break;
        }
        case 0: {
          if (pkt.t === 'READY') {
            this.botUserId = pkt.d?.user?.id || null;
            this.botUsername = pkt.d?.user?.username || 'Bot';
            this.sessionId = pkt.d?.session_id || null;
            this.resumeGatewayUrl = pkt.d?.resume_gateway_url || null;
            this.log(`READY as ${this.botUsername} (id=${this.botUserId}). Watching for @mentions...`);
          } else if (
            pkt.t === 'MESSAGE_CREATE' ||
            pkt.t === 'MESSAGE_UPDATE' ||
            pkt.t === 'THREAD_CREATE'
          ) {
            this.handleDiscordMessageEvent(pkt.t, pkt.d).catch(err => {
              this.log(`Error handling ${pkt.t}: ${err.message || err}`);
            });
          }
          break;
        }
        default:
          break;
      }
    });

    ws.on('close', (code, reason) => {
      if (this.heartbeatTimer) {
        clearInterval(this.heartbeatTimer);
        this.heartbeatTimer = null;
      }
      this.log(`Gateway WebSocket closed (code=${code}, reason=${reason || 'none'}). Reconnecting in 5s...`);
      if (code === 4004) {
        console.error(
          '[discord-bot] FATAL: Discord rejected token (4004 Authentication Failed). Check DISCORD_TOKEN / pbs-bot.config.json.'
        );
        process.exit(1);
      }
      if (code === 4014) {
        console.error(
          '[discord-bot] FATAL: Disallowed intent (4014). Enable "MESSAGE CONTENT INTENT" in Discord Developer Portal -> Bot -> Privileged Gateway Intents.'
        );
        process.exit(1);
      }
      setTimeout(() => this.startGateway(), 5000);
    });

    ws.on('error', err => {
      this.log(`Gateway WebSocket error: ${err.message || err}`);
    });
  }
}

async function runSelfTest() {
  const assert = require('assert');

  // Verify exact room mappings from the Castle Discord screenshots
  assert.deepStrictEqual(parseRoomFromText('第1へ'), {
    roomLetter: 'C',
    roomLabel: '第1',
    roomUrl: 'http://psense.lib.net/PBS/_/C/PS.cgi',
  });
  assert.deepStrictEqual(parseRoomFromText('第2にて'), {
    roomLetter: 'D',
    roomLabel: '第2',
    roomUrl: 'http://psense.lib.net/PBS/_/D/PS.cgi',
  });
  assert.deepStrictEqual(parseRoomFromText('ありがとうございます\n第2'), {
    roomLetter: 'D',
    roomLabel: '第2',
    roomUrl: 'http://psense.lib.net/PBS/_/D/PS.cgi',
  });
  assert.deepStrictEqual(parseRoomFromText('ありがとうございます！では第三でお願いします🙌'), {
    roomLetter: 'E',
    roomLabel: '第3',
    roomUrl: 'http://psense.lib.net/PBS/_/E/PS.cgi',
  });
  assert.deepStrictEqual(parseRoomFromText('[第3](http://psense.lib.net/PBS/_/E/PS.cgi)'), {
    roomLetter: 'E',
    roomLabel: '第3',
    roomUrl: 'http://psense.lib.net/PBS/_/E/PS.cgi',
  });

  assert.deepStrictEqual(parseRoomFromText('第4'), {
    roomLetter: 'F',
    roomLabel: '第4',
    roomUrl: 'http://psense.lib.net/PBS/_/F/PS.cgi',
  });
  assert.deepStrictEqual(parseRoomFromText('第５'), {
    roomLetter: 'G',
    roomLabel: '第5',
    roomUrl: 'http://psense.lib.net/PBS/_/G/PS.cgi',
  });
  assert.deepStrictEqual(parseRoomFromText('第六'), {
    roomLetter: 'H',
    roomLabel: '第6',
    roomUrl: 'http://psense.lib.net/PBS/_/H/PS.cgi',
  });
  assert.deepStrictEqual(
    parseRoomFromText('[第6](http://psense.lib.net/PBS/_/H/PS.cgi)'),
    {
      roomLetter: 'H',
      roomLabel: '第6',
      roomUrl: 'http://psense.lib.net/PBS/_/H/PS.cgi',
    }
  );

  const fwWin = String.fromCodePoint(
    0xff37, 0xff29, 0xff2e, 0xff01
  );

  const roomEWinner =
    '\u30df\u30dc\u30c4\u30c0\u30de';

  assert.strictEqual(
    resolveBotOutcomeFromFinishedHtml(
      `<p>puniu3pbs ${fwWin}</p>`,
      'puniu3pbs',
      'human'
    ),
    'WIN'
  );

  assert.strictEqual(
    resolveBotOutcomeFromFinishedHtml(
      `<p>${roomEWinner} ${fwWin}</p>`,
      'puniu3pbs',
      roomEWinner
    ),
    'LOSE'
  );
  const cfg = loadConfig({ 'dry-run-pbs': true, 'event-log': '', 'auto-pick-room-sec': '0' });
  cfg.eventLogPath = '';
  const bot = new CastleDiscordBot(cfg);
  bot.botUserId = '999999';

  const sentMessages = [];
  const launchedMatches = [];
  bot.sendChannelMessage = async (channelId, content, replyToMessageId) => {
    sentMessages.push({ channelId, content, replyToMessageId });
    return { id: 'sent-1' };
  };
  bot.launchPbsMatch = params => {
    launchedMatches.push(params);
  };

  // Fork-specific routing tests: only the approved channel may start matches.
  const approvedChannel = '989231786288611388';
  cfg.observeOnly = false;

  await bot.handleDiscordMessageEvent('MESSAGE_CREATE', {
    id: 'msg-old',
    channel_id: 'ch-2000',
    author: { id: 'user-1', username: 'User1', bot: false },
    content: '<@999999> battle',
    mentions: [{ id: '999999' }],
  });

  assert.strictEqual(sentMessages.length, 0);
  assert.strictEqual(launchedMatches.length, 0);

  // No room specified: reply once, then launch the K-room host.
  await bot.handleDiscordMessageEvent('MESSAGE_CREATE', {
    id: 'msg-new',
    channel_id: approvedChannel,
    author: { id: 'user-1', username: 'User1', bot: false },
    content: '<@999999> battle',
    mentions: [{ id: '999999' }],
  });

  assert.strictEqual(sentMessages.length, 1);
  assert.strictEqual(
    sentMessages[0].content,
    '\u5fdc\u6226\u3057\u307e\u3059'
  );
  assert.strictEqual(sentMessages[0].replyToMessageId, 'msg-new');
  assert.strictEqual(launchedMatches.length, 1);
  assert.strictEqual(launchedMatches[0].roomLetter, 'K');
  assert.strictEqual(launchedMatches[0].roomUrl, K_ROOM_URL);
  assert.strictEqual(launchedMatches[0].tier, 'default');

  // An explicit human-only room request must still route to K.
  // The mercy tier must remain supported.
  await bot.handleDiscordMessageEvent('MESSAGE_CREATE', {
    id: 'msg-mercy',
    channel_id: approvedChannel,
    author: { id: 'user-2', username: 'User2', bot: false },
    content: '<@999999> \u7b2c2 \u624b\u52a0\u6e1b',
    mentions: [{ id: '999999' }],
  });

  assert.strictEqual(sentMessages.length, 2);
  assert.strictEqual(launchedMatches.length, 2);
  assert.strictEqual(launchedMatches[1].roomLetter, 'K');
  assert.strictEqual(launchedMatches[1].roomUrl, K_ROOM_URL);
  assert.strictEqual(launchedMatches[1].tier, 'mercy');

  // No greeting or room URL is sent before successful PBS entry.
  assert.strictEqual(
    sentMessages.every(m =>
      m.content === '\u5fdc\u6226\u3057\u307e\u3059'
    ),
    true
  );

  // Fork-only IPC tests: verify host mode and entry notifications.
  const { EventEmitter } = require('node:events');
  const { PassThrough } = require('node:stream');

  const testEntryReply = (status, expectedReply) => {
    const testBot = new CastleDiscordBot({
      ...cfg,
      dryRunPbs: false,
    });
    const child = new EventEmitter();
    child.stdout = new PassThrough();
    child.stderr = new PassThrough();

    const replies = [];
    let invocation = null;

    testBot.sendChannelMessage = async (channel, content) => {
      assert.strictEqual(channel, approvedChannel);
      replies.push(content);
      return { id: 'mock-reply' };
    };

    testBot.spawnProcess = (command, argv, options) => {
      invocation = { command, argv, options };
      return child;
    };

    testBot.launchPbsMatch({
      channelId: approvedChannel,
      challengerName: 'TestPlayer',
      tier: 'default',
      roomLetter: 'K',
      roomLabel: '\u7b2cK',
      roomUrl: K_ROOM_URL,
    });

    assert.ok(invocation);
    assert.strictEqual(
      invocation.argv[invocation.argv.indexOf('--room') + 1],
      K_ROOM_URL
    );
    assert.strictEqual(
      invocation.argv[invocation.argv.indexOf('--mode') + 1],
      'host'
    );
    assert.ok(invocation.argv.includes('--entry-status-ipc'));
    assert.ok(invocation.options.stdio.includes('ipc'));

    // No greeting or failure message before PBS responds.
    assert.strictEqual(replies.length, 0);

    if (status === 'closed') {
      child.emit('close', 1);
    } else {
      child.emit('message', {
        type: 'PBS_ENTRY_STATUS',
        status,
        reason: status === 'success' ? 'WAITING_ENTRY' : 'room_busy',
      });

      // Duplicate status notifications must not duplicate replies.
      child.emit('message', {
        type: 'PBS_ENTRY_STATUS',
        status,
        reason: 'duplicate',
      });
      child.emit('close', status === 'success' ? 0 : 1);
    }

    assert.deepStrictEqual(replies, [expectedReply]);
    child.stdout.destroy();
    child.stderr.destroy();
  };

  const successGreeting =
    '\u3088\u308d\u3057\u304f\u304a\u9858\u3044\u3057\u307e\u3059';
  const failureGreeting =
    '\u73fe\u5728\u90e8\u5c4b\u304c\u5229\u7528\u3067\u304d\u307e\u305b\u3093';

  testEntryReply(
    'success',
    `${successGreeting}\n${K_ROOM_LINK}`
  );
  testEntryReply('failed', failureGreeting);
  testEntryReply('closed', failureGreeting);

  console.log('[discord-bot] K-room IPC tests passed.');

  // --- LLOG & PositionSpec State Reconstruction Self-Tests ---
  const wasm = require(path.join(REPO, 'crates/wasm/pkg-node/nc2000_wasm.js'));
  const wasmDex = new wasm.Dex();
  const beliefPoolPath = path.join(REPO, 'data/belief-pool-v3/belief-pool.json');
  const beliefPoolJson = fs.readFileSync(beliefPoolPath, 'utf8');

  const previewSides = [
    [
      { species: 'Snorlax', speciesId: 'snorlax', level: 55, gender: 'M', item_flag: true },
      { species: 'Zapdos', speciesId: 'zapdos', level: 50, gender: 'N', item_flag: true },
      { species: 'Suicune', speciesId: 'suicune', level: 50, gender: 'N', item_flag: true },
      { species: 'Marowak', speciesId: 'marowak', level: 50, gender: 'M', item_flag: true },
      { species: 'Cloyster', speciesId: 'cloyster', level: 50, gender: 'M', item_flag: true },
      { species: 'Misdreavus', speciesId: 'misdreavus', level: 50, gender: 'M', item_flag: true },
    ],
    [
      { species: 'Raikou', speciesId: 'raikou', level: 50, gender: 'N', item_flag: true },
      { species: 'Exeggutor', speciesId: 'exeggutor', level: 50, gender: 'M', item_flag: true },
      { species: 'Tyranitar', speciesId: 'tyranitar', level: 55, gender: 'M', item_flag: true },
      { species: 'Skarmory', speciesId: 'skarmory', level: 50, gender: 'M', item_flag: true },
      { species: 'Miltank', speciesId: 'miltank', level: 50, gender: 'F', item_flag: true },
      { species: 'Gengar', speciesId: 'gengar', level: 50, gender: 'M', item_flag: true },
    ],
  ];

  // Test 1: <HR> turn progression, side conditions (reflect, safeguard, spikes), weather, and 5-turn expiration
  const sampleLlog = `
    <HR>
    <P ALIGN="left">カビゴン をくりだした！<BR></P>
    <P ALIGN="right">ライコウ をくりだした！<BR></P>
    <HR>
    <P ALIGN="right">ライコウ の リフレクター！<BR>ひかりのかべが あらわれた！<BR></P>
    <P ALIGN="left">カビゴン の しんぴのまもり！<BR>しんぴのヴェールに つつまれた！<BR></P>
    <HR>
    <P ALIGN="right">ライコウ の あまごい！<BR>あめが ふりだした！<BR></P>
    <P ALIGN="left">カビゴン の のしかかり！<BR>ライコウ は まひして 技が 出にくくなった！<BR></P>
    <P ALIGN="center">あめが ふりつづいている<BR></P>
    <HR>
    <P ALIGN="left">カビゴン の はらだいこ！<BR>カビゴン の こうげきが ぐーんとあがった！<BR></P>
    <P ALIGN="right">ライコウ は しびれて うごけない！<BR></P>
    <P ALIGN="center">あめが ふりつづいている<BR></P>
    <HR>
    <P ALIGN="left">カビゴン の ねむる！<BR>カビゴン は けんこうになって ねむってしまった！<BR>カビゴン は はっかのみで めをさました！<BR></P>
    <P ALIGN="right">ライコウ の １０まんボルト！<BR></P>
    <P ALIGN="center">あめが ふりつづいている<BR></P>
    <HR>
    <P ALIGN="left">カビゴン の のしかかり！<BR></P>
    <P ALIGN="right">ライコウ の ほえる！<BR>サンダー が ひきずりだされた！<BR></P>
    <P ALIGN="center">あめが ふりつづいている<BR></P>
  `;
  const parsed5 = parseLlogHtml(sampleLlog, previewSides);
  const parsedTurn6 = parseLlogHtml(sampleLlog + '<HR>', previewSides);
  assert.strictEqual(parsed5.turn, 5);
  assert.strictEqual(parsedTurn6.turn, 6);
  // At Turn 5, Reflect & Safeguard started on Turn 1 are still active (5 - 1 = 4 < 5), rain has 4 upkeeps
  assert.strictEqual(parsed5.sides[1].conditions.get('reflect'), 1);
  assert.strictEqual(parsed5.sides[0].conditions.get('safeguard'), 1);
  assert.strictEqual(parsed5.weather.key, 'raindance');
  assert.strictEqual(parsed5.weather.upkeeps, 4);
  // At Turn 6 (after 5th turn completed), Reflect & Safeguard started on Turn 1 must expire automatically
  assert.strictEqual(parsedTurn6.sides[1].conditions.has('reflect'), false);
  assert.strictEqual(parsedTurn6.sides[0].conditions.has('safeguard'), false);
  // Snorlax was roared out for Zapdos on Turn 5 -> Zapdos is active at slot 1 with switchInTurn = 5, appearCount = 1
  assert.strictEqual(parsed5.sides[0].activeSlot, 1);
  assert.strictEqual(parsed5.sides[0].mons[1].switchInTurn, 5);
  assert.strictEqual(parsed5.sides[0].mons[1].appearCount, 1);
  assert.strictEqual(parsed5.sides[0].mons[0].itemCurrent, '');
  assert.strictEqual(parsed5.sides[0].mons[0].itemOriginal, 'mintberry');
  // Raikou was paralyzed on Turn 2 -> compPar = true
  assert.strictEqual(parsed5.sides[1].mons[0].status, 'par');
  assert.strictEqual(parsed5.sides[1].mons[0].compPar, true);

  // Test 2: Side conditions & field effects update even when previewSides is empty (actorSlot unresolved)
  const unresolvedHtml = `
    <HR>
    <HR>
    <P ALIGN="right">フォレトス の まきびし！<BR></P>
    <P ALIGN="left">スターミー の ひかりのかべ！<BR></P>
    <HR>
    <P ALIGN="right">フォレトス の すなあらし！<BR></P>
  `;
  const unresolvedParsed = parseLlogHtml(unresolvedHtml, [[], []]);
  assert.strictEqual(unresolvedParsed.turn, 2);
  assert.strictEqual(unresolvedParsed.sides[0].conditions.get('spikes'), 1);
  assert.strictEqual(unresolvedParsed.sides[0].conditions.get('lightscreen'), 1);
  assert.strictEqual(unresolvedParsed.weather.key, 'sandstorm');
  assert.strictEqual(unresolvedParsed.sides[1].actedThisTurn, true);
  assert.strictEqual(unresolvedParsed.sides[1].lastMove, 'sandstorm');

  // Test 3: Volatiles, Encore/Disable, Toxic counter, Companions, Multi-turn moves, Baton Pass, and WASM PositionSpec validation
  const complexLlog = `
    <HR>
    <P ALIGN="left">サンダー をくりだした！<BR></P>
    <P ALIGN="right">ナッシー をくりだした！<BR></P>
    <HR>
    <P ALIGN="left">サンダー の みがわり！<BR>サンダー の みがわりが あらわれた！<BR></P>
    <P ALIGN="right">ナッシー の やどりぎのタネ！<BR>サンダー に タネが うえつけられた！<BR></P>
    <P ALIGN="left">サンダー の たべのこしで たいりょくが かいふくした！<BR></P>
    <HR>
    <P ALIGN="left">サンダー の かげぶんしん！<BR>サンダー の かいひりつが あがった！<BR></P>
    <P ALIGN="right">ナッシー の どくどく！<BR>サンダー は もうどくを あびた！<BR></P>
    <P ALIGN="left">サンダー は どくのダメージを うけた！<BR></P>
    <HR>
    <P ALIGN="left">サンダー の バトンタッチ！<BR></P>
  `;
  const complexParsed = parseLlogHtml(complexLlog, previewSides);
  assert.strictEqual(complexParsed.turn, 3);
  assert.strictEqual(complexParsed.sides[0].pendingBp, true);
  assert.strictEqual(complexParsed.sides[0].mons[1].boosts[6], 1);
  assert.strictEqual(complexParsed.sides[0].mons[1].volatiles.has('substitute'), true);
  assert.strictEqual(complexParsed.sides[0].mons[1].volatiles.has('leechseed'), true);
  assert.strictEqual(complexParsed.sides[0].mons[1].toxCounter, 1);

  const makeFullSet = s => ({
    name: s.species,
    species: s.species,
    level: s.level,
    item: s.item,
    ability: 'No Ability',
    moves: s.moves,
    gender: s.gender,
    happiness: 255,
    evs: { hp: 255, atk: 255, def: 255, spa: 255, spd: 255, spe: 255 },
    ivs: { hp: 31, atk: 31, def: 31, spa: 31, spd: 31, spe: 31 },
  });
  const ownSets = [
    { species: 'Snorlax', level: 55, gender: 'M', item: 'Mint Berry', moves: ['Body Slam', 'Belly Drum', 'Rest', 'Earthquake'] },
    { species: 'Zapdos', level: 50, gender: 'N', item: 'Leftovers', moves: ['Thunderbolt', 'Substitute', 'Double Team', 'Baton Pass'] },
    { species: 'Suicune', level: 50, gender: 'N', item: 'Leftovers', moves: ['Surf', 'Ice Beam', 'Rest', 'Sleep Talk'] },
    { species: 'Marowak', level: 50, gender: 'M', item: 'Thick Club', moves: ['Earthquake', 'Rock Slide', 'Swords Dance', 'Hidden Power Bug'] },
    { species: 'Cloyster', level: 50, gender: 'M', item: 'Leftovers', moves: ['Spikes', 'Surf', 'Explosion', 'Toxic'] },
    { species: 'Misdreavus', level: 50, gender: 'M', item: 'Leftovers', moves: ['Mean Look', 'Perish Song', 'Protect', 'Thunderbolt'] },
  ].map(makeFullSet);

  const initSeenFoe = () => previewSides[1].map(pm => ({
    species: pm.species,
    speciesId: pm.speciesId,
    level: pm.level,
    gender: pm.gender,
    itemFlag: pm.item_flag,
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

  const seenFoeMons = initSeenFoe();
  mergeLlogIntoSeenFoeMons({
    llog: complexParsed,
    foeSide: 1,
    seenFoeMons,
    previewSides,
    battleTurn: null,
    turnNum: 3,
    isForceSwitch: true,
    lastFoeActiveSlot: 1,
  });

  const specBp = buildAndSanitizePositionSpec({
    mySide: 0,
    turn: 3,
    ownSets,
    previewSides,
    pickedRosterSlots: [1, 0, 2],
    battleTurn: {
      turn: 3,
      sides: [
        { speciesId: 'zapdos', hp: 120, maxHp: 197, status: 'tox' },
        { speciesId: 'exeggutor', hp: 202, maxHp: 202, status: '' },
      ],
      options: [
        { kind: 'switch', value: 'C2', slotIndex: 2, speciesId: 'snorlax', hp: 312, maxHp: 312 },
        { kind: 'switch', value: 'C3', slotIndex: 3, speciesId: 'suicune', hp: 207, maxHp: 207 },
      ],
      trapped: false,
    },
    pdat: null,
    llog: complexParsed,
    seenFoeMons,
    teamPreview: false,
    forceSwitch: true,
  });
  // Mid-turn Baton Pass replacement must have upkeep_this_turn = false and pending_bp = true
  assert.strictEqual(specBp.upkeep_this_turn, false);
  assert.strictEqual(specBp.sides[0].pending_bp, true);
  assert.strictEqual(specBp.sides[0].mons[1].tox_counter, 1);

  // Validate with actual Rust WASM ProtocolSearcher (`#[serde(deny_unknown_fields)]` + `PositionSpec::check()`)
  const searcher = new wasm.ProtocolSearcher(wasmDex, 0, beliefPoolJson, 42, 0.4);
  searcher.setPosition(JSON.stringify(specBp));
  searcher.step(16);
  const bpBest = searcher.best();
  assert.ok(bpBest.startsWith('switch '));
  if (typeof searcher.free === 'function') searcher.free();

  // Test 4: Final LLOG merge on match end (e.g., Selfdestruct / Explosion fainting on final turn)
  const finalLlog = `
    <HR>
    <P ALIGN="left">カビゴン をくりだした！<BR></P>
    <P ALIGN="right">ナッシー をくりだした！<BR></P>
    <HR>
    <P ALIGN="right">ナッシー の だいばくはつ！<BR>ナッシー は たおれた！<BR>カビゴン は たおれた！<BR></P>
  `;
  const finalParsed = parseLlogHtml(finalLlog, previewSides);
  const finalSeen = initSeenFoe();
  const finalMerged = mergeLlogIntoSeenFoeMons({
    llog: finalParsed,
    foeSide: 1,
    seenFoeMons: finalSeen,
    previewSides,
    battleTurn: null,
    turnNum: finalParsed.turn,
    isForceSwitch: false,
    lastFoeActiveSlot: 1,
  });
  const exeggutorObs = finalSeen[1];
  assert.ok(exeggutorObs);
  assert.strictEqual(exeggutorObs.appeared, true);
  assert.strictEqual(exeggutorObs.fainted, true);
  assert.strictEqual(exeggutorObs.hp, 0);
  assert.ok(exeggutorObs.revealedMoves.includes('explosion'));
  assert.strictEqual(exeggutorObs.uses.explosion, 1);

  const oppDb = { schema: 'pbs-opponent-db-v1', updatedAt: null, players: {}, shells: {} };
  recordOpponentObservation({
    opponentDb: oppDb,
    foeName: 'テスト相手',
    foePreviewMons: previewSides[1],
    seenFoeMons: finalSeen,
    llogFoeMons: finalMerged.llogFoeMons,
    outcome: 'WIN',
  });
  assert.strictEqual(oppDb.players['テスト相手'].games, 1);
  const shellBuckets = Object.values(oppDb.players['テスト相手'].shells);
  assert.strictEqual(shellBuckets.length, 1);
  assert.strictEqual(shellBuckets[0].mons['exeggutor:50'].moves.explosion, 1);

  // Test 5: resolveLegalPbsMoveOption diagnostic trace
  const specMove = buildAndSanitizePositionSpec({
    mySide: 0,
    turn: 2,
    ownSets,
    previewSides,
    pickedRosterSlots: [0, 1, 2],
    battleTurn: {
      turn: 2,
      sides: [
        { speciesId: 'snorlax', hp: 280, maxHp: 312, status: '' },
        { speciesId: 'raikou', hp: 197, maxHp: 197, status: '' },
      ],
      options: [
        { kind: 'move', value: 'W1', slotIndex: 1, moveId: 'bodyslam', pp: 23 },
        { kind: 'move', value: 'W4', slotIndex: 4, moveId: 'earthquake', pp: 16 },
        { kind: 'switch', value: 'C2', slotIndex: 2, speciesId: 'zapdos', hp: 197, maxHp: 197 },
      ],
      trapped: false,
    },
    pdat: null,
    llog: parsed5,
    seenFoeMons,
    teamPreview: false,
    forceSwitch: false,
  });
  const resolvedOpt = resolveLegalPbsMoveOption({
    bestChoice: 'move bodyslam',
    rootPolicy: { actions: [{ input: 'move bodyslam', visits: 100 }] },
    spec: specMove,
    pickedRosterSlots: [0, 1, 2],
    options: [
      { kind: 'move', value: 'W1', slotIndex: 1, moveId: 'bodyslam', pp: 23 },
      { kind: 'move', value: 'W4', slotIndex: 4, moveId: 'earthquake', pp: 16 },
    ],
    ownSets,
    isRandom: false,
  });
  assert.strictEqual(resolvedOpt.value, 'W1');
  assert.strictEqual(resolvedOpt.mappingPath, 'direct:move-slot');
  assert.strictEqual(resolvedOpt.matchedAction, 'move bodyslam');

  // Verify normal move turn PositionSpec is accepted by WASM ProtocolSearcher
  const searcherMove = new wasm.ProtocolSearcher(wasmDex, 0, beliefPoolJson, 43, 0.4);
  searcherMove.setPosition(JSON.stringify(specMove));
  searcherMove.step(16);
  assert.ok(searcherMove.best().length > 0);
  if (typeof searcherMove.free === 'function') searcherMove.free();

  // Test 6: Encore/Disable, Sleep Talk called move (revealed without PP deduction), Mean Look trapped, and post-faint forceSwitch
  const specialMechanicsLlog = `
    <HR>
    <P ALIGN="left">スイクン をくりだした！<BR></P>
    <P ALIGN="right">ゲンガー をくりだした！<BR></P>
    <HR>
    <P ALIGN="right">ゲンガー の くろいまなざし！<BR>スイクン は にげられなくなった！<BR></P>
    <P ALIGN="left">スイクン の ねむる！<BR>スイクン は けんこうになって ねむってしまった！<BR></P>
    <HR>
    <P ALIGN="right">ゲンガー の かなしばり！<BR>スイクン の ねむるを 封じた！<BR></P>
    <P ALIGN="left">スイクン は ぐうぐう ねむっている<BR>スイクン の <B>ねごと</B>！<BR>スイクン の <B>れいとうビーム</B>！<BR></P>
  `;
  const specialParsed = parseLlogHtml(specialMechanicsLlog, previewSides);
  assert.strictEqual(specialParsed.turn, 2);
  // Sleep Talk deducted 1 PP from sleeptalk, 0 PP from icebeam, and revealed both
  assert.strictEqual(specialParsed.sides[0].mons[2].uses.sleeptalk, 1);
  assert.strictEqual(specialParsed.sides[0].mons[2].uses.icebeam, undefined);
  assert.strictEqual(specialParsed.sides[0].mons[2].revealedMoves.has('sleeptalk'), true);
  assert.strictEqual(specialParsed.sides[0].mons[2].revealedMoves.has('icebeam'), true);
  assert.strictEqual(specialParsed.sides[0].mons[2].volatiles.has('trapped'), true);
  assert.strictEqual(specialParsed.sides[0].mons[2].volatiles.get('disable').move, 'rest');

  const specSpecial = buildAndSanitizePositionSpec({
    mySide: 0,
    turn: 3,
    ownSets,
    previewSides,
    pickedRosterSlots: [2, 0, 1],
    battleTurn: {
      turn: 3,
      sides: [
        { speciesId: 'suicune', hp: 207, maxHp: 207, status: 'slp' },
        { speciesId: 'gengar', hp: 130, maxHp: 167, status: '' },
      ],
      options: [
        { kind: 'move', value: 'W1', slotIndex: 1, moveId: 'surf', pp: 24 },
        { kind: 'move', value: 'W2', slotIndex: 2, moveId: 'icebeam', pp: 16 },
        { kind: 'move', value: 'W3', slotIndex: 3, moveId: 'rest', pp: 15 },
        { kind: 'move', value: 'W4', slotIndex: 4, moveId: 'sleeptalk', pp: 15 },
      ],
      trapped: false,
    },
    pdat: null,
    llog: specialParsed,
    seenFoeMons: initSeenFoe(),
    teamPreview: false,
    forceSwitch: false,
  });
  assert.strictEqual(specSpecial.trapped, true);
  const restPpEntry = specSpecial.sides[0].mons[2].pp.find(p => p.move === 'rest');
  assert.strictEqual(restPpEntry.disabled, true);
  const searcherSpecial = new wasm.ProtocolSearcher(wasmDex, 0, beliefPoolJson, 44, 0.4);
  searcherSpecial.setPosition(JSON.stringify(specSpecial));
  searcherSpecial.step(16);
  assert.ok(searcherSpecial.best().startsWith('move '));
  if (typeof searcherSpecial.free === 'function') searcherSpecial.free();

  // Test 7: Raw Trace Archiving & Cross-Source Parity Audit (`tools/pbs-parity-audit.js`)
  const rawCheckHtml = `
    <TABLE>
      <TR><TH>puniu3pbs</TH><TD ROWSPAN="2">VS</TD><TH>テスト相手</TH></TR>
      <TR>
        <TD VAlign="middle">
          カビゴン♂ LV55 ●<BR>
          サンダー LV50 ●<BR>
          スイクン LV50 ●<BR>
          ガラガラ♂ LV50 ●<BR>
          パルシェン♂ LV50 ●<BR>
          ムウマ♂ LV50 ●<BR>
        </TD>
        <TD VAlign="middle">
          ライコウ LV50 ●<BR>
          ナッシー♂ LV50 ●<BR>
          バンギラス♂ LV55 ●<BR>
          エアームド♂ LV50 ●<BR>
          ミルタンク♀ LV50 ●<BR>
          ゲンガー♂ LV50 ●<BR>
        </TD>
      </TR>
    </TABLE>
  `;
  const rawTurn2Html = `
    <INPUT TYPE="hidden" NAME="NOW" VALUE="1">
    <TABLE>
      <TR><TD>カビゴン♂ L55</TD><TD>ライコウ L50</TD></TR>
      <TR><TD>ＨＰ 280/312</TD><TD>ＨＰ 197/197</TD></TR>
      <TR><TD>通常</TD><TD>通常</TD></TR>
    </TABLE>
    <SELECT NAME="MOVE">
      <OPTION VALUE="W1">のしかかり [23]
      <OPTION VALUE="W2">はらだいこ [16]
      <OPTION VALUE="W3">ねむる [16]
      <OPTION VALUE="W4">じしん [16]
      <OPTION VALUE="C2">サンダー [197/197]
      <OPTION VALUE="C3">スイクン [207/207]
    </SELECT>
  `;
  const rawPdatTurn2Html = `
    <TABLE>
      <TR><TD>55</TD><TD>カビゴン♂</TD><TD>280／312</TD><TD>　</TD><TD>のしかかり(23)</TD><TD>はらだいこ(16)</TD><TD>ねむる(16)</TD><TD>じしん(16)</TD><TD>はっかのみ</TD></TR>
      <TR><TD>50</TD><TD>サンダー</TD><TD>197／197</TD><TD>　</TD><TD>１０まんボルト(24)</TD><TD>みがわり(16)</TD><TD>かげぶんしん(24)</TD><TD>バトンタッチ(64)</TD><TD>たべのこし</TD></TR>
      <TR><TD>50</TD><TD>スイクン</TD><TD>207／207</TD><TD>　</TD><TD>なみのり(24)</TD><TD>れいとうビーム(16)</TD><TD>ねむる(16)</TD><TD>ねごと(16)</TD><TD>たべのこし</TD></TR>
    </TABLE>
    相手の残り：3
  `;
  const rawLlogTurn2Html = `
    <HR>
    <P ALIGN="left">カビゴン をくりだした！<BR></P>
    <P ALIGN="right">ライコウ をくりだした！<BR></P>
    <HR>
    <P ALIGN="right">ライコウ の リフレクター！<BR>ひかりのかべが あらわれた！<BR></P>
    <P ALIGN="left">カビゴン の のしかかり！<BR></P>
  `;
  const parsedCheck = parseTeamPreviewHtml(rawCheckHtml);
  const tracePreviewSides = [parsedCheck.p1Mons, parsedCheck.p2Mons];
  const parsedTurn2Bt = parseBattleTurnHtml(rawTurn2Html);
  const parsedTurn2Pdat = parsePdatHtml(rawPdatTurn2Html);
  const parsedTurn2Llog = parseLlogHtml(rawLlogTurn2Html, tracePreviewSides);
  const turn2SeenFoeBefore = initSeenFoe();
  const turn2SeenFoe = JSON.parse(JSON.stringify(turn2SeenFoeBefore));
  mergeLlogIntoSeenFoeMons({
    llog: parsedTurn2Llog,
    foeSide: 1,
    seenFoeMons: turn2SeenFoe,
    previewSides: tracePreviewSides,
    battleTurn: parsedTurn2Bt,
    turnNum: 2,
    isForceSwitch: false,
    lastFoeActiveSlot: 0,
  });
  const cleanSpecTurn2 = buildAndSanitizePositionSpec({
    mySide: 0,
    turn: 2,
    ownSets,
    previewSides: tracePreviewSides,
    pickedRosterSlots: [0, 1, 2],
    battleTurn: parsedTurn2Bt,
    pdat: parsedTurn2Pdat,
    llog: parsedTurn2Llog,
    seenFoeMons: turn2SeenFoe,
    teamPreview: false,
    forceSwitch: false,
  });
  const previewSpec = buildAndSanitizePositionSpec({
    mySide: 0,
    turn: 0,
    ownSets,
    previewSides: tracePreviewSides,
    pickedRosterSlots: [0, 1, 2],
    battleTurn: null,
    pdat: null,
    llog: null,
    seenFoeMons: turn2SeenFoeBefore,
    teamPreview: true,
    forceSwitch: false,
  });

  const cleanTrace = [
    {
      type: 'match_start',
      matchId: 'selftest-clean',
      botName: 'puniu3pbs',
      team: { id: 'test-team', sets: ownSets },
    },
    {
      type: 'preview',
      matchId: 'selftest-clean',
      decision: 1,
      mySide: 0,
      foeName: 'テスト相手',
      rawCheckHtml,
      pickedRosterSlots: [0, 1, 2],
      spec: previewSpec,
      sanitizeDiffs: previewSpec._sanitizeDiffs || [],
      errors: [],
    },
    {
      type: 'decision',
      matchId: 'selftest-clean',
      decision: 2,
      turn: 2,
      phase: 'INPUT',
      forceSwitch: false,
      mySide: 0,
      foeName: 'テスト相手',
      pickedRosterSlots: [0, 1, 2],
      rawTurnHtml: rawTurn2Html,
      rawPdatHtml: rawPdatTurn2Html,
      rawLlogHtml: rawLlogTurn2Html,
      battleTurn: parsedTurn2Bt,
      pdat: parsedTurn2Pdat,
      llogSummary: serializeLlogTracker(parsedTurn2Llog),
      seenFoeMonsBefore: initSeenFoe(),
      seenFoeMonsAfter: turn2SeenFoe,
      lastFoeActiveSlotBefore: 0,
      spec: cleanSpecTurn2,
      sanitizeDiffs: cleanSpecTurn2._sanitizeDiffs || [],
      bestChoice: 'move bodyslam',
      matchedAction: 'move bodyslam',
      mappingPath: 'direct:move-slot',
      chosenMoveValue: 'W1',
      postFields: { ACTION: 'INPUT', NOW: '1', MOVE: 'W1' },
      errors: [],
    },
    {
      type: 'finish',
      matchId: 'selftest-clean',
      outcome: 'WIN',
      finishTrigger: 'FINISHED',
      decisions: 2,
      mySide: 0,
      foeName: 'テスト相手',
      rawFinishHtml: `<p>puniu3pbs ${fwWin}</p>`,
      rawFinalLlogHtml: rawLlogTurn2Html,
      errors: [],
    },
  ];

  const cleanReport = auditMatchTrace(cleanTrace, { reparse: true, validateWasm: true });
  assert.strictEqual(cleanReport.ok, true, `Expected clean trace to pass parity audit, got: ${JSON.stringify(cleanReport.anomalies)}`);
  assert.strictEqual(cleanReport.errorCount, 0);
  assert.strictEqual(cleanReport.warnCount, 0);

  // Verify anomaly detection on corrupted trace (species mismatch, status mismatch, foeRemaining mismatch, PP mismatch, unrecognized LLOG line, fallback action)
  const buggyTurnHtml = rawTurn2Html
    .replace('<TD>ライコウ L50</TD>', '<TD>ナッシー♂ L50</TD>')
    .replace('<TD>通常</TD><TD>通常</TD>', '<TD>まひ</TD><TD>通常</TD>')
    .replace('のしかかり [23]', 'のしかかり [20]');
  const buggyPdatHtml = rawPdatTurn2Html.replace('相手の残り：3', '相手の残り：1');
  const buggyLlogHtml = rawLlogTurn2Html + '<P ALIGN="center">謎の未知のメッセージが発生した！<BR></P>';

  const buggyTrace = JSON.parse(JSON.stringify(cleanTrace));
  buggyTrace[2].rawTurnHtml = buggyTurnHtml;
  buggyTrace[2].rawPdatHtml = buggyPdatHtml;
  buggyTrace[2].rawLlogHtml = buggyLlogHtml;
  buggyTrace[2].mappingPath = 'emergency-fallback:first-legal-option';
  buggyTrace[2].sanitizeDiffs = [{ path: 'sides[0].active', from: null, to: 0, reason: 'no_active_mon' }];

  const buggyReport = auditMatchTrace(buggyTrace, { reparse: true, validateWasm: true });
  assert.strictEqual(buggyReport.ok, false);
  const detectedCodes = new Set(buggyReport.anomalies.map(a => a.code));
  assert.ok(detectedCodes.has('ACTIVE_SPECIES_MISMATCH'));
  assert.ok(detectedCodes.has('ACTIVE_STATUS_MISMATCH'));
  assert.ok(detectedCodes.has('PDAT_FOE_REMAINING_MISMATCH'));
  assert.ok(detectedCodes.has('OWN_PP_TRACKING_MISMATCH'));
  assert.ok(detectedCodes.has('LLOG_UNRECOGNIZED_LINE'));
  assert.ok(detectedCodes.has('ACTION_MAPPING_FALLBACK'));
  assert.ok(detectedCodes.has('SANITIZER_FORCED_CLAMP'));

  console.log('[discord-bot] Self-test passed (K-room routing + entry IPC + PBS state machine + Parity Audit)!');
}

async function main() {
  const cliArgs = parseArgs(process.argv);
  if (cliArgs.selftest) {
    await runSelfTest();
    return;
  }

  const config = loadConfig(cliArgs);

  if (!config.discordToken) {
    console.error(
      'Usage: set DISCORD_TOKEN or create pbs-bot.config.json (see pbs-bot.config.example.json),\n' +
      'then run:\n' +
      '  node tools/pbs-discord-bot.js              # Normal live operation\n' +
      '  node tools/pbs-discord-bot.js --observe    # Passive sniffer (logs room-bot messages without replying)\n' +
      '  node tools/pbs-discord-bot.js --dry-run-pbs # Replies "応戦します" and detects room without entering PS.cgi\n' +
      '  node tools/pbs-discord-bot.js --selftest   # Run built-in state-machine self-test'
    );
    process.exit(1);
  }

  const bot = new CastleDiscordBot(config);
  bot.startGateway();
}

if (require.main === module) {
  main().catch(err => {
    console.error('[discord-bot] Fatal error:', err);
    process.exit(1);
  });
}

module.exports = {
  CastleDiscordBot,
  CASTLE_DISCORD_ROOMS,
  loadConfig,
  parseRoomFromText,
  detectRequestedTier,
  parseChallengeMention,
  extractFullMessageText,
  probeCastleDiscordRooms,
  runSelfTest,
};
