#!/usr/bin/env node
/**
 * calibrate-pbs-from-logs.js
 *
 * Ingests Gym Leader's Castle (`ジムリーダーの城`) battle logs (`data/pbs-castle-logs/logs/*.html`)
 * into `data/pbs-opponent-db.json` for offline calibration and opponent habit memory.
 *
 * SOTA Architect Invariants:
 * 1. Format Filter: Only ingests NC2000 matches (all 12 roster mons Lv 50..55, exactly 6v6).
 *    Excludes Little Cup (Lv 5) and invalid roster matches.
 * 2. Symmetric Ingestion: Records P1 and P2 observations independently under their respective names.
 * 3. Guest & Bot Exclusion: Skips ephemeral names (名無し, guest, player, テスト) and bot self-records
 *    (puniu3pbs, NC2000) to prevent circular self-conditioning bias.
 * 4. Move Bound & Accuracy: Reconstructed from `parseLlogHtml` events and revealed sets.
 * 5. M2M Output Principle: Completely silent during processing; outputs a single structured JSON
 *    telemetry line on stdout upon completion.
 *
 * Usage:
 *   node tools/calibrate-pbs-from-logs.js [--logs-dir DIR] [--out FILE] [--merge]
 */
'use strict';

const fs = require('fs');
const path = require('path');
const {
  parseTeamPreviewHtml,
  parseLlogHtml,
  loadOpponentDb,
  saveOpponentDb,
  recordOpponentObservation,
} = require('./pbs-client.js');

const REPO = path.resolve(__dirname, '..');

function parseArgs() {
  const args = process.argv.slice(2);
  const opts = {
    logsDir: path.join(REPO, 'data/pbs-castle-logs/logs'),
    outPath: path.join(REPO, 'data/pbs-opponent-db.json'),
    merge: false,
    minLevel: 50,
    maxLevel: 55,
    skipNames: new Set(['名無し', 'guest', 'player', 'テスト', 'puniu3pbs', 'nc2000']),
  };

  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === '--logs-dir' && args[i + 1]) {
      opts.logsDir = path.resolve(REPO, args[++i]);
    } else if (a === '--out' && args[i + 1]) {
      opts.outPath = path.resolve(REPO, args[++i]);
    } else if (a === '--merge') {
      opts.merge = true;
    } else if (a === '--min-level' && args[i + 1]) {
      opts.minLevel = parseInt(args[++i], 10);
    } else if (a === '--max-level' && args[i + 1]) {
      opts.maxLevel = parseInt(args[++i], 10);
    }
  }
  return opts;
}

function runCalibration() {
  const opts = parseArgs();
  if (!fs.existsSync(opts.logsDir)) {
    console.error(JSON.stringify({ status: 'error', message: `Logs directory not found: ${opts.logsDir}` }));
    process.exit(1);
  }

  const logFiles = fs.readdirSync(opts.logsDir).filter(f => f.endsWith('.html')).sort();
  const db = opts.merge
    ? loadOpponentDb(opts.outPath)
    : { schema: 'pbs-opponent-db-v1', updatedAt: null, players: {}, shells: {} };

  let eligibleGames = 0;
  let skippedNonNc2000 = 0;
  let skippedRosterMismatch = 0;
  let parseErrors = 0;

  for (const f of logFiles) {
    const filePath = path.join(opts.logsDir, f);
    let html = '';
    try {
      const raw = fs.readFileSync(filePath);
      html = new TextDecoder('euc-jp').decode(raw);
    } catch {
      try {
        html = fs.readFileSync(filePath, 'utf8');
      } catch {
        parseErrors++;
        continue;
      }
    }

    let preview;
    try {
      preview = parseTeamPreviewHtml(html);
    } catch {
      parseErrors++;
      continue;
    }

    if (!Array.isArray(preview.p1Mons) || preview.p1Mons.length !== 6 ||
        !Array.isArray(preview.p2Mons) || preview.p2Mons.length !== 6) {
      skippedRosterMismatch++;
      continue;
    }

    const allLevels = [...preview.p1Mons, ...preview.p2Mons].map(m => m.level || 50);
    if (allLevels.some(lv => lv < opts.minLevel || lv > opts.maxLevel)) {
      skippedNonNc2000++;
      continue;
    }

    let llog;
    try {
      llog = parseLlogHtml(html, [preview.p1Mons, preview.p2Mons]);
    } catch {
      parseErrors++;
      continue;
    }

    eligibleGames++;

    // Determine winner if rendered in HTML body
    let winner = null;
    const winMatch = html.match(/Color=["\']?GoldenRod["\']?>\s*(.*?)(?:&nbsp;|\s)*ＷＩＮ！/i);
    if (winMatch) {
      winner = winMatch[1].replace(/<[^>]*>/g, '').trim();
    }

    const sides = [
      { name: preview.p1Name, mons: preview.p1Mons, llogMons: llog.sides[0].mons },
      { name: preview.p2Name, mons: preview.p2Mons, llogMons: llog.sides[1].mons },
    ];

    for (let s = 0; s < 2; s++) {
      const pName = sides[s].name ? sides[s].name.trim() : '';
      if (!pName) continue;
      const lowerName = pName.toLowerCase();
      if (opts.skipNames.has(pName) || opts.skipNames.has(lowerName)) {
        continue;
      }

      const outcome = winner ? (winner.includes(pName) ? 'WIN' : 'LOSE') : 'DRAW';

      recordOpponentObservation({
        opponentDb: db,
        foeName: pName,
        foePreviewMons: sides[s].mons,
        seenFoeMons: sides[s].llogMons,
        llogFoeMons: sides[s].llogMons,
        outcome,
      });
    }
  }

  saveOpponentDb(db, opts.outPath);

  const totalPlayers = Object.keys(db.players || {}).length;
  const totalShells = Object.keys(db.shells || {}).length;

  // Single-Line Structured Telemetry (M2M Output Principle)
  const telemetry = {
    status: 'ok',
    total_logs_examined: logFiles.length,
    eligible_nc2000_games: eligibleGames,
    skipped_non_nc2000: skippedNonNc2000,
    skipped_roster_mismatch: skippedRosterMismatch,
    parse_errors: parseErrors,
    calibrated_players: totalPlayers,
    calibrated_shells: totalShells,
    output_path: path.relative(REPO, opts.outPath),
  };

  console.log(JSON.stringify(telemetry));
}

if (require.main === module) {
  runCalibration();
}

module.exports = { runCalibration };
