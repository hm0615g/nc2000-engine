'use strict';
const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');
const {isDeepStrictEqual} = require('node:util');
const assert = require('node:assert/strict');
const cp = require('node:child_process');

const flags = new Set(process.argv.slice(2));
assert.ok(
  [...flags].every(x => ['--audit','--apply','--verify-installed'].includes(x)),
  'Usage: --audit, --apply, or --verify-installed'
);
assert.ok(!(flags.has('--apply') && flags.has('--verify-installed')));

const apply = flags.has('--apply');
const verifyInstalled = flags.has('--verify-installed');
const sourcePath = path.resolve('tools/pbs-client.js');
const raw = fs.readFileSync(sourcePath,'utf8');
let src = raw;
let patches = 0;

function fix(oldText,newText,label) {
  const count = src.split(oldText).length - 1;
  assert.equal(count,1,'patch anchor '+label);
  src = src.replace(oldText,newText);
  patches++;
  console.log('PATCH='+label+':PASS');
}

if (!verifyInstalled) {
fix('  if (!html) return info;\n',
`  if (!html) return info;
  const pendingCalledMove = [null,null];
  const pendingTransform = [null,null];
`, 'pending_events');

fix('    const targetMon = side.mons[slot];\n\n    // If already active',
`    const targetMon = side.mons[slot];
    if (side.activeSlot !== null && side.activeSlot !== slot) {
      pendingCalledMove[sideIdx] = null;
      pendingTransform[sideIdx] = null;
    }

    // If already active`, 'switch_boundary');

fix(
'        if (nextTurn > info.turn) {\n          advanceLlogTrackerToTurn(info, nextTurn);',
`        if (nextTurn > info.turn) {
          pendingCalledMove[0] = pendingCalledMove[1] = null;
          pendingTransform[0] = pendingTransform[1] = null;
          advanceLlogTrackerToTurn(info, nextTurn);`,
'hr_boundary');

fix(
'    if (blockTurn > info.turn) {\n      advanceLlogTrackerToTurn(info, blockTurn);',
`    if (blockTurn > info.turn) {
      pendingCalledMove[0] = pendingCalledMove[1] = null;
      pendingTransform[0] = pendingTransform[1] = null;
      advanceLlogTrackerToTurn(info, blockTurn);`,
'block_boundary');

fix(
`      const isCalledMove = mIdx > 0 && (
        callerMoveId === 'sleeptalk' ||
        callerMoveId === 'mirrormove' ||
        callerMoveId === 'metronome'
      );`,
`      const previousCaller = pendingCalledMove[sideIdx];
      const inheritedCall = mIdx === 0 && previousCaller &&
        previousCaller.turn === turnNum &&
        previousCaller.slot === info.sides[sideIdx].activeSlot;
      const isCalledMove = Boolean(inheritedCall) || (mIdx > 0 && (
        callerMoveId === 'sleeptalk' ||
        callerMoveId === 'mirrormove' ||
        callerMoveId === 'metronome'
      ));`,
'cross_paragraph_call');

fix(
`      const actorMon = actorSlot !== null ? info.sides[sideIdx].mons[actorSlot] : null;
      const foeSlot`,
`      const actorMon = actorSlot !== null ? info.sides[sideIdx].mons[actorSlot] : null;
      const releasingCharge = Boolean(actorMon &&
        actorMon.charging === mvId &&
        TWO_TURN_CHARGE_MOVES.has(mvId));
      if ((mvId === 'sleeptalk' ||
           mvId === 'mirrormove' ||
           mvId === 'metronome') &&
          !moveFailed && actorSlot !== null) {
        pendingCalledMove[sideIdx] = {
          turn: turnNum, slot: actorSlot, move: mvId
        };
      } else if (isCalledMove) {
        pendingCalledMove[sideIdx] = null;
      }
      const foeSlot`,
'record_caller_and_charge');

fix(
`        const isForeignCall = isCalledMove && callerMoveId !== 'sleeptalk';`,
`        const effectiveCallerMoveId =
          inheritedCall ? previousCaller.move : callerMoveId;
        const isForeignCall =
          isCalledMove && effectiveCallerMoveId !== 'sleeptalk';`,
'called_move_provenance');

fix(
`        if (TWO_TURN_CHARGE_MOVES.has(mvId) && !moveFailed) {`,
`        if (TWO_TURN_CHARGE_MOVES.has(mvId) &&
            !moveFailed && !releasingCharge) {`,
'charge_release');

fix(
`          } else if (mvId === 'transform' && foeSlot !== null && foeMon) {
            actorMon.transformedInto = [foeIdx, foeSlot];
            actorMon.boosts = [...foeMon.boosts];`,
`          } else if (mvId === 'transform' && foeSlot !== null && foeMon) {
            pendingTransform[sideIdx] = {
              turn: turnNum,
              actorSlot,
              foeSide: foeIdx,
              foeSlot
            };`,
'transform_defer');

fix(
`    // 3b. Phaze drag`,
`    const awaitingTransform = pendingTransform[sideIdx];
    if (awaitingTransform) {
      if (isMoveFailureText(clean)) {
        pendingTransform[sideIdx] = null;
      } else if (/(?:\\u3078\\u3093\\u3057\\u3093\\u3057\\u305f|\\u5909\\u8eab\\u3057\\u305f)/.test(clean)) {
        const srcMon =
          info.sides[sideIdx].mons[awaitingTransform.actorSlot];
        const targetMon =
          info.sides[awaitingTransform.foeSide].mons[awaitingTransform.foeSlot];
        if (srcMon && targetMon &&
            info.sides[sideIdx].activeSlot === awaitingTransform.actorSlot) {
          srcMon.transformedInto = [
            awaitingTransform.foeSide,
            awaitingTransform.foeSlot
          ];
          srcMon.boosts = [...targetMon.boosts];
        }
        pendingTransform[sideIdx] = null;
      }
    }
    if (blockMoveIds.length === 0 && isMoveFailureText(clean)) {
      pendingCalledMove[sideIdx] = null;
    }
    // 3b. Phaze drag`,
'transform_resolution');

fix(
`  return info;
}

/**
 * Convert a \`parseLlogHtml\` tracker`,
`  for (const side of [0,1]) {
    if (pendingTransform[side]) {
      info.unrecognizedLines.push({
        turn: info.turn,
        side,
        align: side === 0 ? 'left' : 'right',
        line: 'Transform outcome not yet confirmed by LLOG'
      });
    }
  }
  return info;
}

/**
 * Convert a \`parseLlogHtml\` tracker`,
'unconfirmed_transform_flag');
}

