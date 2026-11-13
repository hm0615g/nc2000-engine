#!/usr/bin/env node
/**
 * Offline & Post-Match Parity Auditor for Gym Leader's Castle (`tools/pbs-client.js`).
 *
 * Inspects per-match raw trace bundles (`data/pbs-traces/*.jsonl`) containing:
 *   1. Raw HTML snapshots (`TEAM_PREVIEW_CHECK`, `BATTLE_INPUT`/`BATTLE_CHANGE`, `PDAT`, `LLOG`, `FINISHED`)
 *   2. Reconstructed `ProtocolTracker` / `nc2000-position-v1` states & sanitizer diffs
 *   3. Search outputs & `<select Name="MOVE">` projection paths
 *
 * Cross-checks independent information channels (Top Status Table vs `PDAT` vs `LLOG`
 * vs `PositionSpec` vs WASM `ProtocolSearcher.setPosition()`) to automatically detect
 * parser drift, silent exceptions, unrecognized Japanese log lines, PP/status/faint
 * desyncs, and action mapping fallbacks.
 *
 * Usage:
 *   node tools/pbs-parity-audit.js [trace-file-or-dir...] [--out data/pbs-anomalies.jsonl] [--no-reparse] [--strict]
 */
'use strict';

const fs = require('fs');
const path = require('path');
const {
  parseTeamPreviewHtml,
  parseBattleTurnHtml,
  parsePdatHtml,
  parseLlogHtml,
  serializeLlogTracker,
  mergeLlogIntoSeenFoeMons,
  buildAndSanitizePositionSpec,
  resolveBotOutcomeFromFinishedHtml,
} = require('./pbs-client.js');

const REPO = path.resolve(__dirname, '..');
const dexData = JSON.parse(fs.readFileSync(path.join(REPO, 'data/gen2stadium2.json'), 'utf8'));
const beliefPoolJson = fs.readFileSync(path.join(REPO, 'data/belief-pool-v3/belief-pool.json'), 'utf8');

let wasmModule = null;
let wasmDexInstance = null;
function getWasm() {
  if (!wasmModule) {
    wasmModule = require(path.join(REPO, 'crates/wasm/pkg-node/nc2000_wasm.js'));
    wasmDexInstance = new wasmModule.Dex();
  }
  return { wasm: wasmModule, wasmDex: wasmDexInstance };
}

function toId(s) {
  return String(s || '')
    .normalize('NFD')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '');
}

function statusesCompatible(tableStatus, llogStatus) {
  const a = String(tableStatus || '');
  const b = String(llogStatus || '');
  if (a === b) return true;
  // PS.cgi status cells may render 'どく' ('psn') for active badly-poisoned ('tox') mons
  if (a === 'psn' && b === 'tox') return true;
  return false;
}

/**
 * Validate a `PositionSpec` against the Rust WASM `ProtocolSearcher::setPosition()`
 * (`#[serde(deny_unknown_fields)]` + `PositionSpec::check()`).
 */
function validateSpecWithWasm(spec, side = 0) {
  try {
    const { wasm, wasmDex } = getWasm();
    const searcher = new wasm.ProtocolSearcher(wasmDex, side, beliefPoolJson, 1, 0.4);
    try {
      searcher.setPosition(JSON.stringify(spec));
      return { ok: true, error: null };
    } finally {
      if (typeof searcher.free === 'function') {
        try { searcher.free(); } catch {}
      }
    }
  } catch (err) {
    return { ok: false, error: String(err.message || err) };
  }
}

/**
 * Audit a single match's trace event array and return a structured parity report.
 */
