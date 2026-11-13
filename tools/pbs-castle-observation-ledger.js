'use strict';

// Castle-only source evidence and public HP. No transport, search, or core writes.
// A staged result is committed only after the caller verifies the final position.
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const REFERENCE_SHA = '79dff80a4689bdb3869fa8649887f5108e4ebf6a30ff146b68d9fa2c69540e4e';
const clone = value => JSON.parse(JSON.stringify(value));
const id = value => String(value ?? '').normalize('NFD').toLowerCase().replace(/[^a-z0-9]/g, '');
const hash = value => crypto.createHash('sha256').update(value).digest('hex');
const integer = (value, low, high, name) => {
  assert.ok(Number.isSafeInteger(value) && value >= low && value <= high, name);
  return value;
};
const lineList = (value, name) => {
  assert.ok(Array.isArray(value) && value.every(line => typeof line === 'string'), name);
  return value;
};
const sourceIdentity = step => JSON.stringify([step.kind, step.segment, step.paragraph,
  step.line, step.text, step.align, step.ownerHint]);

function replaySource(evidence) {
  const steps = evidence.evidence?.steps, segments = evidence.evidence?.segments;
  assert.ok(Array.isArray(steps) && Array.isArray(segments), 'Missing source evidence');
  const protocol = lineList(evidence.protocol, 'Missing final protocol');
  assert.deepEqual(protocol, evidence.raw.trimEnd().split(/\r?\n/), 'Raw/protocol mismatch');
  const effective = protocol.map(line => ({line, authority: 'CONVERTER_OUTSIDE_CAPTURE', sourceStep: null}));
  const assigned = new Set(), resolvedSegments = [];
  let cursor = 0, removedLines = 0, rewrittenSteps = 0;

  function apply(rows, stepIndex) {
    const step = steps[stepIndex], patch = step.protocolPatch;
    // The source reader emits an exact 'segment: ' prefix for unknown text.
    // Require one-to-one multiset coverage and zero generated protocol events.
    const unknownKey = step && typeof step.text === 'string' ? 'segment: ' + step.text : null;
    const matchingUnknownEntries = unknownKey && Array.isArray(evidence.unknown)
      ? evidence.unknown.filter(value => value === unknownKey).length : 0;
    const matchingUnknownSteps = step && typeof step.text === 'string'
      ? steps.filter(value => value && value.acknowledged !== true &&
        value.kind === 'text_line' && value.text === step.text).length : 0;
    const safeUnknownNarration = step && step.acknowledged !== true &&
      step.kind === 'text_line' && matchingUnknownEntries > 0 &&
      matchingUnknownEntries === matchingUnknownSteps &&
      patch && Number.isInteger(patch.index) &&
      Array.isArray(patch.inserted) && patch.inserted.length === 0 &&
      Array.isArray(patch.removed) && patch.removed.length === 0;
    assert.ok(step && typeof step.text === 'string' &&
      (step.acknowledged === true || safeUnknownNarration),
      'Unacknowledged or malformed source step: ' + stepIndex);
    assert.ok(patch && typeof patch === 'object', 'Missing source patch');
    integer(patch.index, 0, rows.length, 'Source patch index');
    lineList(patch.removed, 'Source removed lines');
    lineList(patch.inserted, 'Source inserted lines');
    assert.deepEqual(rows.slice(patch.index, patch.index + patch.removed.length).map(row => row.line),
      patch.removed, 'Source patch removal mismatch');
    assert.ok(!assigned.has(stepIndex), 'Source step belongs to two segments');
    assigned.add(stepIndex);
    if (patch.removed.length) { rewrittenSteps++; removedLines += patch.removed.length; }
    rows.splice(patch.index, patch.removed.length, ...patch.inserted.map(line => ({line,
      authority: 'CASTLE_SOURCE_INTERPRETATION', sourceStep: stepIndex})));
  }

  // Initial sendout patches address the global preamble, unlike text segments.
  const initial = steps.flatMap((step, index) => step.kind === 'initial_sendout' ? [index] : []);
  if (initial.length) {
    const first = steps[initial[0]].protocolPatch;
    integer(first?.index, 0, protocol.length, 'Initial source patch index');
    const rows = effective.slice(0, first.index).map(row => ({...row, authority: 'PUBLIC_PREVIEW_OR_METADATA'}));
    for (const index of initial) apply(rows, index);
    assert.deepEqual(rows.map(row => row.line), protocol.slice(0, rows.length), 'Initial source replay mismatch');
    effective.splice(0, rows.length, ...rows);
    cursor = rows.length;
  }

  for (const segment of segments) {
    integer(segment.stepStart, 0, steps.length, 'Segment start');
    integer(segment.stepEnd, segment.stepStart, steps.length, 'Segment end');
    const rows = lineList(segment.initialOutput, 'Segment initial output')
      .map(line => ({line, authority: 'CONVERTER_SEGMENT_MARKER', sourceStep: null}));
    for (let index = segment.stepStart; index < segment.stepEnd; index++) {
      assert.equal(steps[index].kind, 'text_line', 'Unexpected segment step kind');
      assert.equal(steps[index].segment, segment.index, 'Source segment mismatch');
      apply(rows, index);
    }
    const final = lineList(segment.finalOutput, 'Segment final output');
    assert.deepEqual(rows.map(row => row.line), final, 'Source segment replay mismatch');
    // The pinned converter discards segments containing only separator/time.
    const omitted = final.length <= 2;
    if (!omitted) {
      let at = cursor;
      while (at <= protocol.length - final.length &&
        JSON.stringify(protocol.slice(at, at + final.length)) !== JSON.stringify(final)) at++;
      assert.ok(at <= protocol.length - final.length, 'Resolved segment absent from final protocol');
      for (const gap of protocol.slice(cursor, at)) {
        assert.ok(/^\|(?:turn\|\d+|upkeep)$/.test(gap), 'Unattributed event between source segments: ' + gap);
      }
      effective.splice(at, final.length, ...rows);
      cursor = at + final.length;
    }
    resolvedSegments.push({index: segment.index, omitted, output: rows});
  }
  assert.equal(assigned.size, steps.length, 'Some source steps were dropped');

  // Standalone upkeep is generated; weather [upkeep] remains a source event.
  for (const row of effective) {
    if (row.line === '|upkeep') row.authority = 'CONVERTER_GENERATED_BOUNDARY';
    else if (/^\|turn\|\d+$/.test(row.line)) row.authority = 'CONVERTER_GENERATED_TURN';
  }
  return {steps: clone(steps), resolvedSegments, effectiveProtocol: effective,
    rewrittenSteps, removedLines};
}

