'use strict';

// Castle-only observation seam. The shared WASM is read, never patched or built.
// Each READY result owns a fresh searcher; its caller must free it. This module
// never chooses an action, performs search, or talks to the Castle server.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const {createReader} = require('./pbs-v32-source-reader');
const {readDecisionForm} = require('./pbs-v32-decision-boundary');
const {currentBoundary} = require('./pbs-castle-current-boundary');
const {buildCurrentRequest} = require('./pbs-castle-current-request');
const {capturePpObservation, auditPpImport} = require('./pbs-castle-pp-observation');
const {createObservationLedger} = require('./pbs-castle-observation-ledger');
const {completeExactHpPosition} = require('./pbs-exact-hp-position');
const {sourceHashes} = require('./pbs-castle-submission');
const clone = value => JSON.parse(JSON.stringify(value));
const id = value => String(value ?? '').normalize('NFD').toLowerCase().replace(/[^a-z0-9]/g, '');
const moveId = value => id(value).startsWith('hiddenpower') ? 'hiddenpower' : id(value);
const sha = value => crypto.createHash('sha256').update(value).digest('hex');
const inside = (root, filename) => filename.startsWith(root + path.sep);
const statusMatches = (observed, imported) => observed === imported ||
  (observed === 'psn' && imported === 'tox');

function repositoryPath(repo, filename, {directory = false, create = false} = {}) {
  assert.ok(typeof filename === 'string' && path.isAbsolute(filename), 'ABSOLUTE_REPOSITORY_PATH_REQUIRED');
  assert.ok(!filename.split(path.sep).includes('..'), 'PARENT_PATH_NOT_ALLOWED');
  const absolute = path.resolve(filename);
  assert.ok(inside(repo, absolute), 'PATH_OUTSIDE_REPOSITORY');
  let current = repo;
  const parts = path.relative(repo, absolute).split(path.sep);
  for (const [index, part] of parts.entries()) {
    current = path.join(current, part);
    let stat;
    try { stat = fs.lstatSync(current); } catch (error) {
      if (error.code !== 'ENOENT') throw error;
      if (!create) throw error;
      assert.ok(directory || index < parts.length - 1, 'CANNOT_CREATE_INPUT_FILE');
      fs.mkdirSync(current, {mode: 0o700});
      stat = fs.lstatSync(current);
    }
    assert.equal(stat.isSymbolicLink(), false, 'SYMLINK_REPOSITORY_PATH_REFUSED');
    if (index < parts.length - 1 || directory) assert.ok(stat.isDirectory(), 'DIRECTORY_REQUIRED');
    else assert.ok(stat.isFile(), 'REGULAR_INPUT_FILE_REQUIRED');
  }
  return absolute;
}

function normalizeLlog(html) {
  const openings = [...html.matchAll(/<td\s+valign="middle"\s*>/gi)].map(row => row.index);
  const changes = [];
  if (openings.length === 2 && !/<\/td\s*>\s*$/i.test(html.slice(openings[0], openings[1]))) {
    html = html.slice(0, openings[1]) + '</TD>' + html.slice(openings[1]);
    changes.push('CLOSE_FIRST_PUBLIC_PREVIEW_CELL');
  }
  html = html.replace(/<p\b[^>]*>[\s\S]*?<\/p>/gi, paragraph => {
    if (!/ACTION=PDAT/i.test(paragraph) || !/ACTION=SLOG/i.test(paragraph)) return paragraph;
    changes.push('REMOVE_PDAT_SLOG_NAVIGATION_PARAGRAPH');
    return '';
  });
  return {html, changes};
}

