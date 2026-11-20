'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {
  classifyPageWithRecovery, chooseCurrentFormFallback, transmissionBoundary
} = require('./pbs-castle-continue-on-error');

const readDecisionForm = html => {
  const a = html.match(/<input\b[^>]*name="ACTION"\s+value="(INPUT|CHANGE|BATONN)"/i);
  const n = html.match(/<input\b[^>]*name="NOW"\s+value="([0-9]*)"/i);
  if (!a || !n) throw new Error('FORM_UNREADABLE');
  return {action: a[1].toUpperCase(), now: n[1]};
};
function html(action, options, now='8') {
  return '<form method="post"><input name="ACTION" value="'+action+'">'+
    '<input name="NOW" value="'+now+'"><select name="MOVE">'+
    options.map(([value,label])=>'<option value="'+value+'">'+label+'</option>').join('')+
    '</select></form>';
}

test('input fallback uses only the current offered legal move', () => {
  const raw = html('INPUT', [['W1','X [0]'],['W2','Y [5]'],['C1','Z [20/100]']]);
  const answer = chooseCurrentFormFallback(raw, readDecisionForm);
  assert.equal(answer.status, 'READY');
  assert.deepEqual(answer.fields, {ACTION:'INPUT', NOW:'8', MOVE:'W2'});
});
test('forced change and baton pass only choose offered living switch targets', () => {
  for (const action of ['CHANGE','BATONN']) {
    const raw = html(action, [['C1','X [0/100]'],['C2','Y [40/80]'],['W1','X [20]']]);
    const answer = chooseCurrentFormFallback(raw, readDecisionForm);
    assert.equal(answer.status,'READY');
    assert.deepEqual(answer.fields, {ACTION:action,NOW:'8',MOVE:'C2'});
  }
});
test('automatic move A is accepted only if offered on INPUT', () => {
  const answer = chooseCurrentFormFallback(html('INPUT',[['A','automatic']]),readDecisionForm);
  assert.equal(answer.fields.MOVE,'A');
  const forced = chooseCurrentFormFallback(html('CHANGE',[['A','automatic']]),readDecisionForm);
  assert.equal(forced.status,'WAIT_FOR_FORM');
});
test('missing and duplicate options never invent W1', () => {
  const empty = chooseCurrentFormFallback(html('INPUT',[]),readDecisionForm);
  assert.equal(empty.status,'WAIT_FOR_FORM');
  const duplicate = chooseCurrentFormFallback(html('INPUT',[['W1','X'],['W1','X']]),readDecisionForm);
  assert.equal(duplicate.status,'WAIT_FOR_FORM');
  const invalid = chooseCurrentFormFallback(html('INPUT',[['W1','X']],'bad'),readDecisionForm);
  assert.equal(invalid.status,'WAIT_FOR_FORM');
});
test('unknown page with a valid decision form is recovered', () => {
  const raw = html('INPUT',[['W1','X [3]']]);
  const answer = classifyPageWithRecovery({statusCode:200,html:raw},
    () => ({kind:'UNKNOWN',html:raw}),readDecisionForm);
  assert.equal(answer.kind,'BATTLE_INPUT');
  assert.equal(answer.recoveredFrom,'RAW_FORM');
  assert.equal(classifyPageWithRecovery({statusCode:500,html:raw},
    () => ({kind:'UNKNOWN'}),readDecisionForm).kind,'UNKNOWN');
});
test('unknown page without a readable input remains retryable', () => {
  const raw='<html>temporarily unavailable</html>';
  const answer = classifyPageWithRecovery({statusCode:200,html:raw},
    () => ({kind:'UNKNOWN'}),readDecisionForm);
  assert.equal(answer.kind,'UNKNOWN');
});
test('post-attempt or duplicate is a transmission boundary; observation error is not', () => {
  assert.equal(transmissionBoundary(new Error('PUBLIC_MAXIMUM_HP_NOT_OBSERVED'),
    {submittedKeys:[]},{submittedKeys:[],halted:false}),false);
  assert.equal(transmissionBoundary(new Error('UNSUPPORTED_OBSERVATION_BOUNDARY'),
    {submittedKeys:[]},{submittedKeys:[],halted:false}),false);
  assert.equal(transmissionBoundary(new Error('SUBMISSION_PROGRESS_UNCONFIRMED'),
    {submittedKeys:[]},{submittedKeys:['x'],halted:true}),true);
  assert.equal(transmissionBoundary(new Error('DUPLICATE_SUBMISSION_BLOCKED'),
    {submittedKeys:['x']},{submittedKeys:['x'],halted:false}),true);
});
test('replay stored stopped input screen without networking', () => {
  const root=path.resolve(__dirname,'..');
  const trace=path.join(root,'data/pbs-traces','2026-11-20T16-29-14-394Z-K-g1.jsonl');
  if (!fs.existsSync(trace)) return;
  const events=fs.readFileSync(trace,'utf8').split(/\r?\n/).filter(Boolean).map(JSON.parse);
  const stopped=events.find(x=>x.type==='observation_stopped');
  assert.ok(stopped?.rawTurnHtml,'STOPPED_RAW_FORM_MISSING');
  const reader=require('./pbs-v32-decision-boundary').readDecisionForm;
  const answer=chooseCurrentFormFallback(stopped.rawTurnHtml,reader);
  assert.equal(answer.status,'READY');
  assert.equal(answer.fields.ACTION,'INPUT');
  assert.ok(/^(?:W[1-4]|C[1-3]|A)$/.test(answer.fields.MOVE));
  console.log('REAL_STOPPED_FORM_FALLBACK='+answer.fields.MOVE);
});

