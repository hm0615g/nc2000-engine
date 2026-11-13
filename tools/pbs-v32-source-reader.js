'use strict';

const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const REFERENCE_SHA =
  '79dff80a4689bdb3869fa8649887f5108e4ebf6a30ff146b68d9fa2c69540e4e';
const hash = x => crypto.createHash('sha256').update(x).digest('hex');

// Helpers are inserted only into an in-memory copy of the pinned converter.
function __v32Snapshot(state) {
  return JSON.parse(JSON.stringify(state, (key, value) => {
    if (key === '__v32Evidence') return undefined;
    if (value instanceof Map) return {kind:'Map', entries:[...value]};
    if (value instanceof Set) return {kind:'Set', values:[...value]};
    return value;
  }));
}

function __v32StateDiff(before, after, at = [], out = []) {
  if (JSON.stringify(before) === JSON.stringify(after)) return out;
  if (before && after && typeof before === 'object' &&
      typeof after === 'object' && !Array.isArray(before) &&
      !Array.isArray(after)) {
    for (const key of new Set([...Object.keys(before), ...Object.keys(after)])) {
      if (Object.hasOwn(before,key) && Object.hasOwn(after,key)) {
        __v32StateDiff(before[key],after[key],[...at,key],out);
      } else {
        out.push({path:[...at,key], beforePresent:Object.hasOwn(before,key),
          afterPresent:Object.hasOwn(after,key), before:before[key], after:after[key]});
      }
    }
  } else {
    out.push({path:at, beforePresent:true, afterPresent:true, before, after});
  }
  return out;
}

function __v32Capture(state, out, context, run) {
  const before = __v32Snapshot(state);
  const oldLines = out.slice();
  const hpBefore = {};
  for (const side of ['p1','p2']) {
    for (const [species, mon] of state.monState[side]) {
      hpBefore[side + ':' + species] = mon.hpAbs;
    }
  }

  const handled = run();

  let start = 0;
  while (start < oldLines.length && start < out.length &&
         oldLines[start] === out[start]) start++;

  let tail = 0;
  while (tail < oldLines.length - start && tail < out.length - start &&
         oldLines[oldLines.length - 1 - tail] === out[out.length - 1 - tail]) tail++;

  const hpChanges = [];
  for (const side of ['p1','p2']) {
    for (const [species, mon] of state.monState[side]) {
      const previous = hpBefore[side + ':' + species];
      if (previous !== mon.hpAbs) hpChanges.push({
        side, species, before:previous, after:mon.hpAbs,
        converterMaxHpEstimate:mon.maxHp,
        maxHpAuthoritative:false
      });
    }
  }

  const step = {
    ...context,
    acknowledged:handled === true,
    sourceHp:parseHpLine(context.text),
    protocolPatch:{
      index:start,
      removed:oldLines.slice(start,oldLines.length-tail),
      inserted:out.slice(start,out.length-tail)
    },
    stateChanges:__v32StateDiff(before,__v32Snapshot(state)),
    hpChanges
  };
  state.__v32Evidence.steps.push(step);
}

