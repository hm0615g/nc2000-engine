#!/usr/bin/env node
/**
 * Resident Discord Bot for Gym Leader's Castle (`ジムリーダーの城`, `psense.lib.net/PBS/_/C..O/PS.cgi`).
 *
 * Operational flow:
 * 1. Connects to Discord Gateway v10 via WebSocket (`ws`) + REST v10 (`fetch`) with zero heavy dependencies.
 * 2. Idles with zero traffic to `psense.lib.net` until a user `@mentions` the bot in Discord.
 * 3. Determines strength/party tier from the `@mention` text:
 *    - Default (`tier = "default"`): `81,000` iterations (`c = 0.4`) + `pool:nash` (APEX4 equilibrium mixture).
 *    - Mercy / Handicap (`tier = "mercy"`, triggered by `手加減`, `別パ`, `サブ`, `Tier A`, etc.):
 *      `81,000` iterations (`c = 0.4`) + `pool:tier-a` (8 iconic human-origin Tier A teams).
 * 4. Replies `"応戦します"` to trigger the Castle Discord server's resident room-allocation bot.
 *    - If the `@mention` already specifies a room (`C`–`O` or `PS.cgi` URL), launches `pbs-client.js` immediately.
 *    - Otherwise, watches subsequent messages in the same channel (for up to `roomWaitTimeoutSec`, default 90s)
 *      for the room assignment from the Castle room-allocation bot (or challenger) and launches `pbs-client.js`.
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

function loadConfig( cliArgs = {} ) {
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
 * - Direct URLs: `http://psense.lib.net/PBS/_/K/PS.cgi`
 * - Room labels: `K部屋`, `Ｋ部屋`, `部屋K`, `ルーム: K`, `[K]`, `【K】`, `Room K`
 */