const m = new Module(sourcePath,module);
m.filename = sourcePath;
m.paths = Module._nodeModulePaths(path.dirname(sourcePath));
if (!verifyInstalled) m._compile(src,sourcePath);

console.log(verifyInstalled
  ? 'INSTALLED_MODULE=LOADED'
  : 'IN_MEMORY_COMPILE=PASS');

const oldApi = require(sourcePath);
const api = verifyInstalled ? oldApi : m.exports;

const RUN = '/home/hm/pbs-selfplay-runs/g1-1794173773869-47088a';
const ja = require(path.resolve('data/i18n-ja.json'));
const id = x =>
  String(x || '').toLowerCase().replace(/[^a-z0-9]/g,'');

function decisions(side) {
  const dir = path.join(RUN,side,'traces');
  const files = fs.readdirSync(dir)
    .filter(x=>x.endsWith('.jsonl'));
  assert.equal(files.length,1);
  return fs.readFileSync(path.join(dir,files[0]),'utf8')
    .split(/\r?\n/)
    .filter(Boolean)
    .map(JSON.parse)
    .filter(x=>x.type==='decision');
}

const p1 = decisions('p1');
const p2 = decisions('p2');
assert.equal(p1.length,7);
assert.equal(p2.length,10);

const rosterFor = e => e.spec.sides.map(s =>
  s.mons.map(x=>({
    species:x.species,
    speciesId:id(x.species)
  }))
);

const roster = rosterFor(p1[0]);

function slot(side,species) {
  const i = roster[side].findIndex(
    x=>x.speciesId===species
  );
  assert.ok(i>=0 && ja.species[species]);
  return i;
}

const a = slot(0,'marowak');
const b = slot(1,'smeargle');
const c = slot(1,'electrode');

const name = (side,index) =>
  ja.species[roster[side][index].speciesId];

