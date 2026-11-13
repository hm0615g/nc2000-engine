'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');

// Load the client without initializing the WASM search engine.
process.argv = [
  process.execPath,
  'tools/pbs-client.js',
  '--random'
];

const {
  classifyPage,
  parseBattleTurnHtml,
  interpretBattleRequest,
  resolveLegalPbsMoveOption
} = require('./pbs-client.js');

const fixture =
  '/home/hm/pbs-selfplay-runs/g1-1794171872250-cb8f2c/diagnostics/p2-downloaded/p2-current/current-room.html';

// Test 1: The actual Castle HTML captured after Baton Pass.
test(
  'saved live BATONN form is recognized correctly',
  () => {
    const html = fs.readFileSync(fixture, 'utf8');

    const page = classifyPage({
      statusCode: 200,
      html
    });

    assert.equal(page.kind, 'BATTLE_BATONN');

    const req = interpretBattleRequest(page);

    assert.equal(req.actionType, 'BATONN');
    assert.equal(req.isForceSwitch, true);
    assert.equal(req.battleTurn.nowVal, '1');

    assert.deepEqual(
      req.battleTurn.options.map(o => o.value),
      ['C2', 'C3']
    );

    assert(
      req.battleTurn.options.every(
        o => o.kind === 'switch'
      )
    );

    // Choose a legal option without sending it to the Castle.
    const picked = resolveLegalPbsMoveOption({
      bestChoice: null,
      rootPolicy: null,
      spec: null,
      pickedRosterSlots: null,
      options: req.battleTurn.options,
      ownSets: [],
      isRandom: true
    });

    assert(
      ['C2', 'C3'].includes(picked.value)
    );

    // Verify the exact field names and values that would be sent.
    const fields = {
      ACTION: req.actionType,
      MOVE: picked.value,
      NOW: req.battleTurn.nowVal
    };

    assert.equal(fields.ACTION, 'BATONN');
    assert.equal(fields.NOW, '1');

    assert(
      ['C2', 'C3'].includes(fields.MOVE)
    );

    console.log('BATONN_REAL_HTML=PASS');
  }
);

// Test 2: Existing INPUT and CHANGE actions must remain unchanged.
test(
  'normal INPUT and forced CHANGE keep their action names',
  () => {
    for (const [action, kind, isForced] of [
      ['INPUT', 'BATTLE_INPUT', false],
      ['CHANGE', 'BATTLE_CHANGE', true]
    ]) {
      const html =
        `<form><input name="ACTION" value="${action}">` +
        '<input name="NOW" value="1">' +
        '<select name="MOVE">' +
        '<option value="C2">test</select></form>';

      const page = classifyPage({
        statusCode: 200,
        html
      });

      assert.equal(page.kind, kind);

      const req = interpretBattleRequest(page);

      assert.equal(req.actionType, action);
      assert.equal(req.isForceSwitch, isForced);
    }

    console.log('LEGACY_ACTIONS_UNCHANGED=PASS');
  }
);

// Test 3: Reject unexpected Baton Pass forms.
test(
  'unexpected BATONN form is rejected safely',
  () => {
    const html =
      '<form>' +
      '<input name="ACTION" value="BATONN">' +
      '<input name="NOW" value="1">' +
      '<select name="MOVE">' +
      '<option value="W1">unexpected</select>' +
      '</form>';

    const page = classifyPage({
      statusCode: 200,
      html
    });

    assert.throws(
      () => interpretBattleRequest(page),
      /BATONN page/
    );

    console.log('BATONN_FAIL_CLOSED=PASS');
  }
);