function classifyBoundaries(source) {
  const boundaries = [];
  for (const [sourceStep, step] of source.steps.entries()) {
    if (step.protocolPatch.inserted.length || step.protocolPatch.removed.length) continue;
    let kind = 'RECOGNIZED_SOURCE_WITHOUT_PROTOCOL';
    if (/\u306f\u3053\u3089\u3048\u308b\u4f53\u52e2\u306b\u306f\u3044\u3063\u305f/.test(step.text)) {
      kind = 'ENDURE_STANCE_REQUIRES_EXPLICIT_INPUT';
    } else if (/\u307b\u308d\u3073\u306e\u30ab\u30a6\u30f3\u30c8\u304c[0\uff10]\u306b\u306a\u3063\u305f/.test(step.text)) {
      kind = 'PERISH_ZERO_IS_NOT_A_FAINT_EVENT';
    }
    boundaries.push({kind, sourceStep, text: step.text,
      blocksReady: kind !== 'RECOGNIZED_SOURCE_WITHOUT_PROTOCOL',
      reason: 'Preserved source observation has no certified current-input mapping.'});
  }
  for (const [protocolIndex, row] of source.effectiveProtocol.entries()) {
    const fields = row.line.split('|');
    if (row.authority === 'CONVERTER_OUTSIDE_CAPTURE' && row.line !== '|' &&
      !/^\|(?:t:|init|title|j|l|player|teamsize|gametype|gen|tier|rule|clearpoke|poke|teampreview|start|expire|deinit)(?:\||$)/.test(row.line)) {
      boundaries.push({kind: 'PROTOCOL_EVENT_WITHOUT_SOURCE_PROVENANCE', sourceStep: null,
        protocolIndex, line: row.line, blocksReady: true});
    }
    if (fields[1] === '-start' && id(fields[3]) === 'typechange') {
      boundaries.push({kind: 'CURRENT_TYPE_INPUT_UNSUPPORTED_BY_UNCHANGED_CORE',
        sourceStep: row.sourceStep, protocolIndex, line: row.line, blocksReady: true});
    }
  }
  return boundaries;
}