function auditMatchTrace(traceEvents, options = {}) {
  const reparse = options.reparse !== false;
  const validateWasm = options.validateWasm !== false;
  const filePath = options.filePath || null;

  const events = Array.isArray(traceEvents) ? traceEvents : [];
  const startEv = events.find(e => e && e.type === 'match_start') || null;
  const previewEv = events.find(e => e && e.type === 'preview') || null;
  const decisionEvs = events.filter(e => e && e.type === 'decision');
  const finishEv = events.find(e => e && e.type === 'finish') || null;
  const errorPageEvs = events.filter(e => e && e.type === 'error_page');

  const matchId =
    startEv?.matchId ||
    previewEv?.matchId ||
    decisionEvs[0]?.matchId ||
    finishEv?.matchId ||
    (filePath ? path.basename(filePath, '.jsonl') : 'unknown-match');

  const anomalies = [];
  const pushAnomaly = (severity, code, turn, decision, message, details = {}) => {
    anomalies.push({
      severity,
      code,
      turn: turn ?? null,
      decision: decision ?? null,
      message,
      details,
    });
  };

  for (const errEv of errorPageEvs) {
    pushAnomaly('warn', 'PBS_ERROR_PAGE_ENCOUNTERED', null, null, `PBS returned ERROR page: ${errEv.errorText || 'ERROR'}`, {
      errorText: errEv.errorText || null,
    });
  }

  let ownSets = startEv?.team?.sets || previewEv?.spec?.own_sets || decisionEvs[0]?.spec?.own_sets || [];
  let mySide = previewEv?.mySide ?? decisionEvs[0]?.mySide ?? finishEv?.mySide ?? 0;
  let foeName = previewEv?.foeName ?? decisionEvs[0]?.foeName ?? finishEv?.foeName ?? null;
  let previewSides = null;
  let pickedRosterSlots = previewEv?.pickedRosterSlots || decisionEvs[0]?.pickedRosterSlots || null;

  // ------------------------------------------------------------------ 1. Preview Audit
  if (previewEv) {
    if (Array.isArray(previewEv.errors) && previewEv.errors.length > 0) {
      for (const err of previewEv.errors) {
        pushAnomaly('error', 'RUNTIME_EXCEPTION', 0, previewEv.decision, `Preview exception at ${err.stage}: ${err.message}`, err);
      }
    }

    let parsedPreview = previewEv.parsedPreview || null;
    if (reparse && previewEv.rawCheckHtml) {
      parsedPreview = parseTeamPreviewHtml(previewEv.rawCheckHtml);
      for (const [sideLabel, mons] of [['P1', parsedPreview.p1Mons], ['P2', parsedPreview.p2Mons]]) {
        for (let i = 0; i < (mons || []).length; i++) {
          if (mons[i]._unmappedJp) {
            pushAnomaly(
              'error',
              'UNMAPPED_SPECIES_JP',
              0,
              previewEv.decision,
              `Unmapped Japanese species "${mons[i]._unmappedJp}" in Team Preview (${sideLabel} slot ${i + 1})`,
              { side: sideLabel, slot: i, rawJp: mons[i]._unmappedJp }
            );
          }
        }
      }
    }

    if (parsedPreview) {
      previewSides = [parsedPreview.p1Mons || [], parsedPreview.p2Mons || []];
      if (previewSides[0].length !== 6 || previewSides[1].length !== 6) {
        pushAnomaly(
          'error',
          'PREVIEW_ROSTER_INVALID',
          0,
          previewEv.decision,
          `Expected 6v6 preview rosters, got ${previewSides[0].length}v${previewSides[1].length}`,
          { p1Count: previewSides[0].length, p2Count: previewSides[1].length }
        );
      }
    }

    if (Array.isArray(previewEv.pickedRosterSlots)) {
      const picks = previewEv.pickedRosterSlots;
      const uniq = new Set(picks);
      const levelSum = picks.reduce((acc, idx) => acc + (ownSets[idx]?.level || 55), 0);
      if (picks.length !== 3 || uniq.size !== 3 || picks.some(i => i < 0 || i >= 6) || levelSum > 155) {
        pushAnomaly(
          'error',
          'PREVIEW_PICK_ILLEGAL',
          0,
          previewEv.decision,
          `Illegal 3-mon preview pick [${picks.join(', ')}] (levelSum=${levelSum})`,
          { pickedRosterSlots: picks, levelSum }
        );
      }
    }

    if (Array.isArray(previewEv.sanitizeDiffs) && previewEv.sanitizeDiffs.length > 0) {
      pushAnomaly(
        'error',
        'SANITIZER_FORCED_CLAMP',
        0,
        previewEv.decision,
        `Sanitizer clamped ${previewEv.sanitizeDiffs.length} field(s) at preview`,
        { diffs: previewEv.sanitizeDiffs }
      );
    }

    if (validateWasm && previewEv.spec) {
      const wasmCheck = validateSpecWithWasm(previewEv.spec, mySide);
      if (!wasmCheck.ok) {
        pushAnomaly(
          'error',
          'WASM_SET_POSITION_REJECTED',
          0,
          previewEv.decision,
          `WASM ProtocolSearcher rejected preview spec: ${wasmCheck.error}`,
          { error: wasmCheck.error }
        );
      }
    }
  }

  if (!previewSides && finishEv?.previewSides) {
    previewSides = finishEv.previewSides;
  }

  // Track cumulative unrecognized lines so we only report each newly added line once
  const reportedUnrecognizedKeys = new Set();
  // Track opponent active observations across turns for bench re-entry parity
  const foeHistoryBySlot = new Map();

  // ------------------------------------------------------------------ 2. Turn-by-Turn Decision Audit
  for (const ev of decisionEvs) {
    const turn = ev.turn ?? 1;
    const dec = ev.decision ?? null;
    const side = ev.mySide ?? mySide;
    const foeSide = 1 - side;
    const isForceSwitch = Boolean(ev.forceSwitch || ev.phase === 'CHANGE');
    const slotsPicked = ev.pickedRosterSlots || pickedRosterSlots;

    if (Array.isArray(ev.errors) && ev.errors.length > 0) {
      for (const err of ev.errors) {
        pushAnomaly('error', 'RUNTIME_EXCEPTION', turn, dec, `Turn ${turn} exception at ${err.stage}: ${err.message}`, err);
      }
    }

    let battleTurn = ev.battleTurn || null;
    let pdat = ev.pdat || null;
    let llogSummary = ev.llogSummary || null;
    let specToCheck = ev.spec || null;

    if (reparse) {
      if (ev.rawTurnHtml) {
        battleTurn = parseBattleTurnHtml(ev.rawTurnHtml);
        for (let s = 0; s < 2; s++) {
          if (battleTurn.sides?.[s]?._unmappedJp) {
            pushAnomaly(
              'error',
              'UNMAPPED_BATTLE_TURN_JP',
              turn,
              dec,
              `Unmapped active species "${battleTurn.sides[s]._unmappedJp}" in top status table (side ${s})`,
              { side: s, rawJp: battleTurn.sides[s]._unmappedJp }
            );
          }
        }
        for (const opt of battleTurn.options || []) {
          if (opt._unmappedJp) {
            pushAnomaly(
              'error',
              'UNMAPPED_BATTLE_TURN_JP',
              turn,
              dec,
              `Unmapped ${opt.kind} option "${opt._unmappedJp}" (${opt.value}: ${opt.label})`,
              { option: opt, rawJp: opt._unmappedJp }
            );
          }
        }
      }

      if (ev.rawPdatHtml) {
        pdat = parsePdatHtml(ev.rawPdatHtml);
        for (let cIdx = 0; cIdx < (pdat.mons || []).length; cIdx++) {
          const pm = pdat.mons[cIdx];
          if (pm._unmappedJp) {
            pushAnomaly('error', 'UNMAPPED_PDAT_JP', turn, dec, `Unmapped species "${pm._unmappedJp}" in PDAT row ${cIdx + 1}`, {
              row: cIdx,
              rawJp: pm._unmappedJp,
            });
          }
          if (pm._unmappedMovesJp) {
            pushAnomaly('error', 'UNMAPPED_PDAT_JP', turn, dec, `Unmapped move(s) ${pm._unmappedMovesJp.join(', ')} in PDAT row ${cIdx + 1}`, {
              row: cIdx,
              rawMovesJp: pm._unmappedMovesJp,
            });
          }
          if (pm._unmappedItemJp) {
            pushAnomaly('error', 'UNMAPPED_PDAT_JP', turn, dec, `Unmapped item "${pm._unmappedItemJp}" in PDAT row ${cIdx + 1}`, {
              row: cIdx,
              rawItemJp: pm._unmappedItemJp,
            });
          }
        }
      }

      if (ev.rawLlogHtml !== undefined && ev.rawLlogHtml !== null) {
        const reparsedLlog = parseLlogHtml(ev.rawLlogHtml, previewSides || [[], []]);
        if (ev.rawTurnHtml && ownSets.length === 6) {
          const rawNowInt = battleTurn.nowVal === '' ? 0 : (parseInt(battleTurn.nowVal, 10) || 0);
          const turnNum = isForceSwitch ? Math.max(1, rawNowInt) : (rawNowInt + 1);
          const clonedSeenFoe = ev.seenFoeMonsBefore
            ? JSON.parse(JSON.stringify(ev.seenFoeMonsBefore))
            : (ev.seenFoeMonsAfter ? JSON.parse(JSON.stringify(ev.seenFoeMonsAfter)) : null);
          if (clonedSeenFoe && previewSides && previewSides[foeSide]) {
            mergeLlogIntoSeenFoeMons({
              llog: reparsedLlog,
              foeSide,
              seenFoeMons: clonedSeenFoe,
              previewSides,
              battleTurn,
              turnNum,
              isForceSwitch,
              lastFoeActiveSlot: ev.lastFoeActiveSlotBefore ?? null,
            });
          }
          const rebuiltSpec = buildAndSanitizePositionSpec({
            mySide: side,
            turn: turnNum,
            ownSets,
            previewSides,
            pickedRosterSlots: slotsPicked,
            battleTurn,
            pdat,
            llog: reparsedLlog,
            seenFoeMons: clonedSeenFoe,
            teamPreview: false,
            forceSwitch: isForceSwitch,
          });
          specToCheck = rebuiltSpec;
          if (ev.spec && JSON.stringify(rebuiltSpec) !== JSON.stringify(ev.spec)) {
            const castleLegacyReparse = ev.observationRoute === 'CASTLE_OBSERVATION_UNCHANGED_CORE'
              && ev.observationReport?.schema === 'pbs-castle-observation-report-v1';
            pushAnomaly(
              'warn',
              castleLegacyReparse ? 'CASTLE_LEGACY_REPARSE_NOT_COMPARABLE' : 'REPARSE_SPEC_DRIFT',
              turn,
              dec,
              castleLegacyReparse
                ? `Legacy parser PositionSpec is not equivalent to Castle observation state on turn ${turn}`
                : `Re-parsed PositionSpec on turn ${turn} differs from recorded spec`,
              { recordedTurn: ev.spec.turn, rebuiltTurn: rebuiltSpec.turn,
                castleObservation: castleLegacyReparse }
            );
          }
        }
        llogSummary = serializeLlogTracker(reparsedLlog);
      }
    }

    // Castle-only: verify the actual searcher position recorded after HP completion.
    // Old traces have no finalPositionSpec, so the legacy warning is preserved.
    if (ev.observationRoute === 'CASTLE_OBSERVATION_UNCHANGED_CORE'
        && ev.observationReport?.finalPositionSpecStage === 'post-exact-hp-position') {
      const finalSpec = ev.observationReport.finalPositionSpec;
      const picked = Array.isArray(ev.pickedRosterSlots) ? ev.pickedRosterSlots : pickedRosterSlots;
      const ownMons = finalSpec?.sides?.[side]?.mons;
      let valid = Boolean(finalSpec && finalSpec.schema === 'nc2000-position-v1'
        && finalSpec.side === side && Array.isArray(ownMons)
        && Array.isArray(picked) && picked.length === 3
        && new Set(picked).size === 3 && Array.isArray(pdat?.mons)
        && pdat.mons.length === 3);
      let opaqueBenchHpCount = 0;
      if (valid) {
        for (let i = 0; i < 3; i++) {
          const current = ownMons[picked[i]];
          const observed = pdat.mons[i];
          const sourceHp = observed?.hp;
          const sourceMaxHp = observed?.maxHp;
          const matchingHpChecks = Array.isArray(ev.observationReport.ownHpChecks)
            ? ev.observationReport.ownHpChecks.filter(row => row.rosterSlot === picked[i]
              && row.species === observed?.speciesId && row.hp === sourceHp
              && row.maxHp === sourceMaxHp && row.match === true) : [];
          const expectedPercent = Number.isInteger(sourceHp) && Number.isInteger(sourceMaxHp)
            && sourceMaxHp > 0 && sourceHp >= 0 && sourceHp <= sourceMaxHp
            ? Math.floor((200 * sourceHp + sourceMaxHp) / (2 * sourceMaxHp)) : null;
          const percentMatch = current?.hp_den === 100 && current.hp_num === expectedPercent;
          // The final PositionSpec may retain 48/48 on an unused, non-fainted bench slot.
          // This is NOT accepted as an exact HP reading: the independently checked
          // ProtocolSearcher stateView HP is attested by ownHpChecks instead.
          const opaqueBenchHp = current?.hp_num === 48 && current?.hp_den === 48
            && current.active === false && current.fainted === false && sourceHp > 0;
          if (opaqueBenchHp) opaqueBenchHpCount++;
          if (!current || !observed
              || toId(current.species) !== toId(observed.speciesId)
              || expectedPercent === null || matchingHpChecks.length !== 1
              || current.fainted !== (sourceHp === 0)
              || !(percentMatch || opaqueBenchHp)) {
            valid = false;
            break;
          }
        }
      }
      if (!valid) {
        pushAnomaly('error','CASTLE_FINAL_POSITION_PDAT_MISMATCH',turn,dec,
          `Final Castle searcher position fails own-HP/roster checks on turn ${turn}`,
          {side, pickedRosterSlots:picked});
      } else {
        specToCheck = finalSpec;
      }
      if (ev.observationReport.completePositionCertified !== true
          || ev.observationReport.allConverterBranchesCertified !== true
          || ev.observationReport.phase?.residualCompletionCertified !== true
          || (ev.observationReport.acceptedPpGaps?.length || 0) > 0
          || opaqueBenchHpCount > 0) {
        pushAnomaly('warn','CASTLE_POSITION_CERTIFICATION_LIMITED',turn,dec,
          `Castle final position is only partially certified on turn ${turn}`,
          {completePositionCertified:ev.observationReport.completePositionCertified,
           allConverterBranchesCertified:ev.observationReport.allConverterBranchesCertified,
           residualCompletionCertified:ev.observationReport.phase?.residualCompletionCertified,
           acceptedPpGapCount:ev.observationReport.acceptedPpGaps?.length || 0,
           opaqueBenchHpCount});
      }
    }

    // Check Sanitizer forced clamps
    const sanitizeDiffs =
      specToCheck && Array.isArray(specToCheck._sanitizeDiffs) && specToCheck._sanitizeDiffs.length > 0
        ? specToCheck._sanitizeDiffs
        : (Array.isArray(ev.sanitizeDiffs) ? ev.sanitizeDiffs : []);
    if (Array.isArray(sanitizeDiffs) && sanitizeDiffs.length > 0) {
      pushAnomaly(
        'error',
        'SANITIZER_FORCED_CLAMP',
        turn,
        dec,
        `Sanitizer forcibly clamped ${sanitizeDiffs.length} field(s) on turn ${turn}: ${sanitizeDiffs.map(d => `${d.path}(${d.reason})`).join(', ')}`,
        { diffs: sanitizeDiffs }
      );
    }

    // Check WASM PositionSpec validity
    if (validateWasm && specToCheck) {
      const wasmCheck = validateSpecWithWasm(specToCheck, side);
      if (!wasmCheck.ok) {
        pushAnomaly(
          'error',
          'WASM_SET_POSITION_REJECTED',
          turn,
          dec,
          `WASM ProtocolSearcher rejected spec on turn ${turn}: ${wasmCheck.error}`,
          { error: wasmCheck.error }
        );
      }
    }

    // Check unrecognized LLOG lines
    if (llogSummary && Array.isArray(llogSummary.unrecognizedLines)) {
      for (const u of llogSummary.unrecognizedLines) {
        const key = `${u.turn}:${u.align}:${u.line}`;
        if (!reportedUnrecognizedKeys.has(key)) {
          reportedUnrecognizedKeys.add(key);
          pushAnomaly(
            'warn',
            'LLOG_UNRECOGNIZED_LINE',
            u.turn || turn,
            dec,
            `Unrecognized LLOG line on turn ${u.turn || turn} (${u.align}): "${u.line}"`,
            u
          );
        }
      }
    }

    // Cross-Source Parity 1: Active Species, Faint, & Status (LLOG vs Top Status Table)
    if (battleTurn && Array.isArray(battleTurn.sides) && llogSummary && Array.isArray(llogSummary.sides)) {
      for (const s of [0, 1]) {
        const topSide = battleTurn.sides[s];
        const llogSide = llogSummary.sides[s];
        if (!topSide || !llogSide) continue;

        const llogActiveSlot = llogSide.activeSlot;
        const llogActiveMon =
          llogActiveSlot !== null && llogActiveSlot !== undefined && llogSide.mons
            ? llogSide.mons[llogActiveSlot]
            : null;

        // 1a. Active species parity
        if (
          topSide.speciesId &&
          llogActiveSlot !== null &&
          llogActiveSlot !== undefined &&
          previewSides &&
          previewSides[s] &&
          previewSides[s][llogActiveSlot]
        ) {
          const expectedSpId = toId(
            previewSides[s][llogActiveSlot].speciesId || previewSides[s][llogActiveSlot].species
          );
          let transformedSpId = null;
          if (llogActiveMon && Array.isArray(llogActiveMon.transformedInto)) {
            const [tSide, tSlot] = llogActiveMon.transformedInto;
            const tRoster = previewSides[tSide]?.[tSlot];
            if (tRoster) transformedSpId = toId(tRoster.speciesId || tRoster.species);
          }
          if (topSide.speciesId !== expectedSpId && topSide.speciesId !== transformedSpId) {
            pushAnomaly(
              'error',
              'ACTIVE_SPECIES_MISMATCH',
              turn,
              dec,
              `Side ${s} active species mismatch on turn ${turn}: TopTable="${topSide.speciesId}" vs LLOG="${expectedSpId}" (slot ${llogActiveSlot})`,
              {
                side: s,
                topSpeciesId: topSide.speciesId,
                llogActiveSlot,
                llogSpeciesId: expectedSpId,
              }
            );
          }
        }

        // 1b. Active faint & status parity
        if (topSide.hp !== null && topSide.maxHp !== null && llogActiveMon) {
          const topFainted = topSide.hp <= 0 || topSide.status === 'fnt';
          const llogFainted = Boolean(llogActiveMon.fainted);
          if (topFainted !== llogFainted) {
            pushAnomaly(
              'error',
              'ACTIVE_FAINT_MISMATCH',
              turn,
              dec,
              `Side ${s} active faint mismatch on turn ${turn}: TopTable hp=${topSide.hp}/${topSide.maxHp} (fainted=${topFainted}) vs LLOG fainted=${llogFainted}`,
              {
                side: s,
                topHp: topSide.hp,
                topMaxHp: topSide.maxHp,
                topStatus: topSide.status,
                llogFainted,
              }
            );
          } else if (!topFainted && !llogFainted) {
            const topSt = topSide.status || '';
            const llogSt = llogActiveMon.status || '';
            if (!statusesCompatible(topSt, llogSt)) {
              pushAnomaly(
                'error',
                'ACTIVE_STATUS_MISMATCH',
                turn,
                dec,
                `Side ${s} active status mismatch on turn ${turn}: TopTable="${topSt}" vs LLOG="${llogSt}"`,
                {
                  side: s,
                  speciesId: topSide.speciesId,
                  topStatus: topSt,
                  llogStatus: llogSt,
                }
              );
            }
          }
        }
      }
    }

    // Cross-Source Parity 2: PDAT Self-Consistency & Foe Remaining Count
    if (pdat && Array.isArray(pdat.mons) && pdat.mons.length > 0) {
      for (let cIdx = 0; cIdx < pdat.mons.length; cIdx++) {
        const pm = pdat.mons[cIdx];
        const rSlot = Array.isArray(slotsPicked) && slotsPicked[cIdx] !== undefined
          ? slotsPicked[cIdx]
          : (ownSets || []).findIndex(s => toId(s.species) === pm.speciesId);

        if (rSlot >= 0 && ownSets[rSlot]) {
          const expectedSpId = toId(ownSets[rSlot].species);
          if (pm.speciesId && pm.speciesId !== expectedSpId) {
            pushAnomaly(
              'error',
              'PDAT_SELF_PARITY_MISMATCH',
              turn,
              dec,
              `PDAT slot ${cIdx + 1} species "${pm.speciesId}" disagrees with pickedRosterSlots[${cIdx}]="${expectedSpId}"`,
              { cIdx, pdatSpeciesId: pm.speciesId, expectedSpId }
            );
          }
        }

        // Compare PDAT against `<select Name="MOVE">` bench option `C1..C3`
        if (battleTurn && Array.isArray(battleTurn.options)) {
          const cOpt = battleTurn.options.find(o => o.kind === 'switch' && o.slotIndex === cIdx + 1);
          if (cOpt) {
            if (
              (cOpt.speciesId && pm.speciesId && cOpt.speciesId !== pm.speciesId) ||
              (cOpt.hp !== null && cOpt.hp !== pm.hp) ||
              (cOpt.maxHp !== null && cOpt.maxHp !== pm.maxHp)
            ) {
              pushAnomaly(
                'error',
                'PDAT_SELF_PARITY_MISMATCH',
                turn,
                dec,
                `PDAT slot ${cIdx + 1} (${pm.speciesId} ${pm.hp}/${pm.maxHp}) disagrees with <select> ${cOpt.value} (${cOpt.speciesId} ${cOpt.hp}/${cOpt.maxHp})`,
                { cIdx, pdatMon: pm, selectOption: cOpt }
              );
            }
          }
        }

        // Compare PDAT against LLOG self status & faint
        const llogSelfMon =
          rSlot >= 0 && llogSummary?.sides?.[side]?.mons ? llogSummary.sides[side].mons[rSlot] : null;
        if (llogSelfMon && llogSelfMon.appeared) {
          const pmFainted = pm.hp <= 0 || pm.status === 'fnt';
          if (pmFainted !== Boolean(llogSelfMon.fainted)) {
            pushAnomaly(
              'error',
              'PDAT_SELF_PARITY_MISMATCH',
              turn,
              dec,
              `PDAT slot ${cIdx + 1} (${pm.speciesId}) faint=${pmFainted} disagrees with LLOG faint=${Boolean(llogSelfMon.fainted)}`,
              { cIdx, rSlot, pdatHp: pm.hp, pdatStatus: pm.status, llogFainted: llogSelfMon.fainted }
            );
          } else if (!pmFainted && !statusesCompatible(pm.status || '', llogSelfMon.status || '')) {
            pushAnomaly(
              'error',
              'PDAT_SELF_PARITY_MISMATCH',
              turn,
              dec,
              `PDAT slot ${cIdx + 1} (${pm.speciesId}) status="${pm.status || ''}" disagrees with LLOG status="${llogSelfMon.status || ''}"`,
              { cIdx, rSlot, pdatStatus: pm.status || '', llogStatus: llogSelfMon.status || '' }
            );
          }
        }
      }

      // Check `相手の残り：N` (`pdat.foeRemaining`) against LLOG opponent fainted count
      if (typeof pdat.foeRemaining === 'number' && llogSummary?.sides?.[foeSide]?.mons) {
        const foeMons = llogSummary.sides[foeSide].mons;
        const foeFaintedCount = foeMons.filter(m => m.fainted).length;
        const foeAppearedCount = foeMons.filter(m => m.appeared).length;
        const expectedRemaining = 3 - foeFaintedCount;
        if (pdat.foeRemaining !== expectedRemaining || foeAppearedCount > 3) {
          pushAnomaly(
            'error',
            'PDAT_FOE_REMAINING_MISMATCH',
            turn,
            dec,
            `PDAT foeRemaining=${pdat.foeRemaining} disagrees with LLOG (3 - ${foeFaintedCount} fainted = ${expectedRemaining}, appeared=${foeAppearedCount})`,
            {
              pdatFoeRemaining: pdat.foeRemaining,
              llogFoeFaintedCount: foeFaintedCount,
              llogFoeAppearedCount: foeAppearedCount,
            }
          );
        }
      }
    }

    // Cross-Source Parity 3: Own Active Move PP Tracking (`<select>` W1..W4 vs `maxpp - LLOG uses`)
    if (!isForceSwitch && battleTurn && Array.isArray(battleTurn.options) && specToCheck) {
      const myActiveSlot = specToCheck.sides?.[side]?.active;
      const ownSet = myActiveSlot !== null && myActiveSlot !== undefined ? ownSets[myActiveSlot] : null;
      const llogSelfMon =
        myActiveSlot !== null && myActiveSlot !== undefined && llogSummary?.sides?.[side]?.mons
          ? llogSummary.sides[side].mons[myActiveSlot]
          : null;

      if (
        ownSet &&
        Array.isArray(ownSet.moves) &&
        llogSelfMon &&
        !llogSelfMon.transformedInto &&
        !llogSelfMon.mimicOverlay
      ) {
        const moveOpts = battleTurn.options.filter(o => o.kind === 'move');
        for (let mIdx = 0; mIdx < ownSet.moves.length; mIdx++) {
          const rawMove = ownSet.moves[mIdx];
          const mId = toId(rawMove);
          const baseMoveId = mId.startsWith('hiddenpower') ? 'hiddenpower' : mId;
          const dexMv = dexData.moves[mId] || dexData.moves[baseMoveId];
          const maxpp = dexMv ? Math.floor((dexMv.pp * 8) / 5) : 16;
          const wOpt = moveOpts.find(o => o.slotIndex === mIdx + 1 || o.moveId === baseMoveId);
          if (wOpt && wOpt.pp !== null && wOpt.pp !== undefined) {
            const usedCount = (llogSelfMon.uses && (llogSelfMon.uses[baseMoveId] || llogSelfMon.uses[mId])) || 0;
            const expectedPp = Math.max(0, maxpp - usedCount);
            if (wOpt.pp !== expectedPp) {
              pushAnomaly(
                'warn',
                'OWN_PP_TRACKING_MISMATCH',
                turn,
                dec,
                `Own active (${ownSet.species}) move #${mIdx + 1} (${baseMoveId}) PP in <select> is ${wOpt.pp}, but LLOG uses (${usedCount}) implies ${expectedPp}/${maxpp}`,
                {
                  species: ownSet.species,
                  moveId: baseMoveId,
                  selectPp: wOpt.pp,
                  expectedPp,
                  maxpp,
                  llogUses: usedCount,
                }
              );
            }
          }
        }
      }
    }

    // Cross-Source Parity 4: Opponent Bench Re-Entry Consistency
    if (battleTurn?.sides?.[foeSide] && previewSides?.[foeSide]) {
      const foeTop = battleTurn.sides[foeSide];
      const fSlot = foeTop.speciesId
        ? previewSides[foeSide].findIndex(m => m.speciesId === foeTop.speciesId)
        : -1;
      if (fSlot >= 0) {
        const prevObs = foeHistoryBySlot.get(fSlot);
        if (prevObs) {
          if (prevObs.maxHp && foeTop.maxHp && prevObs.maxHp !== foeTop.maxHp) {
            pushAnomaly(
              'error',
              'FOE_BENCH_REENTRY_MISMATCH',
              turn,
              dec,
              `Foe ${foeTop.speciesId} maxHp changed from ${prevObs.maxHp} (turn ${prevObs.turn}) to ${foeTop.maxHp} (turn ${turn})`,
              { slot: fSlot, speciesId: foeTop.speciesId, prevMaxHp: prevObs.maxHp, curMaxHp: foeTop.maxHp }
            );
          }
          if (prevObs.fainted && foeTop.hp > 0) {
            pushAnomaly(
              'error',
              'FOE_BENCH_REENTRY_MISMATCH',
              turn,
              dec,
              `Foe ${foeTop.speciesId} was marked fainted on turn ${prevObs.turn} but appeared with HP ${foeTop.hp}/${foeTop.maxHp} on turn ${turn}`,
              { slot: fSlot, speciesId: foeTop.speciesId, prevTurn: prevObs.turn, curHp: foeTop.hp }
            );
          }
        }
        foeHistoryBySlot.set(fSlot, {
          turn,
          hp: foeTop.hp,
          maxHp: foeTop.maxHp,
          status: foeTop.status || '',
          fainted: Boolean(foeTop.hp !== null && foeTop.hp <= 0),
        });
      }
    }

    // Cross-Source Parity 5: Action Projection & Legality Audit
    if (battleTurn && Array.isArray(battleTurn.options) && battleTurn.options.length > 0) {
      const submittedVal = ev.chosenMoveValue || ev.postFields?.MOVE;
      if (submittedVal && !battleTurn.options.some(o => o.value === submittedVal)) {
        pushAnomaly(
          'error',
          'ILLEGAL_SUBMITTED_OPTION',
          turn,
          dec,
          `Submitted MOVE="${submittedVal}" is not in <select Name="MOVE"> options [${battleTurn.options.map(o => o.value).join(', ')}]`,
          { submittedVal, options: battleTurn.options }
        );
      }
      if (!startEv?.random && battleTurn.options.length > 1) {
        const mp = String(ev.mappingPath || '');
        const castleObservedVerified = ev.observationRoute === 'CASTLE_OBSERVATION_UNCHANGED_CORE'
          && ev.observationReport?.schema === 'pbs-castle-observation-report-v1'
          && ['observed:search','observed:random','observed:only-option'].includes(mp)
          && typeof ev.bestChoice === 'string'
          && ev.bestChoice === ev.matchedAction
          && ev.chosenMoveValue === submittedVal
          && ev.postFields?.MOVE === submittedVal
          && Array.isArray(ev.errors) && ev.errors.length === 0
          && ev.observationReport.legalMatch === true
          && battleTurn.options.filter(o => o.value === submittedVal).length === 1;
        if (!mp.startsWith('direct:') && !castleObservedVerified) {
          const sev = mp.startsWith('policy-fallback:') ? 'warn' : 'error';
          pushAnomaly(
            sev,
            'ACTION_MAPPING_FALLBACK',
            turn,
            dec,
            `Action projection fell back to "${mp}" on turn ${turn} (bestChoice=${ev.bestChoice || 'null'} -> MOVE=${submittedVal})`,
            {
              mappingPath: mp,
              bestChoice: ev.bestChoice || null,
              matchedAction: ev.matchedAction || null,
              chosenMoveValue: submittedVal,
              options: battleTurn.options,
            }
          );
        }
      }
    }
  }

  // ------------------------------------------------------------------ 3. Finish & Outcome Audit
  if (finishEv) {
    if (Array.isArray(finishEv.errors) && finishEv.errors.length > 0) {
      for (const err of finishEv.errors) {
        pushAnomaly('error', 'RUNTIME_EXCEPTION', null, finishEv.decisions, `Finish exception at ${err.stage}: ${err.message}`, err);
      }
    }

    let finalLlogSummary = finishEv.finalLlogSummary || null;
    if (reparse && (finishEv.rawFinalLlogHtml || finishEv.rawFinishHtml)) {
      const combinedHtml = [finishEv.rawFinishHtml || '', finishEv.rawFinalLlogHtml || ''].filter(Boolean).join('\n');
      const recomputedOutcome = resolveBotOutcomeFromFinishedHtml(
        combinedHtml,
        startEv?.botName || 'nc2000bot',
        foeName
      );
      if (finishEv.outcome && recomputedOutcome !== finishEv.outcome && finishEv.finishTrigger === 'FINISHED') {
        pushAnomaly(
          'warn',
          'FINISH_OUTCOME_ANOMALY',
           null,
          finishEv.decisions,
          `Recomputed outcome "${recomputedOutcome}" differs from recorded outcome "${finishEv.outcome}"`,
          { recordedOutcome: finishEv.outcome, recomputedOutcome }
        );
      }
      const llogRaw = finishEv.rawFinalLlogHtml || finishEv.rawFinishHtml || '';
      if (llogRaw) {
        const reparsedFinal = parseLlogHtml(llogRaw, previewSides || [[], []]);
        finalLlogSummary = serializeLlogTracker(reparsedFinal);
      }
    }

    if (finalLlogSummary && Array.isArray(finalLlogSummary.unrecognizedLines)) {
      for (const u of finalLlogSummary.unrecognizedLines) {
        const key = `${u.turn}:${u.align}:${u.line}`;
        if (!reportedUnrecognizedKeys.has(key)) {
          reportedUnrecognizedKeys.add(key);
          pushAnomaly(
            'warn',
            'LLOG_UNRECOGNIZED_LINE',
            u.turn || null,
            finishEv.decisions,
            `Unrecognized final LLOG line on turn ${u.turn || '?'}: "${u.line}"`,
            u
          );
        }
      }
    }

    if (finishEv.outcome === 'END') {
      pushAnomaly(
        'warn',
        'FINISH_OUTCOME_ANOMALY',
        null,
        finishEv.decisions,
        `Match ended with unresolved outcome "END" (trigger=${finishEv.finishTrigger || 'unknown'})`,
        { finishTrigger: finishEv.finishTrigger }
      );
    }

    if (finalLlogSummary?.sides && (finishEv.outcome === 'WIN' || finishEv.outcome === 'LOSE')) {
      const myFaints = (finalLlogSummary.sides[mySide]?.mons || []).filter(m => m.fainted).length;
      const foeFaints = (finalLlogSummary.sides[1 - mySide]?.mons || []).filter(m => m.fainted).length;
      if (finishEv.outcome === 'WIN' && myFaints >= 3 && foeFaints < 3) {
        pushAnomaly(
          'error',
          'FINISH_OUTCOME_ANOMALY',
          null,
          finishEv.decisions,
          `Outcome is WIN but final LLOG shows myFaints=${myFaints} vs foeFaints=${foeFaints}`,
          { outcome: finishEv.outcome, myFaints, foeFaints }
        );
      } else if (finishEv.outcome === 'LOSE' && foeFaints >= 3 && myFaints < 3) {
        pushAnomaly(
          'error',
          'FINISH_OUTCOME_ANOMALY',
          null,
          finishEv.decisions,
          `Outcome is LOSE but final LLOG shows foeFaints=${foeFaints} vs myFaints=${myFaints}`,
          { outcome: finishEv.outcome, myFaints, foeFaints }
        );
      }
    }
  }

  const errorCount = anomalies.filter(a => a.severity === 'error').length;
  const warnCount = anomalies.filter(a => a.severity === 'warn').length;

  return {
    matchId,
    filePath: filePath ? path.relative(REPO, filePath) : null,
    startedAt: startEv?.ts || null,
    finishedAt: finishEv?.ts || null,
    roomUrl: startEv?.roomUrl || null,
    botName: startEv?.botName || null,
    foeName,
    mySide,
    outcome: finishEv?.outcome || null,
    decisions: decisionEvs.length + (previewEv ? 1 : 0),
    ok: errorCount === 0,
    errorCount,
    warnCount,
    anomalies,
  };
}