const p = (side,text) =>
  '<P ALIGN="'+(side?'right':'left')+'">'+text+'</P>';

const send = (side,index) =>
  p(side,
    '<B>'+name(side,index)+
    '</B>\u3092\u7e70\u308a\u51fa\u3057\u305f\uff01'
  );

function move(side,index,key) {
  assert.ok(ja.moves[key],key);
  return '<B>'+name(side,index)+
    '</B>\u306e<B>'+ja.moves[key]+'</B>\uff01';
}

const init = '<HR>'+send(0,a)+send(1,b)+'<HR>';
const parse = text => api.parseLlogHtml(init+text,roster);
const actor = r => r.sides[1].mons[b];
const uses = (r,key) => actor(r).uses[key] || 0;
const results = [];

function check(label,actual,expected) {
  results.push({
    label,
    actual,
    expected,
    pass:isDeepStrictEqual(actual,expected)
  });
}

console.log('=== ALL MECHANISM CASES ===');

for(const caller of ['metronome','sleeptalk','mirrormove']) {
  for(const mode of ['same','separate']) {
    const body = mode==='same'
      ? p(1,move(1,b,caller)+'<BR>'+
          move(1,b,'thunderwave'))
      : p(1,move(1,b,caller))+
        p(1,move(1,b,'thunderwave'));

    const r = parse(body);
    const events = r.events.filter(
      x=>x.type==='move' && x.side===1
    );

    check('CALL_'+caller+'_'+mode+'_PP',
      [uses(r,caller),uses(r,'thunderwave')],
      [1,0]);

    check('CALL_'+caller+'_'+mode+'_EVENTS',
      events.map(x=>x.called),
      [false,true]);

    check('CALL_'+caller+'_'+mode+'_REVEAL',
      actor(r).revealedMoves.has('thunderwave'),
      caller==='sleeptalk');
  }

  const nextTurn = parse(
    p(1,move(1,b,caller))+
    '<HR>'+
    p(1,move(1,b,'thunderwave'))
  );

  check('CALL_'+caller+'_NEXT_TURN_PP',
    uses(nextTurn,'thunderwave'),1);
}

const up = p(0,
  name(0,a)+
  '\u306e\u3053\u3046\u3052\u304d\u304c'+
  '\u3050\uff5e\u3093\u3068'+
  '\u3042\u304c\u3063\u305f\uff01'
);

const successText =
  name(1,b)+'\u306f'+name(0,a)+
  '\u306b\u3078\u3093\u3057\u3093\u3057\u305f\uff01';

const failText =
  '\u3057\u304b\u3057\u3001\u3046\u307e\u304f'+
  '\u304d\u307e\u3089\u306a\u304b\u3063\u305f\u3002';

for(const mode of ['same','separate']) {
  const success = up+(
    mode==='same'
      ? p(1,move(1,b,'transform')+'<BR>'+successText)
      : p(1,move(1,b,'transform'))+
        p(1,successText)
  );

  const failure = mode==='same'
    ? p(1,move(1,b,'transform')+'<BR>'+failText)
    : p(1,move(1,b,'transform'))+
      p(1,failText);

  const r = parse(success);

  check('TRANSFORM_'+mode+'_SUCCESS',
    [
      actor(r).transformedInto,
      actor(r).boosts[0],
      uses(r,'transform')
    ],
    [[0,a],2,1]);

  check('TRANSFORM_'+mode+'_RESET',
    parse(success+send(1,c))
      .sides[1].mons[b].transformedInto,
    null);

  check('TRANSFORM_'+mode+'_FAIL',
    actor(parse(failure)).transformedInto,
    null);
}

const unresolved = parse(
  p(1,move(1,b,'transform'))
);

check('TRANSFORM_UNCONFIRMED',
  actor(unresolved).transformedInto,null);

check('TRANSFORM_DIAGNOSTIC',
  unresolved.unrecognizedLines.some(
    x=>String(x.line).includes('not yet confirmed')
  ),
  true);

const flyStart = p(1,
  move(1,b,'fly')+'<BR>'+
  name(1,b)+
  '\u306f\u7a7a\u9ad8\u304f'+
  '\u98db\u3073\u4e0a\u304c\u3063\u305f\uff01'
);

