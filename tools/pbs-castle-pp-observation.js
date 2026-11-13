'use strict';

// Pure, own-side observation and import audit. No engine input is constructed.
// The supplied digest and roster mapping are claims, not independently checked
// against raw HTML or a preview. Only the caller's own PDAT belongs here.
const OBSERVATION_SCHEMA = 'pbs-observed-pp-v1';
const CAPTURE_KEYS = ['mySide', 'decisionOrdinal', 'pickedRosterSlots', 'pdat', 'rawPdatSha256'];
const HP_TYPED = /^hiddenpower(?:bug|dark|dragon|electric|fighting|fire|flying|ghost|grass|ground|ice|poison|psychic|rock|steel|water)$/;
const isObject = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const natural = value => Number.isSafeInteger(value) && value >= 0;
function requireFact(ok, code) { if (!ok) throw new Error(code); }
function parsedId(value, code) {
  requireFact(typeof value === 'string' && /^[a-z][a-z0-9]*$/.test(value), code);
  return value;
}
function moveId(value, code) {
  const id = parsedId(value, code);
  return HP_TYPED.test(id) ? 'hiddenpower' : id;
}
function slots(value, code) {
  requireFact(Array.isArray(value) && value.length === 3 &&
    Array.from(value).every(slot => natural(slot) && slot <= 5) && new Set(value).size === 3, code);
}
function moveSourceId(move) {
  requireFact(isObject(move), 'PP_MOVE_SHAPE');
  const raw = move.moveId ?? move.id;
  moveId(raw, 'PP_MOVE_ID');
  if (move.moveId !== undefined && move.id !== undefined) {
    requireFact(move.moveId === move.id, 'PP_MOVE_ID_CONFLICT');
  }
  return raw;
}
function freeze(value) {
  if (value && typeof value === 'object') {
    for (const child of Object.values(value)) freeze(child);
    Object.freeze(value);
  }
  return value;
}

function capturePpObservation(input) {
  requireFact(isObject(input) && Object.keys(input).every(key => CAPTURE_KEYS.includes(key)),
    'PP_CAPTURE_ARGUMENTS');
  const {mySide, decisionOrdinal, pickedRosterSlots, pdat, rawPdatSha256} = input;
  requireFact(mySide === 0 || mySide === 1, 'PP_SIDE');
  requireFact(natural(decisionOrdinal) && decisionOrdinal > 0, 'PP_DECISION_ORDINAL');
  slots(pickedRosterSlots, 'PP_PICKED_ROSTER_SLOTS');
  requireFact(typeof rawPdatSha256 === 'string' && /^[a-f0-9]{64}$/.test(rawPdatSha256),
    'PP_SOURCE_SHA256');
  requireFact(isObject(pdat) && Object.keys(pdat).every(key => ['mons', 'foeRemaining'].includes(key)),
    'PP_OWN_PDAT_ONLY');
  requireFact(Array.isArray(pdat.mons) && pdat.mons.length === 3, 'PP_PDAT_MON_COUNT');
  const mons = Array.from(pdat.mons).map((mon, pdatPosition) => {
    requireFact(isObject(mon) && !Object.hasOwn(mon, '_unmappedJp') &&
      !Object.hasOwn(mon, '_unmappedMovesJp'), 'PP_UNRESOLVED_MON_OR_MOVES');
    if (Object.hasOwn(mon, 'side')) requireFact(mon.side === mySide, 'PP_FOREIGN_SIDE');
    const speciesId = parsedId(mon.speciesId, 'PP_SPECIES_ID');
    requireFact(natural(mon.level) && mon.level >= 1 && mon.level <= 100, 'PP_LEVEL');
    requireFact(Array.isArray(mon.moves) && mon.moves.length === 4, 'PP_MOVE_COUNT');
    const rosterSlot = pickedRosterSlots[pdatPosition];
    const moves = Array.from(mon.moves).map((move, index) => {
      const sourceMoveId = moveSourceId(move);
      requireFact(natural(move.pp), 'PP_CURRENT_VALUE');
      return {mySide, rosterSlot, moveSlot: index + 1, sourceMoveId,
        moveId: moveId(sourceMoveId, 'PP_MOVE_ID'), currentPp: move.pp,
        maximumPp: null, maximumPpAuthority: 'UNKNOWN', rawPdatSha256};
    });
    return {mySide, rosterSlot, pdatPosition, speciesId, level: mon.level, moves};
  });
  requireFact(new Set(mons.map(mon => mon.speciesId)).size === 3, 'PP_DUPLICATE_SPECIES');
  return freeze({schema: OBSERVATION_SCHEMA, mySide, decisionOrdinal,
    pickedRosterSlots: pickedRosterSlots.slice(), rawPdatSha256,
    source: 'CURRENT_OWN_PDAT', rawSourceAuthenticated: false,
    rosterMappingAuthority: 'CALLER_SUPPLIED_PDAT_POSITION_TO_ROSTER_SLOT',
    maximumPpAuthority: 'UNKNOWN', mons});
}