function loadTraceEventsFromFile(filePath) {
  const raw = fs.readFileSync(filePath, 'utf8');
  const events = [];
  for (const line of raw.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    events.push(JSON.parse(trimmed));
  }
  return events;
}

function collectTraceFiles(inputs) {
  const targets = inputs.length > 0 ? inputs : [path.join(REPO, 'data/pbs-traces')];
  const files = [];
  for (const t of targets) {
    const resolved = path.resolve(t);
    if (!fs.existsSync(resolved)) continue;
    const st = fs.statSync(resolved);
    if (st.isDirectory()) {
      for (const entry of fs.readdirSync(resolved).sort()) {
        if (entry.endsWith('.jsonl')) {
          files.push(path.join(resolved, entry));
        }
      }
    } else if (st.isFile()) {
      files.push(resolved);
    }
  }
  return files;
}

function appendAnomalyReport(report, outPath) {
  if (!outPath || !report) return;
  try {
    fs.mkdirSync(path.dirname(outPath), { recursive: true });
    fs.appendFileSync(outPath, JSON.stringify(report) + '\n', { mode: 0o600 });
  } catch {
    // Non-fatal
  }
}

function runCli() {
  const argv = process.argv.slice(2);
  const inputs = [];
  let outPath = path.join(REPO, 'data/pbs-anomalies.jsonl');
  let reparse = true;
  let strict = false;
  let targetTurn = null;
  let expectAction = null;
  let emitRepro = null;
  let iters = null;

  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--out' && i + 1 < argv.length) {
      outPath = path.resolve(argv[++i]);
    } else if (a === '--no-out') {
      outPath = '';
    } else if (a === '--no-reparse') {
      reparse = false;
    } else if (a === '--strict' || a === '--fail-on-error') {
      strict = true;
    } else if (a === '--turn' && i + 1 < argv.length) {
      targetTurn = parseInt(argv[++i], 10);
    } else if (a === '--expect-action' && i + 1 < argv.length) {
      expectAction = argv[++i];
    } else if (a === '--emit-repro' && i + 1 < argv.length) {
      emitRepro = path.resolve(argv[++i]);
    } else if (a === '--iters' && i + 1 < argv.length) {
      iters = argv[++i];
    } else if (!a.startsWith('--')) {
      inputs.push(a);
    }
  }

  const files = collectTraceFiles(inputs);

  // Single-turn triage mode: Gate 1 check + delegate to tools/diagnose-position.js
  if (targetTurn !== null) {
    if (files.length === 0) {
      console.error(JSON.stringify({ status: 'error', message: 'No match trace file provided for turn triage' }));
      process.exit(1);
    }
    const file = files[0];
    const events = loadTraceEventsFromFile(file);
    const report = auditMatchTrace(events, { filePath: file, reparse });

    // Gate 1 check: Filter anomalies occurring on or before targetTurn
    const fatalAnomalies = report.anomalies.filter(
      anom => anom.turn !== null && anom.turn <= targetTurn && (anom.severity === 'error' || anom.code.includes('SAN'))
    );

    if (fatalAnomalies.length > 0) {
      console.log(
        JSON.stringify({
          schema: 'nc2000-triage-report-v1',
          match_id: report.matchId,
          turn: targetTurn,
          verdict: 'CATEGORY_A1_ADAPTER_DESYNC',
          upstream_report_allowed: false,
          message: 'Adapter desync or sanitizer clamp detected on or before target turn. Fix downstream adapter before reporting upstream.',
          anomalies: fatalAnomalies,
        })
      );
      process.exit(0);
    }

    const decisionEv = events.find(e => e && e.type === 'decision' && e.turn === targetTurn);
    if (!decisionEv || !decisionEv.spec) {
      console.error(JSON.stringify({ status: 'error', message: `No decision event with PositionSpec found on turn ${targetTurn}` }));
      process.exit(1);
    }

    // Write temp spec and invoke tools/diagnose-position.js
    const tmpSpecPath = path.join('/tmp', `trace-${report.matchId}-turn${targetTurn}.json`);
    fs.writeFileSync(tmpSpecPath, JSON.stringify(decisionEv.spec, null, 2));

    const diagArgs = ['tools/diagnose-position.js', '--spec', tmpSpecPath];
    if (expectAction) diagArgs.push('--expect-action', expectAction);
    if (emitRepro) diagArgs.push('--emit-repro', emitRepro);
    if (iters) diagArgs.push('--iters', iters);

    const { execFileSync } = require('child_process');
    try {
      const diagOutput = execFileSync('node', diagArgs, { cwd: REPO, encoding: 'utf8' }).trim();
      console.log(diagOutput);
    } catch (err) {
      console.error(JSON.stringify({ status: 'error', message: `diagnose-position failed: ${err.message}` }));
      process.exit(1);
    } finally {
      try { fs.unlinkSync(tmpSpecPath); } catch {}
    }
    return;
  }

  let totalDecisions = 0;
  let okMatches = 0;
  let totalErrors = 0;
  let totalWarnings = 0;
  const byCode = {};

  for (const file of files) {
    const events = loadTraceEventsFromFile(file);
    const report = auditMatchTrace(events, { filePath: file, reparse });
    totalDecisions += report.decisions;
    if (report.ok) okMatches++;
    totalErrors += report.errorCount;
    totalWarnings += report.warnCount;
    for (const anom of report.anomalies) {
      byCode[anom.code] = (byCode[anom.code] || 0) + 1;
    }
    if (report.anomalies.length > 0 && outPath) {
      appendAnomalyReport(report, outPath);
    }
  }

  console.log(
    JSON.stringify({
      matches: files.length,
      decisions: totalDecisions,
      ok_matches: okMatches,
      error_count: totalErrors,
      warn_count: totalWarnings,
      by_code: byCode,
    })
  );

  if (strict && totalErrors > 0) {
    process.exit(1);
  }
}

if (require.main === module) {
  runCli();
}

module.exports = {
  auditMatchTrace,
  loadTraceEventsFromFile,
  collectTraceFiles,
  appendAnomalyReport,
  validateSpecWithWasm,
  statusesCompatible,
};
