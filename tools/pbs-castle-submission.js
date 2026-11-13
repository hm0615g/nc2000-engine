'use strict';

// Castle-only transport boundary. All I/O is injected so this module can be
// tested without sockets, credentials, timers, or the battle engine.
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const sha256 = value => crypto.createHash('sha256').update(value).digest('hex');
const BATTLE_KINDS = new Set(['BATTLE_INPUT', 'BATTLE_BATONN', 'BATTLE_CHANGE']);
const RAW_KEYS = ['rawTurnHtml', 'rawPdatHtml', 'rawLlogHtml'];

function sourceHashes(bundle) {
  return Object.fromEntries(RAW_KEYS.map(key => {
    assert.ok(typeof bundle?.[key] === 'string' && bundle[key].length,
      'MISSING_OBSERVATION_SOURCE:' + key);
    return [key, sha256(bundle[key])];
  }));
}

function requirePage(response, label) {
  assert.ok(response && Number.isInteger(response.statusCode) &&
    response.statusCode >= 200 && response.statusCode < 300 &&
    typeof response.html === 'string' && response.html.length,
  'OBSERVATION_HTTP_FAILED:' + label);
  return response.html;
}

async function captureStableBundle({http, roomUrl, classifyPage, initialResponse}) {
  assert.equal(typeof http?.get, 'function');
  assert.equal(typeof classifyPage, 'function');
  const first = initialResponse || await http.get(roomUrl);
  const rawTurnHtml = requirePage(first, 'turn');
  assert.ok(BATTLE_KINDS.has(classifyPage(first).kind), 'NO_CURRENT_BATTLE_FORM');
  const rawPdatHtml = requirePage(await http.get(roomUrl + '?ACTION=PDAT'), 'PDAT');
  const rawLlogHtml = requirePage(await http.get(roomUrl + '?ACTION=LLOG'), 'LLOG');
  const last = await http.get(roomUrl);
  assert.equal(sha256(requirePage(last, 'turn-confirm')), sha256(rawTurnHtml),
    'OBSERVATION_CHANGED_WHILE_READING');
  return {rawTurnHtml, rawPdatHtml, rawLlogHtml, response: last};
}

