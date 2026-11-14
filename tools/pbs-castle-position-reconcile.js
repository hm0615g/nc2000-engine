
'use strict';
// Castle-only corrections supported by independently observed source facts.
// Does not modify the shared engine, inferred opponent sets, or choose actions.
const assert = require('node:assert/strict');
const normalize = value => String(value ?? '').normalize('NFD').toLowerCase().replace(/[^a-z0-9]/g, '');
const moveKey = value => {
  const key = normalize(value);
  return key.startsWith('hiddenpower') ? 'hiddenpower' : key;
};
const clone = value => JSON.parse(JSON.stringify(value));

function reconcileOwnPp({spec, initial, current, activeSlot}) {
  assert.ok(spec && spec.schema === 'nc2000-position-v1', 'PP_POSITION_SPEC_REQUIRED');
  const side = spec.side;
  assert.ok(side === 0 || side === 1, 'PP_SIDE_REQUIRED');
  assert.equal(initial?.mySide, side, 'PP_INITIAL_SIDE_MISMATCH');
  assert.equal(current?.mySide, side, 'PP_CURRENT_SIDE_MISMATCH');
  assert.deepEqual(current.pickedRosterSlots, initial.pickedRosterSlots, 'PP_PICKED_SLOTS_CHANGED');
  assert.equal(current.mons.length, 3, 'PP_CURRENT_THREE_REQUIRED');
  assert.equal(initial.mons.length, 3, 'PP_INITIAL_THREE_REQUIRED');
  assert.equal(spec.sides[side].active, activeSlot, 'PP_ACTIVE_SLOT_MISMATCH');
  const entries = [];
  for (const observed of current.mons) {
    const base = initial.mons.find(row => row.rosterSlot === observed.rosterSlot);
    assert.ok(base, 'PP_MISSING_INITIAL_MON');
    const mon = spec.sides[side].mons[observed.rosterSlot];
    assert.ok(mon, 'PP_MISSING_POSITION_MON');
    assert.equal(normalize(mon.species), observed.speciesId, 'PP_SPECIES_MISMATCH');
    assert.equal(observed.speciesId, base.speciesId, 'PP_INITIAL_SPECIES_MISMATCH');
    assert.equal(mon.level, observed.level, 'PP_LEVEL_MISMATCH');
    assert.equal(observed.level, base.level, 'PP_INITIAL_LEVEL_MISMATCH');
    assert.equal(observed.moves.length, 4, 'PP_CURRENT_FOUR_MOVES_REQUIRED');
    assert.equal(base.moves.length, 4, 'PP_INITIAL_FOUR_MOVES_REQUIRED');
    assert.ok(Array.isArray(mon.uses), 'PP_PUBLIC_USES_REQUIRED');
    assert.ok(Array.isArray(mon.pp), 'PP_ACTIVE_PP_SHAPE');
    const known = new Set(base.moves.map(m => moveKey(m.moveId)));
    assert.equal(known.size, 4, 'PP_DUPLICATE_MOVE_KEYS');
    const existing = new Map();
    for (const use of mon.uses) {
      const k = moveKey(use.move);
      assert.ok(k && Number.isSafeInteger(use.n) && use.n >= 0, 'PP_INVALID_OLD_USE');
      assert.ok(!existing.has(k), 'PP_DUPLICATE_OLD_USES');
      existing.set(k, use.n);
    }
    const rewrites = [];
    for (let i=0; i<4; i++) {
      const observedMove = observed.moves[i], baseMove = base.moves[i];
      const key = moveKey(baseMove.moveId);
      assert.equal(observedMove.moveId, baseMove.moveId, 'PP_MOVE_ID_CHANGED');
      assert.equal(observedMove.sourceMoveId, baseMove.sourceMoveId, 'PP_SOURCE_MOVE_ID_CHANGED');
      assert.equal(observedMove.moveSlot, i+1, 'PP_MOVE_SLOT_CHANGED');
      assert.equal(baseMove.moveSlot, i+1, 'PP_INITIAL_MOVE_SLOT_CHANGED');
      const max = baseMove.currentPp, pp = observedMove.currentPp;
      assert.ok(Number.isSafeInteger(max) && max > 0, 'PP_INITIAL_MAXIMUM_INVALID');
      assert.ok(Number.isSafeInteger(pp) && pp >= 0 && pp <= max, 'PP_CURRENT_OUT_OF_RANGE');
      rewrites.push({move:key, n:max-pp});
      const old = existing.get(key) ?? 0;
      if (old !== max-pp) entries.push({rosterSlot:observed.rosterSlot, moveSlot:i+1, move:key,
        oldUses:old, correctedUses:max-pp, expectedPp:pp, sourceMaxPp:max});
    }
    // Retain use markers for non-set moves (e.g. publicly called moves). They
    // do not determine the base four PP slots and must not be silently lost.
    const extras = mon.uses.filter(u => !known.has(moveKey(u.move)));
    mon.uses = [...rewrites, ...extras];
  }
  return {schema:'pbs-castle-pp-reconciliation-v1',
    source:'INITIAL_UNSPENT_AND_CURRENT_OWN_PDAT', changedFields:entries};
}

