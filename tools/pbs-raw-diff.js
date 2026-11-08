#!/usr/bin/env node
/**
 * Differential parity auditor between Showdown raw.log (from convert-pbs-html-dir-to-raw.js)
 * and tools/pbs-client.js (parseLlogHtml).
 *
 * M2M Output Principle:
 *   - Zero stdout spam / banner output.
 *   - Emits exactly 1 JSON line on stdout summarizing parity metrics.
 *   - Offloads detailed diff traces to /tmp/pbs-raw-diff.log if any.
 */
'use strict';

const fs = require('fs');
const path = require('path');
const { parseTeamPreviewHtml, parseLlogHtml } = require('./pbs-client.js');

function runRawDiff(options = {}) {
  const rawDir = options.rawDir || '/tmp/pbs-test-out/raw';
  const htmlDir = options.htmlDir || path.resolve(__dirname, '../data/pbs-castle-logs/logs');
  const traceFile = options.traceFile || '/tmp/pbs-raw-diff.log';

  if (!fs.existsSync(rawDir) || !fs.existsSync(htmlDir)) {
    const res = { error: 'Required directories not found', rawDir, htmlDir };
    console.log(JSON.stringify(res));
    return res;
  }

  const rawFiles = fs.readdirSync(rawDir).filter(f => f.endsWith('.raw.log'));
  const mismatches = {
    faint: [],
    status: [],
    weather: [],
    unrecognized: [],
  };

  let auditedBattles = 0;

  for (const rf of rawFiles) {
    const base = rf.replace('battle-gen2nintendocup2000noohkostadium2strict-pbs-', '').replace('.raw.log', '');
    const htmlFile = base.replace(/-/g, '_') + '.html';
    const htmlPath = path.join(htmlDir, htmlFile);
    if (!fs.existsSync(htmlPath)) continue;

    const rawContent = fs.readFileSync(path.join(rawDir, rf), 'utf8');
    const htmlBytes = fs.readFileSync(htmlPath);
    const html = new TextDecoder('euc-jp').decode(htmlBytes);

    let preview;
    try {
      preview = parseTeamPreviewHtml(html);
    } catch {
      continue;
    }

    const llog = parseLlogHtml(html, [preview.p1Mons, preview.p2Mons]);
    auditedBattles++;

    if (llog.unrecognizedLines && llog.unrecognizedLines.length > 0) {
      mismatches.unrecognized.push({ file: htmlFile, lines: llog.unrecognizedLines });
    }

    const rawLines = rawContent.split('\n');
    const faintedFromRaw = { p1: new Set(), p2: new Set() };
    const statusFromRaw = { p1: new Map(), p2: new Map() };
    let lastRawWeather = null;

    for (const line of rawLines) {
      if (line.startsWith('|faint|')) {
        const parts = line.split('|');
        const side = parts[2].slice(0, 2);
        const sp = parts[2].split(': ')[1];
        faintedFromRaw[side].add(sp);
      } else if (line.startsWith('|-status|')) {
        const parts = line.split('|');
        const side = parts[2].slice(0, 2);
        const sp = parts[2].split(': ')[1];
        const st = parts[3];
        statusFromRaw[side].set(sp, st);
      } else if (line.startsWith('|-curestatus|')) {
        const parts = line.split('|');
        const side = parts[2].slice(0, 2);
        const sp = parts[2].split(': ')[1];
        statusFromRaw[side].delete(sp);
      } else if (line.startsWith('|-cureteam|')) {
        const parts = line.split('|');
        const side = parts[2].slice(0, 2);
        statusFromRaw[side].clear();
      } else if (line.startsWith('|-weather|')) {
        const parts = line.split('|');
        lastRawWeather = parts[2];
      }
    }

    for (let s = 0; s < 2; s++) {
      const sLabel = s === 0 ? 'p1' : 'p2';
      for (let slot = 0; slot < preview[sLabel + 'Mons'].length; slot++) {
        const mon = preview[sLabel + 'Mons'][slot];
        const llogMon = llog.sides[s].mons[slot];
        const rawFainted = faintedFromRaw[sLabel].has(mon.species);
        if (llogMon.fainted !== rawFainted) {
          mismatches.faint.push({
            file: htmlFile,
            side: sLabel,
            species: mon.species,
            llogFainted: llogMon.fainted,
            rawFainted,
          });
        }
        if (!llogMon.fainted && !rawFainted && llogMon.appeared) {
          const rawSt = statusFromRaw[sLabel].get(mon.species) || '';
          const llogSt = llogMon.status || '';
          const normRaw = rawSt === 'tox' ? 'psn' : rawSt;
          const normLlog = llogSt === 'tox' ? 'psn' : llogSt;
          if (normRaw !== normLlog) {
            mismatches.status.push({
              file: htmlFile,
              side: sLabel,
              species: mon.species,
              llogStatus: llogMon.status,
              rawStatus: rawSt,
            });
          }
        }
      }
    }

    const llogWeather =
      typeof llog.weather === 'string'
        ? llog.weather
        : llog.weather && (llog.weather.key || llog.weather.type)
        ? llog.weather.key || llog.weather.type
        : 'none';
    const normRawWeather = !lastRawWeather || lastRawWeather === 'none' ? 'none' : String(lastRawWeather).toLowerCase();
    const normLlogWeather = !llogWeather || llogWeather === 'none' ? 'none' : String(llogWeather).toLowerCase();
    if (normRawWeather !== normLlogWeather) {
      mismatches.weather.push({
        file: htmlFile,
        rawWeather: normRawWeather,
        llogWeather: normLlogWeather,
      });
    }
  }

  const totalMismatches =
    mismatches.faint.length +
    mismatches.status.length +
    mismatches.weather.length +
    mismatches.unrecognized.length;

  if (totalMismatches > 0) {
    fs.writeFileSync(traceFile, JSON.stringify(mismatches, null, 2), 'utf8');
  } else if (fs.existsSync(traceFile)) {
    try { fs.unlinkSync(traceFile); } catch {}
  }

  const summary = {
    total_battles: auditedBattles,
    faint_mismatches: mismatches.faint.length,
    status_mismatches: mismatches.status.length,
    weather_mismatches: mismatches.weather.length,
    unrecognized_lines: mismatches.unrecognized.length,
    parity_rate: auditedBattles > 0 ? (auditedBattles - (totalMismatches > 0 ? 1 : 0)) / auditedBattles : 1.0,
    trace_file: totalMismatches > 0 ? traceFile : null,
  };

  console.log(JSON.stringify(summary));
  return summary;
}

if (require.main === module) {
  runRawDiff();
}

module.exports = { runRawDiff };
