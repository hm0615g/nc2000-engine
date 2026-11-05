#!/usr/bin/env node
/**
 * Resident Discord Bot for Gym Leader's Castle (`ジムリーダーの城`, `psense.lib.net/PBS/_/C..O/PS.cgi`).
 *
 * Operational flow:
 * 1. Connects to Discord Gateway v10 via WebSocket (`ws`) + REST v10 (`fetch`).
 * 2. Idles with zero traffic to `psense.lib.net` until `@mentioned` in Discord.
 * 3. Determines strength/party tier from the `@mention` text:
 *    - Default (`tier = "default"`): `81,000` iterations (`c = 0.4`) + `pool:nash` (APEX4 equilibrium mixture).
 *    - Mercy / Handicap (`tier = "mercy"`, triggered by `手加減`, `別パ`, `サブ`, `Tier A`, etc.):
 *      `81,000` iterations (`c = 0.4`) + `pool:tier-a` (8 iconic human-origin Tier A teams).
 * 4. Immediately replies `"応戦します"` to trigger the Castle Discord server's resident room-allocation bot.
 *    - If the `@mention` already specifies a room (`C`–`O` or `PS.cgi` URL), launches `pbs-client.js` immediately.
 *    - Otherwise, watches subsequent `MESSAGE_CREATE`, `MESSAGE_UPDATE`, and `THREAD_CREATE` events
 *      in the channel (for up to `roomWaitTimeoutSec`, default 90s) for the room assignment from the
 *      Castle room-allocation bot (or challenger) and immediately launches `pbs-client.js`.
 *
 * Testing & diagnostic modes for inspecting the Castle room-allocation bot:
 * - `--observe` (`observeOnly: true`): Passive sniffer mode. Logs all channel messages/embeds/updates
 *   to console and `data/discord-events.jsonl` and shows what room/tier parser extracts, without
 *   sending any Discord replies or launching `pbs-client.js`.
 * - `--dry-run-pbs` (`dryRunPbs: true`): Replies `"応戦します"` on `@mention` and detects the assigned
 *   room from the room bot, but stops before entering `PS.cgi`.
 * - `--selftest`: Runs built-in parser & state-machine simulation tests and exits.
 */
'use strict';

const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');
const WebSocket = require('ws');

const REPO = path.resolve(__dirname, '..');

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
    // replyMode: "reply" (Discord message_reference reply), "mention" ("<@id> 応戦します"), or "plain" ("応戦します")
    replyMode:
      cliArgs['reply-mode'] ||
      process.env.REPLY_MODE ||
      fileCfg.replyMode ||
      'reply',
    postMatchThanksText:
      cliArgs['thanks-text'] ??
      process.env.POST_MATCH_THANKS_TEXT ??
      fileCfg.postMatchThanksText ??
      '対戦ありがとうございました！',
    iters: parseInt(cliArgs.iters || process.env.PBS_ITERS || fileCfg.iters || '81000', 10),
    hostTimeoutSec: parseInt(
      cliArgs['host-timeout-sec'] || process.env.HOST_TIMEOUT_SEC || fileCfg.hostTimeoutSec || '240',
      10
    ),
    roomWaitTimeoutSec: parseInt(
      cliArgs['room-wait-sec'] || process.env.ROOM_WAIT_TIMEOUT_SEC || fileCfg.roomWaitTimeoutSec || '90',
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
      cliArgs['event-log'] || fileCfg.eventLogPath || 'data/discord-events.jsonl'
    ),
  };
}

/**
 * Normalize fullwidth ASCII letters `Ｃ`–`Ｏ` to halfwidth `C`–`O`.
 */
function normalizeFullwidthAscii(str) {
  return String(str || '').replace(/[Ａ-Ｚａ-ｚ０-９]/g, ch =>
    String.fromCharCode(ch.charCodeAt(0) - 0xfee0)
  );
}