test('current-form fallback is accepted once by the existing submission guard', async () => {
  if (!fs.existsSync(path.join(__dirname,'pbs-castle-submission.js'))) return;
  const {createSubmissionGuard} = require('./pbs-castle-submission');
  const raw = html('INPUT', [['W1','X [5]']]);
  const bundle={rawTurnHtml:raw,rawPdatHtml:'pdat',rawLlogHtml:'llog'};
  let posts=0;
  const classifyPage = response => response.html==='WAITING_TURN'
    ? {kind:'WAITING_TURN'} : {kind:'BATTLE_INPUT'};
  const guard=createSubmissionGuard({
    matchId:'offline-fallback-only',readDecisionForm,classifyPage,
    getBundle:async()=>bundle,
    getPage:async()=>({statusCode:200,html:'WAITING_TURN'}),
    postOnce:async fields=>{posts++;return {statusCode:200,html:'WAITING_TURN'};}
  });
  const picked=chooseCurrentFormFallback(raw,readDecisionForm);
  const accepted=await guard.submit({bundle,fields:picked.fields});
  assert.equal(posts,1);
  assert.equal(accepted.confirmation.kind,'INPUT_ACKNOWLEDGED');
  await assert.rejects(guard.submit({bundle,fields:picked.fields}),/DUPLICATE_SUBMISSION_BLOCKED/);
  assert.equal(posts,1);
});

test('stale form is rejected before any POST and can be recaptured', async () => {
  if (!fs.existsSync(path.join(__dirname,'pbs-castle-submission.js'))) return;
  const {createSubmissionGuard} = require('./pbs-castle-submission');
  const raw=html('INPUT',[['W1','X [5]']]);
  const original={rawTurnHtml:raw,rawPdatHtml:'pdat',rawLlogHtml:'llog'};
  let posts=0;
  const guard=createSubmissionGuard({
    matchId:'offline-stale-form',readDecisionForm,
    classifyPage:()=>({kind:'BATTLE_INPUT'}),
    getBundle:async()=>({...original,rawLlogHtml:'new llog'}),
    getPage:async()=>({statusCode:200,html:raw}),
    postOnce:async()=>{posts++;return {statusCode:200,html:'WAITING_TURN'};}
  });
  await assert.rejects(guard.submit({bundle:original,
    fields:chooseCurrentFormFallback(raw,readDecisionForm).fields}),/STALE_OBSERVATION_BLOCKED/);
  assert.equal(posts,0);
  assert.equal(guard.snapshot().halted,false);
});