// Pure staging: failure downstream must not establish the initial maximum PP.
function preparePp(initial, observation, form, protocol, built) {
  assert.equal(observation.schema, 'pbs-observed-pp-v1');
  let basis = initial ? clone(initial) : null;
  if (!basis) {
    assert.equal(observation.decisionOrdinal, 1, 'INITIAL_SCREEN_REQUIRED');
    assert.equal(form.action, 'INPUT', 'INITIAL_INPUT_SCREEN_REQUIRED');
    assert.ok(form.now === '' || form.now === '0', 'INITIAL_UNSPENT_SCREEN_REQUIRED');
    assert.equal(form.decisionTurn, 1, 'INITIAL_TURN_REQUIRED');
    assert.ok(!protocol.some(line => /^\|(?:move|cant|-activate|-enditem|-transform|-setpp)\|/.test(line)),
      'INITIAL_SCREEN_ALREADY_HAS_MOVE_OR_PP_EVENTS');
    basis = clone(observation);
    for (const mon of basis.mons) for (const move of mon.moves) {
      assert.ok(Number.isSafeInteger(move.currentPp) && move.currentPp > 0, 'INITIAL_UNSPENT_PP_REQUIRED');
    }
  }
  assert.equal(observation.mySide, basis.mySide);
  assert.deepEqual(observation.pickedRosterSlots, basis.pickedRosterSlots);
  for (const mon of observation.mons) {
    const base = basis.mons.find(row => row.rosterSlot === mon.rosterSlot);
    assert.ok(base, 'INITIAL_ROSTER_SLOT_MISSING');
    assert.equal(mon.speciesId, base.speciesId);
    assert.equal(mon.level, base.level);
    assert.equal(mon.moves.length, 4);
    for (const [index, move] of mon.moves.entries()) {
      assert.equal(move.moveSlot, index + 1);
      assert.equal(move.sourceMoveId, base.moves[index].sourceMoveId,
        'COPIED_OR_REPLACED_MOVE_REQUIRES_SEPARATE_HANDLING');
      assert.equal(move.moveId, base.moves[index].moveId);
      assert.ok(Number.isSafeInteger(move.currentPp) && move.currentPp >= 0 &&
        move.currentPp <= base.moves[index].currentPp, 'OBSERVED_PP_EXCEEDS_INITIAL_MAXIMUM');
    }
  }
  const request = clone(built.request);
  assert.equal(request.side.id, 'p' + (observation.mySide + 1));
  const forced = Array.isArray(request.forceSwitch) && request.forceSwitch.some(Boolean);
  assert.equal(forced, form.action !== 'INPUT');
  if (!forced) {
    assert.equal(built.engineParty[0], built.activeRosterSlot);
    const current = observation.mons.find(mon => mon.rosterSlot === built.activeRosterSlot);
    const base = basis.mons.find(mon => mon.rosterSlot === built.activeRosterSlot);
    assert.ok(current && base);
    assert.equal(request.active.length, 1);
    assert.equal(request.active[0].moves.length, 4);
    for (const [index, row] of request.active[0].moves.entries()) {
      assert.equal(moveId(row.id), current.moves[index].moveId);
      assert.equal(row.pp, current.moves[index].currentPp);
      row.maxpp = base.moves[index].currentPp;
    }
  }
  return {initial: basis, request};
}

function publicOpponent(spec, preview, foe, observations) {
  const known = new Map(observations.map(row => [row.slot, row]));
  return spec.sides[foe].mons.map((mon, slot) => {
    assert.equal(id(mon.species), id(preview[slot].speciesId ?? preview[slot].species));
    const observed = known.get(slot);
    return {species: mon.species, speciesId: id(mon.species), level: mon.level,
      gender: mon.gender, itemFlag: preview[slot].item_flag,
      appeared: Boolean(mon.appeared), fainted: Boolean(mon.fainted),
      hp: observed?.hp ?? null, maxHp: observed?.maxHp ?? null, status: mon.status,
      itemOriginal: mon.item_original?.known ? mon.item_original.item : null,
      itemCurrent: mon.item_current?.known ? mon.item_current.item : null,
      itemGained: Boolean(mon.item_gained),
      itemOriginalKnown: Boolean(mon.item_original?.known), itemCurrentKnown: Boolean(mon.item_current?.known),
      noLeftovers: false, damagedTurnsCount: 0,
      revealedMoves: clone(mon.revealed_moves ?? []),
      uses: Object.fromEntries((mon.uses ?? []).map(row => [row.move, row.n]))};
  });
}