function createReader({referencePath, dictionaryPath} = {}) {
  referencePath = path.resolve(referencePath ||
    path.join(__dirname,'pbs-converter-v32.reference.js'));

  assert.ok(dictionaryPath, 'dictionaryPath is required');
  dictionaryPath = path.resolve(dictionaryPath);

  const bytes = fs.readFileSync(referencePath);
  assert.equal(hash(bytes),REFERENCE_SHA,'Reference SHA256 mismatch');

  let source = bytes.toString('utf8');
  assert.ok(!source.includes('__v32Evidence'),
    'Unexpected instrumented reference');

  function once(before, after) {
    assert.equal(source.split(before).length - 1,1,
      'Instrumentation anchor: '+before);
    source = source.replace(before,after);
  }

  once(
    '  const state = new BattleState(preview);',
    '  const state = new BattleState(preview);\n' +
    '  state.__v32Evidence = {steps:[], segments:[]};'
  );

  once(
    '      lines.push(...state.switchIn(sw.side, mon));',
    '      __v32Capture(state, lines, {kind:"initial_sendout",\n' +
    '        segment:segments.indexOf(leadSeg), converterTurn:0,\n' +
    '        paragraph:leadSeg.paras.indexOf(para), line:0,\n' +
    '        text:para.text, align:para.align, ownerHint:para.alt, html:para.raw},\n' +
    '        () => { lines.push(...state.switchIn(sw.side, mon)); return true; });'
  );

  once(
    'function processSegment(seg, state, out, hooks) {',
    'function processSegment(seg, state, out, hooks) {\n' +
    '  const __segment = {index:hooks.segmentIndex, converterTurn:hooks.turn,\n' +
    '    initialOutput:out.slice(), stepStart:state.__v32Evidence.steps.length};\n' +
    '  state.__v32Evidence.segments.push(__segment);'
  );

  once(
    '    for (const line of textLines) processTextLine(line, para, pi);',
    '    for (let li = 0; li < textLines.length; li++) {\n' +
    '      const line = textLines[li];\n' +
    '      __v32Capture(state, out, {kind:"text_line",\n' +
    '        segment:hooks.segmentIndex, converterTurn:hooks.turn,\n' +
    '        paragraph:pi, line:li, text:line, align:para.align,\n' +
    '        ownerHint:para.alt, html:para.raw},\n' +
    '        () => processTextLine(line, para, pi));\n' +
    '    }'
  );

  once(
    '  auditSegmentStatCoverage();',
    '  auditSegmentStatCoverage();\n' +
    '  __segment.stepEnd = state.__v32Evidence.steps.length;\n' +
    '  __segment.finalOutput = out.slice();'
  );

  once(
    'return {battleId, stem, raw: lines.join',
    'return {evidence:state.__v32Evidence, finalState:__v32Snapshot(state),\n' +
    '    preview, battleId, stem, raw: lines.join'
  );

  source += '\n' + [__v32Snapshot,__v32StateDiff,__v32Capture]
    .map(fn => fn.toString()).join('\n') +
    '\nmodule.exports = {parseBattle,loadJaDict};\n';

  const mod = new Module(referencePath,module);
  mod.filename = referencePath;
  mod.paths = Module._nodeModulePaths(path.dirname(referencePath));
  mod._compile(source,referencePath);

  assert.equal(mod.exports.loadJaDict(dictionaryPath),dictionaryPath);

  const options = {
    battlePrefix:'battle-gen2nintendocup2000noohkostadium2strict-pbs',
    missingTimestampPolicy:'synthetic',
    includeIncomplete:true,
    allowNoDict:true
  };

  function parseFile(filename) {
    const result = mod.exports.parseBattle(filename,options,'');
    let checkedSegments = 0;

    for (const segment of result.evidence.segments) {
      const restored = segment.initialOutput.slice();

      for (const step of result.evidence.steps.slice(
        segment.stepStart,segment.stepEnd
      )) {
        assert.equal(step.segment,segment.index,
          'Evidence segment mismatch');

        const patch = step.protocolPatch;

        assert.deepEqual(
          restored.slice(patch.index,patch.index+patch.removed.length),
          patch.removed,
          'Evidence removed-line mismatch'
        );

        restored.splice(
          patch.index,patch.removed.length,...patch.inserted
        );
      }

      assert.deepEqual(restored,segment.finalOutput,
        'Evidence replay mismatch');
      checkedSegments++;
    }

    return {
      ...result,
      schema:'pbs-v32-source-evidence-v1',
      referenceSha256:REFERENCE_SHA,
      inputSha256:hash(fs.readFileSync(filename)),
      protocol:result.raw.trimEnd().split(/\r?\n/),
      checkedSegments,
      interpretationNotes:{
        converterTurnIsDecisionTurn:false,
        converterUpkeepIsLivePhaseProof:false,
        converterMaxHpIsAuthoritative:false,
        finalStateDefaultsAreAbsenceProof:false,
        livePositionConnected:false
      }
    };
  }

  return {parseFile};
}

module.exports = {createReader,REFERENCE_SHA};