function validateObservation(observation) {
  requireFact(isObject(observation) && observation.schema === OBSERVATION_SCHEMA,
    'PP_OBSERVATION_SCHEMA');
  requireFact(observation.maximumPpAuthority === 'UNKNOWN', 'PP_MAXIMUM_AUTHORITY');
  requireFact(Array.isArray(observation.mons) && observation.mons.length === 3,
    'PP_OBSERVATION_MON_COUNT');
  const checked = capturePpObservation({mySide: observation.mySide,
    decisionOrdinal: observation.decisionOrdinal, pickedRosterSlots: observation.pickedRosterSlots,
    rawPdatSha256: observation.rawPdatSha256, pdat: {mons: observation.mons.map(mon => ({
      speciesId: mon?.speciesId, level: mon?.level,
      moves: mon?.moves?.map(move => ({moveId: move?.sourceMoveId, pp: move?.currentPp}))
    }))}});
  for (const [position, expected] of checked.mons.entries()) {
    const actual = observation.mons[position];
    requireFact(actual.mySide === checked.mySide && actual.rosterSlot === expected.rosterSlot &&
      actual.pdatPosition === position, 'PP_OBSERVATION_MAPPING');
    for (const [index, row] of expected.moves.entries()) {
      const supplied = actual.moves[index];
      requireFact(supplied.mySide === row.mySide && supplied.rosterSlot === row.rosterSlot &&
        supplied.moveSlot === row.moveSlot && supplied.moveId === row.moveId &&
        supplied.rawPdatSha256 === row.rawPdatSha256, 'PP_OBSERVATION_MOVE_MAPPING');
      requireFact(supplied.maximumPp === null && supplied.maximumPpAuthority === 'UNKNOWN',
        'PP_MAXIMUM_AUTHORITY');
    }
  }
  return checked;
}