function createSubmissionGuard({matchId, getBundle, getPage, getFinishedPage, postOnce,
  classifyPage, readDecisionForm, onEvent = () => {}}) {
  assert.ok(typeof matchId === 'string' && matchId.length, 'MATCH_ID_REQUIRED');
  for (const fn of [getBundle, getPage, postOnce, classifyPage, readDecisionForm, onEvent]) {
    assert.equal(typeof fn, 'function');
  }
  const submitted = new Set();
  let busy = false, halted = false;

  function progress(response, before) {
    if (!response || !Number.isInteger(response.statusCode) ||
        response.statusCode < 200 || response.statusCode >= 300) return null;
    const page = classifyPage(response);
    if (['WAITING_TURN', 'WAITING_SELECT', 'SELECT_DONE'].includes(page.kind)) {
      return {kind: 'INPUT_ACKNOWLEDGED', pageKind: page.kind};
    }
    if (page.kind === 'FINISHED' && page.reason === 'match_end_page') {
      return {kind: 'FINISHED', pageKind: page.kind};
    }
    if (!BATTLE_KINDS.has(page.kind)) return null;
    try {
      const next = readDecisionForm(response.html);
      const oldNow = Number(before.now || '0'), newNow = Number(next.now || '0');
      if (newNow > oldNow || (newNow === oldNow && next.action !== before.action)) {
        return {kind: 'NEXT_DECISION', pageKind: page.kind, action: next.action, now: next.now};
      }
    } catch {}
    // A changed byte, HTTP 200, or a repeated same-phase form does not prove
    // acceptance. In particular, do not resend an uncertain switch.
    return null;
  }

  async function submit({bundle, fields}) {
    assert.equal(halted, false, 'SUBMISSION_SESSION_HALTED');
    assert.equal(busy, false, 'SUBMISSION_ALREADY_IN_FLIGHT');
    assert.deepEqual(Object.keys(fields || {}).sort(), ['ACTION', 'MOVE', 'NOW']);
    const expected = sourceHashes(bundle);
    const form = readDecisionForm(bundle.rawTurnHtml);
    assert.equal(fields.ACTION, form.action, 'SUBMISSION_ACTION_MISMATCH');
    assert.equal(fields.NOW, form.now, 'SUBMISSION_NOW_MISMATCH');
    assert.match(fields.MOVE, form.action === 'INPUT' ? /^(?:W[1-4]|C[1-3]|A)$/ : /^C[1-3]$/);
    if (fields.MOVE === 'A') {
      const selection = bundle.rawTurnHtml.match(/<select[^>]*name="?MOVE"?[^>]*>([\s\S]*?)<\/select>/i);
      assert.ok(selection, 'CASTLE_A_SELECT_NOT_FOUND');
      const values = Array.from(selection[1].matchAll(/<option[^>]*value="?([^"\s>]+)"?[^>]*>/gi), m => m[1]);
      assert.equal(values.filter(value => value === 'A').length, 1, 'CASTLE_A_RAW_OPTION_NOT_UNIQUE');
      assert.ok(!values.some(value => /^W[1-4]$/.test(value)), 'CASTLE_A_MIXED_WITH_W');
    }
    const key = sha256(JSON.stringify({matchId, action: form.action, now: form.now, sources: expected}));
    assert.equal(submitted.has(key), false, 'DUPLICATE_SUBMISSION_BLOCKED');
    busy = true;
    let attempted = false;
    try {
      const fresh = await getBundle();
      assert.deepEqual(sourceHashes(fresh), expected, 'STALE_OBSERVATION_BLOCKED');
      assert.deepEqual(readDecisionForm(fresh.rawTurnHtml), form, 'STALE_FORM_BLOCKED');
      submitted.add(key);
      attempted = true;
      onEvent({type: 'submission_attempt', matchId, key, action: form.action,
        now: form.now, value: fields.MOVE, sourceHashes: expected, attempts: 1});
      let response = null, transportError = null;
      try {
        response = await postOnce({...fields});
      } catch (error) {
        transportError = String(error?.message || error);
      }
      let confirmation = progress(response, form);
      let reconciled = false;
      let finishSignal = Boolean(response && classifyPage(response).kind === 'FINISHED');
      if (!confirmation) {
        reconciled = true;
        onEvent({type: 'submission_reconcile', matchId, key,
          postAttempted: true, retryPost: false, transportError});
        try {
          response = await getPage();
          finishSignal = finishSignal || classifyPage(response).kind === 'FINISHED';
          confirmation = progress(response, form);
        } catch (error) {
          transportError = transportError || String(error?.message || error);
        }
      }
      if (!confirmation && finishSignal && typeof getFinishedPage === 'function') {
        try {
          const terminal = await getFinishedPage();
          const checked = progress(terminal, form);
          if (checked?.kind === 'FINISHED') {
            response = terminal;
            confirmation = {...checked, outcomeEvidence: 'MATCHED_FINAL_LLOG'};
          }
        } catch (error) {
          transportError = transportError || String(error?.message || error);
        }
      }
      if (!confirmation) {
        halted = true;
        const failure = new Error('SUBMISSION_PROGRESS_UNCONFIRMED');
        failure.code = 'SUBMISSION_PROGRESS_UNCONFIRMED';
        failure.submission = {matchId, key, action: form.action, now: form.now,
          postAttempts: 1, reconciled, transportError, halted: true};
        onEvent({type: 'submission_stopped', ...failure.submission});
        throw failure;
      }
      const result = {response, matchId, key, confirmation, reconciled,
        postAttempts: 1, transportError};
      onEvent({type: 'submission_confirmed', matchId, key, confirmation,
        reconciled, postAttempts: 1});
      return result;
    } catch (error) {
      if (attempted) halted = true;
      throw error;
    } finally {
      busy = false;
    }
  }
  return Object.freeze({submit, snapshot: () => ({matchId, busy, halted,
    submittedKeys: [...submitted]})});
}

module.exports = {captureStableBundle, createSubmissionGuard, sourceHashes};