function reconcileSafeguard({spec, evidence}) {
  assert.ok(spec && spec.schema === 'nc2000-position-v1', 'SAFEGUARD_POSITION_REQUIRED');
  const steps = evidence?.evidence?.steps;
  assert.ok(Array.isArray(steps), 'SAFEGUARD_SOURCE_STEPS_REQUIRED');
  const starts = [null, null];
  const verified = [];
  const emitted = [];
  for (let i=0;i<steps.length;i++) {
    const row = steps[i];
    if (row.kind !== 'text_line') continue;
    const text = row.text;
    if (typeof text !== 'string') continue;
    const success = /^(.+?)\u306f\u3057\u3093\u3074\u306e\u30f4\u30a7\u30fc\u30eb\u306b\u3064\u3064\u307e\u308c\u305f\u3002$/.exec(text);
    if (success) {
      const prev = steps[i-1];
      assert.ok(prev && prev.kind === 'text_line' && prev.segment === row.segment &&
        prev.paragraph === row.paragraph && prev.line + 1 === row.line &&
        prev.html === row.html, 'SAFEGUARD_SUCCESS_NOT_ADJACENT_TO_MOVE');
      assert.equal(prev.acknowledged, true, 'SAFEGUARD_MOVE_NOT_ACKNOWLEDGED');
      assert.equal(row.acknowledged, false, 'SAFEGUARD_SUCCESS_ALREADY_MAPPED');
      assert.deepEqual(row.protocolPatch.inserted, [], 'SAFEGUARD_SOURCE_ALREADY_INSERTED');
      assert.deepEqual(row.protocolPatch.removed, [], 'SAFEGUARD_SOURCE_ALREADY_REMOVED');
      const moves = prev.protocolPatch?.inserted || [];
      assert.equal(moves.length, 1, 'SAFEGUARD_MOVE_NOT_UNIQUE');
      const move = /^\|move\|p([12])a: ([^|]+)\|Safeguard\|/.exec(moves[0]);
      assert.ok(move, 'SAFEGUARD_SUCCESS_WITHOUT_VERIFIED_MOVE');
      assert.match(prev.text, /\u306e\u3057\u3093\u3074\u306e\u307e\u3082\u308a\uff01$/, 'SAFEGUARD_MOVE_TEXT_NOT_VERIFIED');
      assert.equal(row.ownerHint, prev.ownerHint, 'SAFEGUARD_ACTOR_OWNER_CHANGED');
      const turn = row.converterTurn;
      assert.ok(Number.isSafeInteger(turn) && turn > 0 && turn < spec.turn, 'SAFEGUARD_TURN_INVALID');
      const side = Number(move[1])-1;
      const previous = starts[side];
      assert.ok(previous === null || turn - previous >= 5,
        'SAFEGUARD_REPEATED_WHILE_ACTIVE');
      starts[side] = turn;
      verified.push({side, sourceStep:i, moveSourceStep:i-1, startTurn:turn,
        sourceText:text, actor:move[2]});
      emitted.push(i-1,i);
      continue;
    }
    if (/\u3057\u3093\u3074|\u30f4\u30a7\u30fc\u30eb/.test(text)) {
      // A move announcement alone does not prove its effect succeeded.
      if (/\u306e\u3057\u3093\u3074\u306e\u307e\u3082\u308a\uff01$/.test(text)) {
        const next = steps[i+1];
        assert.ok(next && next.kind === 'text_line' && next.segment === row.segment &&
          next.paragraph === row.paragraph && next.line === row.line + 1,
          'SAFEGUARD_RESULT_NOT_ADJACENT');
        const succeeded = /^(.+?)\u306f\u3057\u3093\u3074\u306e\u30f4\u30a7\u30fc\u30eb\u306b\u3064\u3064\u307e\u308c\u305f\u3002$/.test(next.text);
        const failed = next.text === '\u3057\u304b\u3057\u3001\u3046\u307e\u304f\u304d\u307e\u3089\u306a\u304b\u3063\u305f\u3002';
        assert.ok(succeeded || failed, 'SAFEGUARD_MOVE_WITHOUT_VERIFIED_SUCCESS_OR_FAILURE');
        if (failed) {
          const move = /^\|move\|p([12])a: ([^|]+)\|Safeguard\|/.exec(row.protocolPatch?.inserted?.[0] || '');
          assert.ok(move, 'SAFEGUARD_FAILED_MOVE_NOT_VERIFIED');
          assert.deepEqual(next.protocolPatch?.inserted, ['|-fail|p'+move[1]+'a: '+move[2]],
            'SAFEGUARD_FAILURE_PROTOCOL_NOT_VERIFIED');
          assert.equal(next.acknowledged, true, 'SAFEGUARD_FAILURE_NOT_ACKNOWLEDGED');
        }
      } else {
        assert.fail('UNRECOGNIZED_SAFEGUARD_SOURCE_LINE:' + text);
      }
    }
  }
  const active = [];
  const expired = [];
  assert.ok(Number.isSafeInteger(spec.turn) && spec.turn > 0,
    'SAFEGUARD_DECISION_TURN_INVALID');
  for (let side=0;side<2;side++) {
    const start = starts[side];
    if (start === null) continue;
    // Gen 2 Safeguard lasts through start+4; it has expired by start+5.
    // Source success is verified above; end is determined from turn number.
    if (spec.turn - start >= 5) {
      expired.push({side, startTurn:start, expiryTurn:start+5});
      continue;
    }
    const conditions = spec.sides[side].conditions;
    assert.ok(Array.isArray(conditions), 'SAFEGUARD_CONDITION_SHAPE');
    const prior = conditions.filter(row => row.key === 'safeguard');
    assert.equal(prior.length, 0, 'SAFEGUARD_ALREADY_PRESENT_IN_BASE_SPEC');
    conditions.push({key:'safeguard', start_turn:start});
    active.push({side, startTurn:start});
  }
  return {schema:'pbs-castle-safeguard-reconciliation-v1',
    source:'EXPLICIT_CASTLE_SUCCESS_PLUS_GEN2_FIVE_TURN_DURATION',
    active, expired, verifiedEvents:verified};
}

module.exports = {reconcileOwnPp,reconcileSafeguard};