// stateView is the parsed, existing WASM stateView result. engineParty maps its
// own party display positions to original roster slots. Move array order is
// authoritative for this comparison; matching by move name would hide swaps or
// conflate two copied slots with the same move. The opposite side is never read.
function auditPpImport({observation, stateView, engineParty} = {}) {
  const checked = validateObservation(observation);
  slots(engineParty, 'PP_ENGINE_PARTY');
  requireFact(engineParty.every(slot => checked.pickedRosterSlots.includes(slot)),
    'PP_ENGINE_PARTY_MEMBERSHIP');
  requireFact(isObject(stateView) && Array.isArray(stateView.sides) && stateView.sides.length === 2,
    'PP_STATE_VIEW');
  const own = stateView.sides[checked.mySide];
  requireFact(isObject(own) && Array.isArray(own.party), 'PP_OWN_STATE_VIEW');
  const mismatches = [], comparisons = [];
  function mismatch(code, where, field, expected, actual) {
    mismatches.push({code, ...where, field, expected, actual});
  }
  if (own.party.length !== 3) mismatch('PARTY_COUNT', {}, 'party.length', 3, own.party.length);
  for (const mon of checked.mons) {
    const enginePosition = engineParty.indexOf(mon.rosterSlot);
    const native = own.party[enginePosition];
    const where = {rosterSlot: mon.rosterSlot, enginePosition};
    const nativeSpecies = typeof native?.species === 'string'
      ? native.species.normalize('NFD').toLowerCase().replace(/[^a-z0-9]/g, '') : null;
    const nativeLevel = natural(native?.level) ? native.level : null;
    const monMatches = nativeSpecies === mon.speciesId && nativeLevel === mon.level;
    if (nativeSpecies !== mon.speciesId) {
      mismatch('SPECIES_IDENTITY', where, 'speciesId', mon.speciesId, nativeSpecies);
    }
    if (nativeLevel !== mon.level) mismatch('LEVEL_IDENTITY', where, 'level', mon.level, nativeLevel);
    const nativeMoves = Array.isArray(native?.moves) ? native.moves : [];
    if (nativeMoves.length !== 4) mismatch('MOVE_COUNT', where, 'moves.length', 4, nativeMoves.length);
    for (const row of mon.moves) {
      const at = {...where, moveSlot: row.moveSlot};
      const actual = nativeMoves[row.moveSlot - 1];
      let actualId = null;
      try { actualId = moveId(actual?.id, 'PP_NATIVE_MOVE_ID'); } catch (_) {}
      const actualPp = natural(actual?.pp) ? actual.pp : null;
      const actualMaxPp = natural(actual?.maxpp) ? actual.maxpp : null;
      const identityMatches = monMatches && actualId === row.moveId;
      if (actualId !== row.moveId) mismatch('MOVE_SLOT_IDENTITY', at, 'moveId', row.moveId, actualId);
      if (actualPp === null) mismatch('CURRENT_PP_INVALID', at, 'currentPp', row.currentPp, null);
      else if (identityMatches && actualPp !== row.currentPp) {
        mismatch('CURRENT_PP_VALUE', at, 'currentPp', row.currentPp, actualPp);
      }
      if (actualMaxPp === null) mismatch('ENGINE_MAXIMUM_PP_INVALID', at, 'maxpp', 'NONNEGATIVE_INTEGER', null);
      const enginePpWithinMaximum = actualPp === null || actualMaxPp === null
        ? null : actualPp <= actualMaxPp;
      if (enginePpWithinMaximum === false) {
        mismatch('ENGINE_PP_EXCEEDS_MAXIMUM', at, 'pp<=maxpp', true, false);
      }
      comparisons.push({mySide: checked.mySide, ...at, speciesId: mon.speciesId,
        moveId: row.moveId, nativeMoveId: actualId, observedCurrentPp: row.currentPp,
        nativeCurrentPp: actualPp, nativeMaximumPp: actualMaxPp,
        observedMaximumPp: null, maximumPpAuthority: 'UNKNOWN', identityMatches,
        currentPpMatches: identityMatches && actualPp !== null ? actualPp === row.currentPp : null,
        enginePpWithinMaximum, rawPdatSha256: checked.rawPdatSha256});
    }
  }
  return {schema: 'pbs-observed-pp-import-audit-v1', mySide: checked.mySide,
    decisionOrdinal: checked.decisionOrdinal, rawPdatSha256: checked.rawPdatSha256,
    status: mismatches.length ? 'MISMATCH' : 'MATCH',
    currentPpAllMatch: comparisons.every(row => row.currentPpMatches === true),
    maximumPpCompared: false, maximumPpAuthority: 'UNKNOWN',
    scope: 'OWN_CURRENT_PP_AND_ENGINE_PP_CONSISTENCY',
    rawSourceAuthenticated: false, nativeExecutionVerified: false,
    productionReadyCertified: false, comparisons, mismatches};
}

module.exports = {capturePpObservation, auditPpImport};
