'use strict';
const assert = require('node:assert/strict');
const REFERENCE = '79dff80a4689bdb3869fa8649887f5108e4ebf6a30ff146b68d9fa2c69540e4e';

// Current-request boundary only; this does not certify all residual timing.
// A generated standalone upkeep is not a
// Castle observation. Weather [upkeep] notifications remain separate events.
// Converter turn numbers are retained as adapter metadata, not phase proof.
function currentBoundary(evidence, form) {
  assert.equal(evidence.referenceSha256, REFERENCE);
  assert.equal(evidence.complete, false);
  assert.deepEqual(evidence.errors, []);
  {
    // Unknown Castle narration must be retained verbatim and have no protocol effect.
    const __unknownSteps = evidence?.evidence?.steps;
    assert.ok(Array.isArray(evidence.unknown) && Array.isArray(__unknownSteps),
      'MALFORMED_UNKNOWN_SOURCE_EVIDENCE');
    const __expectedUnknown = __unknownSteps.filter(s=>s && s.acknowledged!==true).map(s=>{
      assert.ok(s.kind==='text_line' && typeof s.text==='string' && s.text.length>0 &&
        s.protocolPatch && Number.isInteger(s.protocolPatch.index) &&
        Array.isArray(s.protocolPatch.inserted) && s.protocolPatch.inserted.length===0 &&
        Array.isArray(s.protocolPatch.removed) && s.protocolPatch.removed.length===0,
        'UNKNOWN_TEXT_HAS_PROTOCOL_EFFECT');
      return 'segment: '+s.text;
    });
    assert.deepEqual([...evidence.unknown].sort(),__expectedUnknown.sort(),
      'UNPAIRED_UNKNOWN_SOURCE');
  }
  assert.equal(evidence.rejectReason, '');
  assert.ok(['INPUT', 'BATONN', 'CHANGE'].includes(form.action));
  assert.ok(form.now === '' || /^\d+$/.test(form.now));
  assert.equal(form.decisionTurn, Number(form.now || '0') + 1);
  assert.ok(Number.isSafeInteger(form.decisionTurn) && form.decisionTurn > 0);
  assert.deepEqual(evidence.protocol, evidence.raw.trimEnd().split(/\r?\n/));
  const removed = [];
  const protocol = evidence.protocol.filter((line, index) => {
    if (line !== '|upkeep') return true;
    removed.push({index, line, authority: 'CONVERTER_GENERATED_BOUNDARY'});
    return false;
  });
  const turns = protocol.filter(line => /^\|turn\|\d+$/.test(line));
  assert.ok(turns.length > 0);
  const lastTurn = Number(turns.at(-1).split('|')[2]);
  let appendedTurn = null;
  if (form.action === 'INPUT' && lastTurn + 1 === form.decisionTurn) {
    appendedTurn = '|turn|' + form.decisionTurn;
    protocol.push(appendedTurn);
  } else {
    assert.equal(lastTurn, form.decisionTurn, 'Current form/converter turn mismatch');
  }
  const gameplay = lines => lines.filter(line => !/^\|(?:turn|upkeep)(?:\||$)/.test(line));
  assert.deepEqual(gameplay(protocol), gameplay(evidence.protocol));
  return {protocol, form: {...form}, removed, appendedTurn,
    currentRequestIncluded: true, productionReady: false,
    residualCompletionCertified: false, converterTurnsAreObservationAuthority: false,
    policy: 'CURRENT_REQUEST_WITHOUT_SYNTHETIC_UPKEEP'};
}

module.exports = {currentBoundary};