function createObservationSession(options = {}) {
  const {client, wasm, dex, dexData, side, matchId, poolProvider} = options;
  assert.ok(side === 0 || side === 1, 'PLAYER_SIDE_REQUIRED');
  assert.ok(typeof matchId === 'string' && matchId.length, 'MATCH_ID_REQUIRED');
  assert.ok(wasm && dex && client && dexData, 'OBSERVATION_RUNTIME_REQUIRED');
  assert.ok(poolProvider === undefined || typeof poolProvider === 'function', 'POOL_PROVIDER_REQUIRED');
  assert.ok(typeof options.repoRoot === 'string' && path.isAbsolute(options.repoRoot), 'ABSOLUTE_REPOSITORY_REQUIRED');
  const repo = path.resolve(options.repoRoot);
  assert.equal(fs.realpathSync(repo), repo, 'SYMLINK_REPOSITORY_REFUSED');
  assert.ok(typeof options.workDir === 'string' && path.isAbsolute(options.workDir), 'ABSOLUTE_WORK_DIRECTORY_REQUIRED');
  assert.ok(inside(path.join(repo, 'pbs-work'), path.resolve(options.workDir)), 'WORK_DIRECTORY_OUTSIDE_REPOSITORY_PBS_WORK');
  const workDir = repositoryPath(repo, options.workDir, {directory: true, create: true});
  const dictionaryPath = repositoryPath(repo, options.dictionaryPath || path.join(repo, 'tools/pbs-castle-dictionary.js'));
  const referencePath = repositoryPath(repo, path.join(repo, 'tools/pbs-converter-v32.reference.js'));
  const basePoolJson = options.poolJson ?? fs.readFileSync(repositoryPath(repo,
    path.join(repo, 'data/belief-pool-v3/belief-pool.json')), 'utf8');
  assert.equal(typeof basePoolJson, 'string', 'POOL_JSON_REQUIRED');
  assert.ok(Array.isArray(JSON.parse(basePoolJson).teams), 'POOL_TEAMS_REQUIRED');
  const seedValue = options.seed ?? 1, c = options.c ?? 0.4;
  assert.ok(Number.isSafeInteger(seedValue) && seedValue >= 0 && Number.isFinite(c) && c >= 0,
    'INVALID_SEARCHER_CONFIGURATION');
  // Matches the existing WASM constructor's u32 conversion without changing policy.
  const seed = seedValue >>> 0;
  const ownSets = clone(options.ownSets), previewSides = clone(options.previewSides);
  const pickedRosterSlots = clone(options.pickedRosterSlots);
  const reader = createReader({referencePath, dictionaryPath});
  const ledger = createObservationLedger({mySide: side, previewSides});
  let state = {ordinal: 0, initialPp: null, latestPp: null, latestHashes: null, latest: null};
  const consumed = new Set();
  let preparing = false;

  function writeEvidence(relative, contents) {
    const filename = path.join(workDir, relative);
    assert.ok(inside(workDir, filename), 'OUTPUT_OUTSIDE_SESSION');
    repositoryPath(repo, path.dirname(filename), {directory: true, create: true});
    if (fs.existsSync(filename)) {
      repositoryPath(repo, filename);
      assert.equal(fs.readFileSync(filename, 'utf8'), contents, 'EVIDENCE_FILENAME_COLLISION');
    } else fs.writeFileSync(filename, contents, {flag: 'wx', mode: 0o600});
    return filename;
  }

  function prepare(input) {
    assert.equal(preparing, false, 'REENTRANT_OBSERVATION_PREPARE_REFUSED');
    preparing = true;
    let imported = null, exact = null;
    try {
      const rawHashes = sourceHashes(input), bundleKey = sha(JSON.stringify(rawHashes));
      const duplicate = JSON.stringify(rawHashes) === JSON.stringify(state.latestHashes);
      assert.ok(duplicate || !consumed.has(bundleKey), 'OLDER_OBSERVATION_REPLAY_REFUSED');
      const ordinal = duplicate ? state.ordinal : state.ordinal + 1;
      const label = 'P' + (side + 1) + '-D' + String(ordinal).padStart(2, '0');
      const normalized = normalizeLlog(input.rawLlogHtml);
      const inputPath = writeEvidence('inputs/' + sha(normalized.html) + '.html', normalized.html);
      const evidence = reader.parseFile(inputPath);
      if (Array.isArray(evidence.unknown) && evidence.unknown.length) {
        const errorRecord = {schema: 'pbs-castle-text-error-v1', severity: 'ERROR',
          kind: 'UNRECOGNIZED_CASTLE_SOURCE_TEXT', matchId, label,
          normalizedLlogSha256: sha(normalized.html), entries: evidence.unknown};
        const serialized = JSON.stringify(errorRecord);
        writeEvidence('errors/' + sha(serialized) + '.json', serialized + '\n');
        process.stderr.write('[pbs-castle-observation] UNRECOGNIZED_CASTLE_SOURCE_TEXT count=' +
          evidence.unknown.length + ' label=' + label + '\n');
      }
      assert.deepEqual(evidence.automaticRepairs, [], 'AUTOMATIC_SOURCE_REPAIR_REFUSED');
      const boundary = currentBoundary(evidence, readDecisionForm(input.rawTurnHtml));
      const counts = [0, 0];
      boundary.protocol = boundary.protocol.map(line => {
        const fields = line.split('|');
        if (fields[1] !== 'poke') return line;
        assert.ok(fields[2] === 'p1' || fields[2] === 'p2', 'PUBLIC_PREVIEW_SIDE');
        const publicSide = Number(fields[2].slice(1)) - 1;
        const mon = previewSides[publicSide][counts[publicSide]++];
        assert.equal(typeof mon?.item_flag, 'boolean', 'PUBLIC_ITEM_FLAG_REQUIRED');
        return fields.slice(0, 4).join('|') + '|' + (mon.item_flag ? 'item' : '');
      });
      assert.deepEqual(counts, [6, 6], 'PUBLIC_PREVIEW_SLOTS_REQUIRED');
      const built = buildCurrentRequest({client, dexData, mySide: side, ownSets,
        previewSides, pickedRosterSlots, rawTurnHtml: input.rawTurnHtml,
        rawPdatHtml: input.rawPdatHtml, protocol: boundary.protocol,
        decisionTurn: boundary.form.decisionTurn});
      assert.equal(built.ui.actionType, boundary.form.action, 'FORM_ACTION_CONFLICT');
      assert.equal(built.ui.battleTurn.nowVal, boundary.form.now, 'FORM_NOW_CONFLICT');
      const observation = capturePpObservation({mySide: side, decisionOrdinal: ordinal,
        pickedRosterSlots, pdat: built.pdat, rawPdatSha256: rawHashes.rawPdatHtml});
      const ppStage = preparePp(state.initialPp, observation, boundary.form, boundary.protocol, built);
      built.request = ppStage.request;
      const staged = ledger.stage({evidence, built, label, rawHashes});
      const unmappedNarration = staged.sourceReport.inputBoundaries.filter(row =>
        row.kind === 'RECOGNIZED_SOURCE_WITHOUT_PROTOCOL' && row.blocksReady === false);
      if (unmappedNarration.length) {
        const errorRecord = {schema: 'pbs-castle-text-error-v1', severity: 'ERROR',
          kind: 'RECOGNIZED_SOURCE_WITHOUT_PROTOCOL', matchId, label,
          rawLlogSha256: rawHashes.rawLlogHtml, entries: unmappedNarration};
        const serialized = JSON.stringify(errorRecord);
        writeEvidence('errors/' + sha(serialized) + '.json', serialized + '\n');
        process.stderr.write('[pbs-castle-observation] RECOGNIZED_SOURCE_WITHOUT_PROTOCOL count=' +
          unmappedNarration.length + ' label=' + label + '\n');
      }
      const blockers = staged.sourceReport.inputBoundaries.filter(row => row.blocksReady);
      assert.equal(blockers.length, 0, 'UNSUPPORTED_OBSERVATION_BOUNDARY:' +
        blockers.map(row => row.kind).join(','));
      imported = new wasm.ProtocolSearcher(dex, side, basePoolJson, seed, c);
      imported.setOwnTeam(JSON.stringify(ownSets));
      imported.pushLines(JSON.stringify(boundary.protocol));
      assert.equal(imported.onRequest(JSON.stringify(built.request)), true, 'CURRENT_REQUEST_REJECTED');
      const spec = JSON.parse(imported.positionSpec()), foe = 1 - side;
      assert.equal(spec.side, side);
      assert.equal(spec.team_preview, false);
      assert.equal(spec.sides[side].active, built.activeRosterSlot);
      assert.equal(spec.force_switch, boundary.form.action !== 'INPUT');
      const publicTop = built.ui.battleTurn.sides[foe];
      if (publicTop.speciesId !== null && publicTop.hp > 0) {
        const active = spec.sides[foe].mons[built.activeSlots[foe]];
        assert.ok(statusMatches(publicTop.status, active.status), 'PUBLIC_TOP_SOURCE_STATUS_CONFLICT');
      }
      for (const row of staged.observations) {
        const mon = spec.sides[foe].mons[row.slot];
        assert.equal(id(mon.species), id(row.species));
        assert.equal(mon.fainted, row.hp === 0, 'PUBLIC_HP_FAINT_CONFLICT');
        mon.hp_num = row.hp;
        mon.hp_den = row.maxHp;
        // hp_exact is an own-side core facility. Opponent ratios are paired
        // with measured candidate maxHP and checked after final setPosition.
      }
      const seenFoeMons = publicOpponent(spec, previewSides[foe], foe, staged.observations);
      let prior = basePoolJson, hasCustomShellMemory = false;
      if (poolProvider) {
        const supplied = poolProvider({seenFoeMons: clone(seenFoeMons), spec: clone(spec),
          observations: clone(staged.observations), foeSide: foe});
        prior = typeof supplied === 'string' ? supplied : supplied?.json;
        hasCustomShellMemory = Boolean(supplied?.hasCustomShellMemory);
        assert.equal(typeof prior, 'string', 'POOL_PROVIDER_MUST_RETURN_JSON_SYNCHRONOUSLY');
      }
      exact = completeExactHpPosition({wasm, dex, side, poolJson: prior, spec,
        observations: staged.observations, speciesData: dexData.species, seed, c});
      assert.equal(exact.status, 'READY', 'PUBLIC_HP_POSITION_NOT_READY:' + exact.status + ':' + (exact.error ?? ''));
      const stateView = JSON.parse(exact.searcher.stateView());
      const positioned = JSON.parse(exact.searcher.positionSpec());
      assert.equal(positioned.sides[side].active, built.activeRosterSlot);
      assert.equal(positioned.force_switch, boundary.form.action !== 'INPUT');
      const ownHpChecks = [], ownStatusChecks = [], ownItemChecks = [];
      const publicStatusChecks = [], activePpChecks = [], acceptedPpGaps = [];
      for (const [enginePosition, rosterSlot] of built.engineParty.entries()) {
        const source = built.pdat.mons[pickedRosterSlots.indexOf(rosterSlot)];
        const native = stateView.sides[side].party[enginePosition];
        assert.equal(id(native.species), source.speciesId, 'OWN_SPECIES_MISMATCH');
        assert.equal(native.level, source.level, 'OWN_LEVEL_MISMATCH');
        assert.equal(native.hp, source.hp, 'OWN_HP_MISMATCH');
        assert.equal(native.maxhp, source.maxHp, 'OWN_MAXIMUM_HP_MISMATCH');
        assert.equal(native.fainted, source.hp === 0, 'OWN_FAINT_MISMATCH');
        if (source.hp > 0) {
          assert.ok(statusMatches(source.status, native.status), 'OWN_PDAT_STATUS_MISMATCH');
          ownStatusChecks.push({rosterSlot, observed: source.status, native: native.status, match: true});
        }
        assert.equal(id(native.item), id(source.item), 'OWN_CURRENT_ITEM_MISMATCH');
        ownItemChecks.push({rosterSlot, observed: source.item, native: native.item, match: true});
        ownHpChecks.push({rosterSlot, enginePosition, species: source.speciesId,
          hp: native.hp, maxHp: native.maxhp, match: true});
      }
      const ppAudit = auditPpImport({observation, stateView, engineParty: built.engineParty});
      for (const mismatch of ppAudit.mismatches) {
        assert.equal(mismatch.code, 'CURRENT_PP_VALUE', 'UNACCEPTED_PP_MISMATCH:' + mismatch.code);
        assert.ok(boundary.form.action !== 'INPUT' || mismatch.rosterSlot !== built.activeRosterSlot,
          'ACTIVE_CURRENT_PP_MISMATCH');
      }
      for (const check of ppAudit.comparisons) {
        const initialMon = ppStage.initial.mons.find(mon => mon.rosterSlot === check.rosterSlot);
        const maximumPp = initialMon.moves[check.moveSlot - 1].currentPp;
        const row = {...check, maximumPp, maximumPpAuthority: 'INITIAL_UNSPENT_PDAT_USER_CONFIRMED_RULE'};
        if (boundary.form.action === 'INPUT' && check.rosterSlot === built.activeRosterSlot) {
          assert.equal(check.nativeCurrentPp, check.observedCurrentPp, 'ACTIVE_CURRENT_PP_MISMATCH');
          assert.equal(check.nativeMaximumPp, maximumPp, 'ACTIVE_MAXIMUM_PP_MISMATCH');
          activePpChecks.push(row);
        } else if (check.nativeCurrentPp !== check.observedCurrentPp || check.nativeMaximumPp !== maximumPp) {
          acceptedPpGaps.push({...row, reason: 'ACCEPTED_EXISTING_CORE_BENCH_OR_FORCE_SWITCH_PP_LIMITATION'});
        }
      }
      assert.ok(exact.rootChecks.every(row => row.match), 'PUBLIC_ROOT_HP_MISMATCH');
      if (publicTop.speciesId !== null && publicTop.hp > 0) {
        const matches = stateView.sides[foe].party.filter(mon => id(mon.species) === publicTop.speciesId);
        assert.equal(matches.length, 1, 'PUBLIC_ACTIVE_IDENTITY_MISMATCH');
        assert.ok(statusMatches(publicTop.status, matches[0].status), 'PUBLIC_TOP_NATIVE_STATUS_MISMATCH');
        publicStatusChecks.push({rosterSlot: built.activeSlots[foe], observed: publicTop.status,
          native: matches[0].status, match: true});
      }
      const policy = JSON.parse(exact.searcher.rootPolicy());
      assert.equal(policy.iterations, 0, 'OBSERVATION_MUST_NOT_SEARCH');
      assert.deepEqual(policy.actions.map(row => row.input).sort(), built.expectedNativeInputs.slice().sort(),
        'RAW_LEGAL_SET_MISMATCH');
      for (const input of built.expectedNativeInputs) {
        const mapped = built.mapNativeInput(input);
        const matches = built.ui.battleTurn.options.filter(row => row.value === mapped.value);
        assert.equal(matches.length, 1, 'CASTLE_ACTION_NOT_UNIQUE');
        assert.equal(matches[0].kind, mapped.kind, 'CASTLE_ACTION_KIND_MISMATCH');
      }
      const metrics = JSON.parse(exact.searcher.metrics());
      assert.equal(metrics.projections, 0, 'PROJECTED_ACTION_NOT_ACCEPTED');
      assert.equal(metrics.legalityDrift, 0, 'LEGALITY_DRIFT_NOT_ACCEPTED');
      const report = {schema: 'pbs-castle-observation-report-v1', matchId, side, label, ordinal, duplicate,
        sourceHashes: clone(rawHashes), sourceReport: clone(staged.sourceReport),
        normalization: normalized.changes, sourceProtocolSha256: sha(evidence.raw),
        importedProtocolSha256: sha(boundary.protocol.join('\n') + '\n'),
        ownHpChecks, ownStatusChecks, ownItemChecks, publicHpChecks: clone(exact.rootChecks),
        publicStatusChecks, activePpChecks, acceptedPpGaps,
        legalMatch: true, inputBoundaries: clone(staged.sourceReport.inputBoundaries),
        initialPdatSha256: ppStage.initial.rawPdatSha256,
        maximumPpAuthority: 'INITIAL_UNSPENT_PDAT_USER_CONFIRMED_RULE',
        priorSource: poolProvider ? 'EXISTING_PUBLIC_ADAPTATION_PROVIDER' : 'SUPPLIED_BASE_PRIOR',
        hasCustomShellMemory, hpCompletion: clone(exact.completion),
        unverifiedLegacyItemAbsenceInferenceUsed: false,
        candidateCount: exact.afterBelief.count, seed, c,
        phase: {action: boundary.form.action, now: boundary.form.now,
          removedGeneratedUpkeep: clone(boundary.removed), appendedTurn: boundary.appendedTurn,
          residualCompletionCertified: false},
        requestNotes: built.notes.slice(),
        finalPositionSpecStage: 'post-exact-hp-position',
        finalPositionSpec: clone(positioned), completePositionCertified: false,
        allConverterBranchesCertified: false, searchRun: false, sentToCastle: false};
      const submission = {matchId, action: boundary.form.action, now: boundary.form.now,
        ordinal, sourceHashes: clone(rawHashes), rawOptionValues: built.ui.battleTurn.options.map(row => row.value)};
      const result = {status: 'READY', searcher: exact.searcher, built, spec: clone(spec),
        report: clone(report), submission, sourceHashes: clone(rawHashes),
        seenFoeMons: clone(seenFoeMons), lastFoeActiveSlot: built.activeSlots[foe]};
      if (!duplicate) {
        writeEvidence('ledger/' + label + '-' + bundleKey + '.json', JSON.stringify({
          report, ledger: staged.snapshot, submission}, null, 2) + '\n');
        const next = {ordinal, initialPp: clone(ppStage.initial), latestPp: clone(observation),
          latestHashes: clone(rawHashes), latest: clone(report)};
        staged.commit();
        state = next;
        consumed.add(bundleKey);
      }
      exact.searcher = null;
      return result;
    } finally {
      try { if (imported) imported.free(); }
      finally {
        if (exact?.searcher) exact.searcher.free();
        preparing = false;
      }
    }
  }
  return Object.freeze({prepare, snapshot: () => clone({...state, ledger: ledger.snapshot()})});
}

module.exports = {createObservationSession};