function createObservationLedger({mySide, previewSides}) {
  assert.ok(mySide === 0 || mySide === 1, 'Invalid player side');
  assert.ok(Array.isArray(previewSides) && previewSides.length === 2, 'Public preview required');
  const preview = previewSides.map((side, sideIndex) => {
    assert.ok(Array.isArray(side) && side.length === 6, 'Public preview must have six slots');
    const result = side.map((mon, slot) => {
      const species = id(mon.speciesId ?? mon.species);
      assert.ok(species, 'Missing public species');
      return {side: sideIndex, slot, species, level: integer(mon.level, 1, 100, 'Public level')};
    });
    assert.equal(new Set(result.map(mon => mon.species)).size, 6, 'Ambiguous public species');
    return result;
  });
  const foe = 1 - mySide, sideName = 'p' + (foe + 1);
  let state = {revision: 0, label: null, sourceIdentities: [], knownHp: [], latest: null};

  function stage({evidence, built, label, rawHashes}) {
    assert.equal(evidence?.referenceSha256, REFERENCE_SHA, 'Unpinned source evidence');
    assert.deepEqual(evidence.errors, [], 'Converter errors');
    assert.ok(Array.isArray(evidence.unknown), 'Malformed unknown source evidence');
    assert.equal(evidence.rejectReason, '', 'Rejected source evidence');
    assert.ok(typeof label === 'string' && label.length, 'Decision label required');
    assert.ok(rawHashes && typeof rawHashes === 'object', 'Raw source hashes required');
    for (const key of ['rawTurnHtml', 'rawPdatHtml', 'rawLlogHtml']) {
      assert.match(rawHashes[key], /^[a-f0-9]{64}$/, 'Invalid source hash: ' + key);
    }
    const source = replaySource(evidence), sourceIdentities = source.steps.map(sourceIdentity);
    assert.ok(sourceIdentities.length >= state.sourceIdentities.length, 'Source history regressed');
    assert.deepEqual(sourceIdentities.slice(0, state.sourceIdentities.length), state.sourceIdentities,
      'Source history prefix changed; retained public HP needs reconciliation');
    const nextKnown = new Map(state.knownHp.map(row => [row.slot, clone(row)]));
    const latestHp = new Map(), unlinkedHpIgnored = [], missingMaxHp = [];
    const inputBoundaries = classifyBoundaries(source);
    for (const [sourceStep, step] of source.steps.entries()) {
      assert.ok(Array.isArray(step.hpChanges), 'Missing source HP changes');
      for (const change of step.hpChanges) {
        if (change.side !== sideName) continue;
        const species = id(change.species), slot = preview[foe].findIndex(mon => mon.species === species);
        assert.ok(slot >= 0, 'Source HP species absent from public preview');
        integer(change.after, 0, 10000, 'Observed absolute HP');
        latestHp.set(slot, {hp: change.after, species, sourceStep,
          source: {kind: 'SOURCE_HP_CHANGE', label, sourceStep, rawSha256: rawHashes.rawLlogHtml,
            text: step.text, segment: step.segment, paragraph: step.paragraph, line: step.line}});
      }
    }

    const health = evidence.finalState?.monState?.[sideName];
    assert.ok(health?.kind === 'Map' && Array.isArray(health.entries), 'Missing converter HP evidence map');
    const finalHealth = new Map();
    for (const [speciesName, mon] of health.entries) {
      const species = id(speciesName), slot = preview[foe].findIndex(row => row.species === species);
      assert.ok(slot >= 0 && !finalHealth.has(slot), 'Invalid converter health identity');
      finalHealth.set(slot, mon);
      const linked = latestHp.get(slot);
      if (linked) assert.equal(mon.hpAbs, linked.hp, 'Final HP differs from source-linked change');
      else if (mon.hpAbs !== null && mon.hpAbs !== undefined) {
        unlinkedHpIgnored.push({slot, species, hp: mon.hpAbs, reason: 'NO_SOURCE_HP_CHANGE'});
      }
    }
    for (const slot of latestHp.keys()) assert.ok(finalHealth.has(slot), 'Source HP missing from final evidence');

    const top = built?.ui?.battleTurn?.sides?.[foe], active = built?.activeSlots?.[foe];
    integer(active, 0, 5, 'Public active roster slot');
    assert.ok(top && typeof top === 'object', 'Missing public top structure');
    const topAbsent = top.speciesId === null && top.hp === null && top.maxHp === null;
    if (!topAbsent) {
      const species = id(top.speciesId ?? top.species), old = nextKnown.get(active);
      assert.equal(species, preview[foe][active].species, 'Top/public active species mismatch');
      assert.equal(top.level, preview[foe][active].level, 'Top/public level mismatch');
      integer(top.maxHp, 1, 10000, 'Public maximum HP');
      integer(top.hp, 0, top.maxHp, 'Public current HP');
      if (old) assert.equal(top.maxHp, old.maxHp, 'Previously observed maximum HP changed');
      const fromSource = latestHp.get(active);
      if (fromSource) assert.equal(top.hp, fromSource.hp, 'Top/source HP disagreement');
      const topSource = {kind: 'PUBLIC_TOP', label, rawSha256: rawHashes.rawTurnHtml,
        observedSteps: source.steps.length};
      nextKnown.set(active, {side: foe, slot: active, species, hp: top.hp, maxHp: top.maxHp,
        maxSource: old?.maxSource ?? topSource, hpSource: topSource,
        observedSteps: source.steps.length});
    }

    for (const [slot, observed] of latestHp) {
      if (slot === active && !topAbsent) continue;
      const known = nextKnown.get(slot);
      if (!known) {
        missingMaxHp.push({slot, species: observed.species, hp: observed.hp,
          sourceStep: observed.sourceStep, reason: 'NO_INDEPENDENT_PUBLIC_MAXIMUM_HP'});
        continue;
      }
      integer(observed.hp, 0, known.maxHp, 'Source HP exceeds independent public maximum');
      // Replaying cumulative history must not replace a later top observation
      // with an earlier source event. Newly appended events may update it.
      if (observed.sourceStep >= known.observedSteps) {
        known.hp = observed.hp;
        known.hpSource = observed.source;
        known.observedSteps = source.steps.length;
      }
    }
    for (const row of missingMaxHp) inputBoundaries.push({kind: 'PUBLIC_MAXIMUM_HP_NOT_OBSERVED',
      ...row, blocksReady: true});
    const knownHp = [...nextKnown.values()].sort((a, b) => a.slot - b.slot);
    const observations = knownHp.map(({side, slot, species, hp, maxHp}) => ({side, slot, species, hp, maxHp}));
    const sourceReport = {schema: 'pbs-castle-source-ledger-report-v1', label,
      sourceSteps: source.steps.length, checkedSegments: source.resolvedSegments.length,
      sourceOnlySteps: source.steps.filter(step =>
        step.protocolPatch.inserted.length === 0 && step.protocolPatch.removed.length === 0).length,
      sourceHpRows: source.steps.filter(step => step.sourceHp !== null && step.sourceHp !== undefined).length,
      rewrittenSteps: source.rewrittenSteps, removedProtocolLines: source.removedLines,
      sourceProtocolSha256: hash(evidence.raw), inputSha256: evidence.inputSha256,
      historyPrefixVerified: true, exactOpponentObservations: observations.length,
      missingMaxHp, unlinkedHpIgnored, inputBoundaries,
      warnings: clone(evidence.warnings ?? []), automaticRepairs: clone(evidence.automaticRepairs ?? []),
      completePositionCertified: false};
    const snapshot = {schema: 'pbs-castle-observation-ledger-v1', label, mySide,
      revision: state.revision + 1, rawHashes: clone(rawHashes), knownPublicOpponentHp: clone(knownHp),
      sourceSteps: source.steps, resolvedSegments: source.resolvedSegments,
      effectiveProtocol: source.effectiveProtocol, sourceReport: clone(sourceReport)};
    const expectedRevision = state.revision;
    const nextState = {revision: expectedRevision + 1, label, sourceIdentities,
      knownHp: clone(knownHp), latest: clone(snapshot)};
    let committed = false;
    return {snapshot: clone(snapshot), observations: clone(observations), sourceReport: clone(sourceReport),
      commit() {
        assert.equal(committed, false, 'Observation stage already committed');
        assert.equal(state.revision, expectedRevision, 'Stale observation stage');
        assert.equal(inputBoundaries.some(row => row.blocksReady), false,
          'Cannot commit an unresolved source-input boundary');
        state = clone(nextState);
        committed = true;
        return state.revision;
      }};
  }
  return Object.freeze({stage, snapshot: () => clone(state)});
}

module.exports = Object.freeze({createObservationLedger, REFERENCE_SHA});