function parseRoomFromText(rawText, pbsBaseUrl = 'http://psense.lib.net/PBS/_/{ROOM}/PS.cgi') {
  if (!rawText) return null;
  const text = normalizeFullwidthAscii(String(rawText));

  // 1. Direct PS.cgi URL
  const urlMatch = text.match(/https?:\/\/psense\.lib\.net\/PBS\/_\/([C-O])\/PS\.cgi/i);
  if (urlMatch) {
    const letter = urlMatch[1].toUpperCase();
    return {
      roomLetter: letter,
      roomUrl: pbsBaseUrl.replace('{ROOM}', letter),
    };
  }

  // 2. Strip `<@...>` and `<#...>` Discord tags before looking for room letters
  const stripped = text.replace(/<[@#!&][^>]+>/g, ' ');

  const patterns = [
    /(?:^|[^A-Za-z0-9])([C-O])\s*(?:部屋|ルーム|room)(?![A-Za-z])/i,
    /(?:部屋|ルーム|room)\s*[：:はが]?\s*([C-O])(?![A-Za-z0-9])/i,
    /[【\[「『]\s*([C-O])\s*(?:部屋|ルーム)?\s*[】\]」』]/i,
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
    challengerName: msg.member?.nick || msg.author?.global_name || msg.author?.username || 'Challenger',
  };
}

/**
 * Extract text from a Discord message including any embeds (since room-allocation bots
 * often post the assigned room or `PS.cgi` link inside an embed description/field/url).
 */
function extractFullMessageText(msg) {
  if (!msg) return '';
  const parts = [msg.content || ''];
  if (Array.isArray(msg.embeds)) {
    for (const emb of msg.embeds) {
      if (emb.title) parts.push(emb.title);
      if (emb.description) parts.push(emb.description);
      if (emb.url) parts.push(emb.url);
      if (Array.isArray(emb.fields)) {
        for (const f of emb.fields) {
          if (f.name) parts.push(f.name);
          if (f.value) parts.push(f.value);
        }
      }
    }
  }
  return parts.join('\n');
}

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
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
    this.activeMatch = null;     // { roomLetter, roomUrl, challengerName, tier, child }
    this.pendingChallenge = null; // { channelId, messageId, challengerId, challengerName, tier, timer }
  }

  log(msg) {
    console.log(`[discord-bot ${new Date().toISOString()}] ${msg}`);
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

  async handleMessageCreate(msg) {
    if (!msg || !msg.channel_id) return;
    if (msg.author?.id === this.botUserId) return;

    // Filter by allowed channelIds if configured
    if (
      this.config.channelIds.length > 0 &&
      !this.config.channelIds.includes(String(msg.channel_id))
    ) {
      return;
    }

    // 1. If we are waiting for a room assignment in this channel after replying "応戦します",
    //    check if this message (from the Castle room-allocation bot or challenger) assigns a room!
    if (this.pendingChallenge && this.pendingChallenge.channelId === msg.channel_id && !this.activeMatch) {
      const isRoomBot =
        this.config.roomBotIds.length > 0
          ? this.config.roomBotIds.includes(String(msg.author?.id))
          : Boolean(msg.author?.bot);
      const isSameChallenger = msg.author?.id === this.pendingChallenge.challengerId;

      if (isRoomBot || isSameChallenger || this.config.roomBotIds.length === 0) {
        const fullText = extractFullMessageText(msg);
        const room = parseRoomFromText(fullText, this.config.pbsBaseUrl);
        if (room) {
          const pending = this.pendingChallenge;
          this.clearPendingChallenge();
          this.log(
            `Detected room assignment ${room.roomLetter} (${room.roomUrl}) for challenger "${pending.challengerName}" (tier=${pending.tier}).`
          );
          this.launchPbsMatch({
            channelId: pending.channelId,
            challengerName: pending.challengerName,
            tier: pending.tier,
            roomLetter: room.roomLetter,
            roomUrl: room.roomUrl,
          });
          return;
        }
      }
    }

    // 2. Check if this message `@mentions` our bot to request a match
    const challenge = parseChallengeMention(msg, this.botUserId, this.config.pbsBaseUrl);
    if (!challenge) return;

    if (this.activeMatch) {
      this.log(
        `Ignored challenge from "${challenge.challengerName}" because a match is already active in room ${this.activeMatch.roomLetter}.`
      );
      await this.sendChannelMessage(
        msg.channel_id,
        `現在 ${this.activeMatch.roomLetter}部屋で対戦中です。終了後にまたお声がけください！`,
        msg.id
      );
      return;
    }

    this.log(
      `Received @mention challenge from "${challenge.challengerName}" in channel ${msg.channel_id} ` +
      `(tier=${challenge.tier}${challenge.room ? `, room=${challenge.room.roomLetter}` : ', waiting for room allocation'})`
    );

    // Human-like 1.5s - 2.8s delay before replying "応戦します"
    await sleep(1500 + Math.floor(Math.random() * 1300));

    if (this.config.replyTriggerText) {
      await this.sendChannelMessage(msg.channel_id, this.config.replyTriggerText, msg.id);
    }

    // If the `@mention` already included a room letter/URL, launch immediately!
    if (challenge.room) {
      this.clearPendingChallenge();
      this.launchPbsMatch({
        channelId: msg.channel_id,
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
      channelId: msg.channel_id,
      messageId: msg.id,
      challengerId: challenge.challengerId,
      challengerName: challenge.challengerName,
      tier: challenge.tier,
      timer,
    };
  }

  launchPbsMatch({ channelId, challengerName, tier, roomLetter, roomUrl }) {
    if (this.activeMatch) return;

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
      this.log(`pbs-client.js for room ${roomLetter} exited with code ${code} (playedTurns=${playedTurns}, timedOut=${timedOut}).`);

      if (playedTurns && !timedOut && this.config.postMatchThanksText) {
        await sleep(1500 + Math.floor(Math.random() * 1500));
        await this.sendChannelMessage(channelId, this.config.postMatchThanksText);
      }
    });
  }

  startGateway() {
    const gatewayUrl = 'wss://gateway.discord.gg/?v=10&encoding=json';
    this.log(`Connecting to Discord Gateway (${gatewayUrl})...`);
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
          // Immediate heartbeat request
          ws.send(JSON.stringify({ op: 1, d: this.lastSeq }));
          break;
        }
        case 7: {
          // Reconnect requested
          this.log('Discord Gateway requested reconnect (op 7).');
          ws.close();
          break;
        }
        case 9: {
          // Invalid session
          this.log('Discord Gateway reported invalid session (op 9), reconnecting in 5s...');
          this.sessionId = null;
          ws.close();
          break;
        }
        case 0: {
          // DISPATCH
          if (pkt.t === 'READY') {
            this.botUserId = pkt.d?.user?.id || null;
            this.botUsername = pkt.d?.user?.username || 'Bot';
            this.sessionId = pkt.d?.session_id || null;
            this.resumeGatewayUrl = pkt.d?.resume_gateway_url || null;
            this.log(`READY as ${this.botUsername} (id=${this.botUserId}). Watching for @mentions...`);
          } else if (pkt.t === 'MESSAGE_CREATE') {
            this.handleMessageCreate(pkt.d).catch(err => {
              this.log(`Error handling MESSAGE_CREATE: ${err.message || err}`);
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
        console.error('[discord-bot] FATAL: Discord rejected token (4004 Authentication Failed). Check DISCORD_TOKEN / pbs-bot.config.json.');
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

async function main() {
  const cliArgs = parseArgs(process.argv);
  const config = loadConfig(cliArgs);

  if (!config.discordToken) {
    console.error(
      'Usage: set DISCORD_TOKEN or create pbs-bot.config.json (see pbs-bot.config.example.json),\n' +
      'then run: node tools/pbs-discord-bot.js'
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
};