for(const mode of ['same','separate']) {
  const damage =
    name(0,a)+
    '\u306f\u30c0\u30e1\u30fc\u30b8'+
    '\u3092\u53d7\u3051\u305f\uff01';

  const release = mode==='same'
    ? p(1,move(1,b,'fly')+'<BR>'+damage)
    : p(1,move(1,b,'fly'))+p(0,damage);

  const before = parse(flyStart);
  const after = parse(
    flyStart+'<HR>'+release
  );

  check('TWO_TURN_'+mode+'_CHARGE',
    actor(before).charging,'fly');

  check('TWO_TURN_'+mode+'_RELEASE',
    [actor(after).charging,uses(after,'fly')],
    [null,1]);
}

const rollout = p(1,move(1,b,'rollout'));
const rr = parse(
  rollout+'<HR>'+rollout+'<HR>'+rollout
);

check('LOCKED_ROLLOUT',
  [
    uses(rr,'rollout'),
    actor(rr).locked && actor(rr).locked.n
  ],
  [1,3]);

console.log('=== NEGATIVE CONTROL CASES ===');

for(const caller of ['metronome','sleeptalk','mirrormove']) {
  for(const mode of ['same','separate']) {
    const failedMessage =
      '\u3057\u304b\u3057\u3001\u3046\u307e\u304f'+
      '\u304d\u307e\u3089\u306a\u304b\u3063\u305f\u3002';

    const failed = mode==='same'
      ? p(1,move(1,b,caller)+'<BR>'+failedMessage)
      : p(1,move(1,b,caller))+p(1,failedMessage);

    const r = parse(
      failed+p(1,move(1,b,'thunderwave'))
    );

    const last = r.events.filter(
      x=>x.type==='move' && x.side===1
    ).at(-1);

    check('NEG_'+caller+'_'+mode+'_FAILED_CALL',
      [uses(r,'thunderwave'),last && last.called],
      [1,false]);
  }

  const interleaved = parse(
    p(1,move(1,b,caller))+
    p(0,move(0,a,'earthquake'))+
    p(1,move(1,b,'thunderwave'))
  );

  check('NEG_'+caller+'_SIDE_ISOLATION',
    [
      uses(interleaved,'thunderwave'),
      interleaved.sides[0].mons[a].uses.earthquake
    ],
    [0,1]);

  const switched = parse(
    p(1,move(1,b,caller))+
    send(1,c)+
    p(1,move(1,c,'thunderwave'))
  );

  const last = switched.events.filter(
    x=>x.type==='move' && x.side===1
  ).at(-1);

  check('NEG_'+caller+'_SWITCH_CANCEL',
    [
      switched.sides[1].mons[c].uses.thunderwave || 0,
      last && last.called
    ],
    [1,false]);
}

const wrongText =
  name(0,a)+'\u306f'+name(1,b)+
  '\u306b\u3078\u3093\u3057\u3097\u305f\uff01';

const wrongSide = parse(
  p(1,move(1,b,'transform'))+
  p(0,wrongText)
);

check('NEG_TRANSFORM_WRONG_SIDE',
  actor(wrongSide).transformedInto,null);

const lateTransform = parse(
  p(1,move(1,b,'transform'))+
  '<HR>'+
  p(1,
    name(1,b)+'\u306f'+name(0,a)+
    '\u306b\u3078\u3093\u3057\u3093\u3057\u305f\uff01'
  )
);

check('NEG_TRANSFORM_NEXT_TURN',
  actor(lateTransform).transformedInto,null);

const switchedFly = parse(flyStart+send(1,c));

check('NEG_CHARGE_SWITCH_RESET',
  switchedFly.sides[1].mons[b].charging,
  null);

console.log('=== ALL SAVED SNAPSHOT PARITY ===');

let realDiff = 0;

for(const [side,events] of [['p1',p1],['p2',p2]]) {
  for(let i=0;i<events.length;i++) {
    const e = events[i];
    const r = rosterFor(e);

    const before = oldApi.serializeLlogTracker(
      oldApi.parseLlogHtml(e.rawLlogHtml,r)
    );

    const after = api.serializeLlogTracker(
      api.parseLlogHtml(e.rawLlogHtml,r)
    );

    if(!isDeepStrictEqual(before,after)) {
      realDiff++;
      console.log('REAL_DIFF='+side+'_D'+(i+1));
    }
  }
}