/**
 * Extract a Castle room letter (`C`..`O`) or direct `PS.cgi` URL from a Discord message.
 * Handles:
 * - Direct URLs: `http://psense.lib.net/PBS/_/K/PS.cgi` or `.../PBS/_/K/`
 * - Room labels: `K部屋`, `Ｋ部屋`, `部屋K`, `ルーム: K`, `[K]`, `【K】`, `「K」`, `Room K`, `城K`, `割当: K`, `会場: K`
 */
function parseRoomFromText(rawText, pbsBaseUrl = 'http://psense.lib.net/PBS/_/{ROOM}/PS.cgi') {
  if (!rawText) return null;
  const text = normalizeFullwidthAscii(String(rawText));

  // 1. Direct psense.lib.net URL (with or without PS.cgi suffix)
  const urlMatch = text.match(/https?:\/\/psense\.lib\.net\/PBS\/_\/([C-O])(?:\/PS\.cgi|\/|\b)/i);
  if (urlMatch) {
    const letter = urlMatch[1].toUpperCase();
    return {
      roomLetter: letter,
      roomUrl: pbsBaseUrl.replace('{ROOM}', letter),
    };
  }

  // 2. Strip `<@...>` and `<#...>` Discord tags and custom emoji `<:name:id>` before matching letters
  const stripped = text
    .replace(/<a?:[A-Za-z0-9_]+:\d+>/g, ' ')
    .replace(/<[@#!&][^>]+>/g, ' ');

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
 * Extract all human-readable text from a Discord message or thread payload, including embeds
 * and interactive component labels (since room-allocation bots often put the assigned room
 * or `PS.cgi` URL in an embed description, field, or link button).
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
    this.activeMatch = null;      // { roomLetter, roomUrl, challengerName, tier, child }
    this.pendingChallenge = null; // { channelId, messageId, challengerId, challengerName, tier, timer }
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
    if (this.pendingChallenge && this.pendingChallenge.timer) {
      clearTimeout(this.pendingChallenge.timer);
    }
    this.pendingChallenge = null;
  }

  async handleDiscordMessageEvent(eventType, msg) {
    if (!msg) return;
    const channelId = msg.channel_id || msg.parent_id || msg.id;
    if (!channelId) return;
    if (msg.author?.id && msg.author.id === this.botUserId) return;

    // Filter by allowed channelIds if configured
    if (
      this.config.channelIds.length > 0 &&
      !this.config.channelIds.includes(String(channelId)) &&
      !this.config.channelIds.includes(String(msg.parent_id || ''))
    ) {
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
        `-> parsedRoom=${parsedRoom ? `${parsedRoom.roomLetter} (${parsedRoom.roomUrl})` : 'none'}, ` +
        `tier=${detectRequestedTier(fullText)}`
      );
      return;
    }

    // 1. If we are waiting for a room assignment after replying "応戦します",
    //    check if this event (MESSAGE_CREATE, MESSAGE_UPDATE, or THREAD_CREATE) assigns a room!
    if (
      this.pendingChallenge &&
      (this.pendingChallenge.channelId === channelId || this.pendingChallenge.channelId === msg.parent_id) &&
      !this.activeMatch
    ) {
      const isRoomBot =
        this.config.roomBotIds.length > 0
          ? this.config.roomBotIds.includes(String(msg.author?.id))
          : Boolean(msg.author?.bot);
      const isSameChallenger = msg.author?.id && msg.author.id === this.pendingChallenge.challengerId;

      this.log(
        `[PENDING_ROOM ${eventType}] author=${msg.author?.username || '?'}(id=${msg.author?.id || '?'}, bot=${Boolean(msg.author?.bot)}) ` +
        `text=${JSON.stringify(fullText.replace(/\s+/g, ' ').trim())} -> parsedRoom=${parsedRoom ? parsedRoom.roomLetter : 'none'}`
      );

      if (parsedRoom && (isRoomBot || isSameChallenger || this.config.roomBotIds.length === 0)) {
        const pending = this.pendingChallenge;
        this.clearPendingChallenge();
        this.log(
          `Detected room assignment ${parsedRoom.roomLetter} (${parsedRoom.roomUrl}) for challenger "${pending.challengerName}" (tier=${pending.tier}).`
        );
        this.launchPbsMatch({
          channelId: pending.channelId,
          challengerName: pending.challengerName,
          tier: pending.tier,
          roomLetter: parsedRoom.roomLetter,
          roomUrl: parsedRoom.roomUrl,
        });
        return;
      }
    }

    // Only initiate a new challenge on MESSAGE_CREATE
    if (eventType !== 'MESSAGE_CREATE') return;

    // 2. Check if this message `@mentions` our bot to request a match
    const challenge = parseChallengeMention(msg, this.botUserId, this.config.pbsBaseUrl);
    if (!challenge) return;

    if (this.activeMatch) {
      this.log(
        `Ignored challenge from "${challenge.challengerName}" because a match is already active in room ${this.activeMatch.roomLetter}.`
      );
      await this.sendChannelMessage(
        channelId,
        `現在 ${this.activeMatch.roomLetter}部屋で対戦中です。終了後にまたお声がけください！`,
        msg.id
      );
      return;
    }

    this.log(
      `Received @mention challenge from "${challenge.challengerName}" in channel ${channelId} ` +
      `(tier=${challenge.tier}${challenge.room ? `, room=${challenge.room.roomLetter}` : ', waiting for room allocation'})`
    );

    // Send immediate response ("応戦します") using configured replyMode
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

    // If the `@mention` already included a room letter/URL, launch immediately!
    if (challenge.room) {
      this.clearPendingChallenge();
      this.launchPbsMatch({
        channelId,
        challengerName: challenge.challengerName,
        tier: challenge.tier,
        roomLetter: challenge.room.roomLetter,
        roomUrl: challenge.room.roomUrl,
      });
      return;
    }

    // Otherwise, wait for the Castle room-allocation bot (or challenger) to post the assigned room
    this.clearPendingChallenge();
    const timeoutMs = Math.max(15, this.config.roomWaitTimeoutSec) * 1000;
    const timer = setTimeout(() => {
      if (this.pendingChallenge && this.pendingChallenge.messageId === msg.id) {
        this.log(
          `Room allocation wait timed out (${this.config.roomWaitTimeoutSec}s) for challenger "${challenge.challengerName}".`
        );
        this.pendingChallenge = null;
      }
    }, timeoutMs);

    this.pendingChallenge = {
      channelId,
      messageId: msg.id,
      challengerId: challenge.challengerId,
      challengerName: challenge.challengerName,
      tier: challenge.tier,
      timer,
    };
  }

  launchPbsMatch({ channelId, challengerName, tier, roomLetter, roomUrl }) {
    if (this.activeMatch) return;

    if (this.config.dryRunPbs) {
      this.log(
        `[DRY-RUN-PBS] Would launch pbs-client.js in room ${roomLetter} (${roomUrl}) ` +
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
      '--mode', 'auto',
      '--host-timeout-sec', String(this.config.hostTimeoutSec),
      '--seed', String(Date.now() % 1000000),
    ];

    this.log(
      `Spawning pbs-client.js in room ${roomLetter} (${roomUrl}) [tier=${tier}, iters=${this.config.iters}]...`
    );

    const child = spawn(process.execPath, cliArgs, {
      cwd: REPO,
      stdio: ['ignore', 'pipe', 'pipe'],
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
      roomUrl,
      child,
    };

    child.on('close', async code => {
      this.activeMatch = null;
      const timedOut = lastOutput.includes('wait timeout') || lastOutput.includes('(TIMEOUT)');
      const playedTurns = /after\s+([1-9]\d*)\s+decisions/.test(lastOutput);
      this.log(
        `pbs-client.js for room ${roomLetter} exited with code ${code} (playedTurns=${playedTurns}, timedOut=${timedOut}).`
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
          // HELLO: start heartbeating and send IDENTIFY
          const intervalMs = pkt.d?.heartbeat_interval || 41250;
          if (this.heartbeatTimer) clearInterval(this.heartbeatTimer);
          this.heartbeatTimer = setInterval(() => {
            if (this.ws && this.ws.readyState === WebSocket.OPEN) {
              this.ws.send(JSON.stringify({ op: 1, d: this.lastSeq }));
            }
          }, intervalMs);

          // Intents: GUILDS (1 << 0) | GUILD_MESSAGES (1 << 9) | DIRECT_MESSAGES (1 << 12) | MESSAGE_CONTENT (1 << 15) = 37377
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
            this.log(`READY as ${this.botUsername} (id=${this.botUserId}). Watching for events...`);
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
  const cfg = loadConfig({ 'dry-run-pbs': true, 'event-log': '' });
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

  // Scenario 1: Challenger @mentions bot -> Bot immediately replies "応戦します" -> Room bot posts embed with room K
  await bot.handleDiscordMessageEvent('MESSAGE_CREATE', {
    id: 'msg-101',
    channel_id: 'ch-1',
    author: { id: 'user-1', username: 'ChallengerA', bot: false },
    content: '<@999999> 対戦お願いします！',
    mentions: [{ id: '999999' }],
  });
  assert.strictEqual(sentMessages.length, 1);
  assert.strictEqual(sentMessages[0].content, '応戦します');
  assert.strictEqual(sentMessages[0].replyToMessageId, 'msg-101');
  assert.strictEqual(launchedMatches.length, 0);
  assert.ok(bot.pendingChallenge);

  // Room bot assigns room K via an embed update (`MESSAGE_UPDATE`)
  await bot.handleDiscordMessageEvent('MESSAGE_UPDATE', {
    id: 'msg-102',
    channel_id: 'ch-1',
    author: { id: 'roombot-1', username: 'CastleRoomBot', bot: true },
    content: '',
    embeds: [
      {
        title: '部屋割り当て完了',
        description: 'ChallengerA vs Bot\n会場: Ｋ部屋\nhttp://psense.lib.net/PBS/_/K/PS.cgi',
      },
    ],
  });
  assert.strictEqual(launchedMatches.length, 1);
  assert.strictEqual(launchedMatches[0].roomLetter, 'K');
  assert.strictEqual(launchedMatches[0].tier, 'default');
  assert.strictEqual(bot.pendingChallenge, null);

  // Scenario 2: Challenger requests mercy ("手加減") -> Room bot assigns room D in plain text
  await bot.handleDiscordMessageEvent('MESSAGE_CREATE', {
    id: 'msg-201',
    channel_id: 'ch-1',
    author: { id: 'user-2', username: 'ChallengerB', bot: false },
    content: '<@999999> 手加減でお願いします',
    mentions: [{ id: '999999' }],
  });
  await bot.handleDiscordMessageEvent('MESSAGE_CREATE', {
    id: 'msg-202',
    channel_id: 'ch-1',
    author: { id: 'roombot-1', username: 'CastleRoomBot', bot: true },
    content: '【D】部屋を使ってください',
  });
  assert.strictEqual(launchedMatches.length, 2);
  assert.strictEqual(launchedMatches[1].roomLetter, 'D');
  assert.strictEqual(launchedMatches[1].tier, 'mercy');

  console.log('[discord-bot] Self-test passed (immediate reply + MESSAGE_CREATE/MESSAGE_UPDATE/embed room allocation + tier switching)!');
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
  loadConfig,
  parseRoomFromText,
  detectRequestedTier,
  parseChallengeMention,
  extractFullMessageText,
  runSelfTest,
};