const failures = results.filter(x=>!x.pass);

console.log('=== RESULT ===');

for(const category of [
  'CALL','TRANSFORM','TWO_TURN','LOCKED'
]) {
  const subset = results.filter(
    x=>x.label.startsWith(category+'_')
  );

  console.log(category+'_ASSERTIONS='+subset.length);
  console.log(category+'_FAILURES='+
    subset.filter(x=>!x.pass).length);
}

console.log('TEST_ASSERTIONS='+results.length);
console.log('TEST_FAILURES='+failures.length);
console.log('NEGATIVE_ASSERTIONS='+
  results.filter(x=>x.label.startsWith('NEG_')).length);
console.log('SAVED_REAL_SNAPSHOTS='+(p1.length+p2.length));
console.log('REAL_SNAPSHOT_PARITY_DIFFERENCES='+realDiff);
console.log('PATCH_COUNT='+patches);

for(const f of failures) {
  console.log('FAIL='+f.label+
    ':expected='+JSON.stringify(f.expected)+
    ':actual='+JSON.stringify(f.actual));
}

const passed = failures.length===0 && realDiff===0;

console.log('RESULT='+
  (passed?'STAGED_CANDIDATE_PASS':'INCOMPLETE'));

console.log('MODE='+
  (apply?'APPLY_AFTER_PASS':
   verifyInstalled?'VERIFY_INSTALLED':'AUDIT_ONLY'));

console.log('PBS_CONNECTION=NOT_ATTEMPTED');
console.log('BATTLE_TEST=NOT_STARTED');

if(!passed) {
  process.exitCode=1;
} else if(apply) {
  const branch = cp.execFileSync(
    'git',['branch','--show-current'],
    {encoding:'utf8'}
  ).trim();

  assert.equal(branch,'review-local-changes');

  const ps = cp.execFileSync(
    'ps',['-eo','args='],
    {encoding:'utf8'}
  );

  if(/pbs-client\.js|pbs-selfplay-runner\.js|pbs-selfplay-controller\.js/.test(ps)) {
    throw new Error('ACTIVE_PBS_PROCESS');
  }

  assert.equal(
    fs.readFileSync(sourcePath,'utf8'),raw
  );

  const regressionPath = path.resolve(
    'tools/pbs-event-hardened-regression.js'
  );

  assert.ok(!fs.existsSync(regressionPath));

  const diagnostics = path.join(RUN,'diagnostics');
  fs.mkdirSync(diagnostics,{recursive:true});

  const backup = path.join(
    diagnostics,
    'pbs-client-pre-event-fix-'+
    new Date().toISOString().replace(/[:.]/g,'-')+'.js'
  );

  fs.copyFileSync(
    sourcePath,backup,fs.constants.COPYFILE_EXCL
  );

  const mode = fs.statSync(sourcePath).mode;
  const temp = sourcePath+'.'+process.pid+'.stage';
  let installed = false;

  try {
    fs.writeFileSync(temp,src,{flag:'wx'});
    fs.chmodSync(temp,mode);
    fs.renameSync(temp,sourcePath);
    installed = true;

    fs.copyFileSync(
      __filename,
      regressionPath,
      fs.constants.COPYFILE_EXCL
    );

    cp.execFileSync(
      process.execPath,['--check',sourcePath],
      {stdio:'inherit'}
    );

    cp.execFileSync(
      process.execPath,
      [regressionPath,'--verify-installed'],
      {stdio:'inherit'}
    );

    cp.execFileSync(
      process.execPath,
      ['--test','tools/pbs-batonn-regression.test.js'],
      {stdio:'inherit'}
    );

    console.log('LOCAL_P1_EVENT_FIX=PASS');
    console.log('BACKUP='+backup);
    console.log('P2_UNCHANGED=YES');
  } catch(error) {
    if(installed) {
      fs.copyFileSync(backup,sourcePath);
      if(fs.existsSync(regressionPath)) {
        fs.unlinkSync(regressionPath);
      }
      console.error('ROLLBACK=RESTORED_ORIGINAL');
    }
    throw error;
  } finally {
    if(fs.existsSync(temp)) fs.unlinkSync(temp);
  }
}
