'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {createObservationLedger, REFERENCE_SHA} =
  require('./pbs-castle-observation-ledger');

const root = path.resolve(__dirname, '..');
const stopId = '2026-11-20T16-29-14-394Z-K-g1';
const hashes = {
  rawTurnHtml: 'a'.repeat(64),
  rawPdatHtml: 'b'.repeat(64),
  rawLlogHtml: 'c'.repeat(64)
};

const p1 = [
  'cloyster','snorlax','marowak','starmie','gengar','zapdos'
];
const p2 = [
  'mrmime','charizard','tauros','smeargle','snorlax','quagsire'
];
const previewSides = [p1,p2].map(team =>
  team.map(speciesId => ({speciesId, level: 50})));

function built() {
  return {
    activeSlots: [0,0],
    ui: {
      battleTurn: {
        sides: [
          {speciesId: 'cloyster', level: 50, hp: 100, maxHp: 100},
          {speciesId: 'mrmime', level: 50, hp: 74, maxHp: 146}
        ]
      }
    }
  };
}

function evidence(hp, withFaint = true) {
  const commands = [
    {
      line: '|switch|p2a: Smeargle|Smeargle, L50, F|100/100',
      changed: false
    },
    {
      line: '|-damage|p2a: Smeargle|' +
        (hp === 0 ? '0 fnt' : hp + '/161'),
      changed: true
    },
    ...(withFaint ? [{
      line: '|faint|p2a: Smeargle',
      changed: false
    }] : [])
  ];

  const protocol = ['|', ...commands.map(x => x.line)];
  const steps = commands.map((row,i) => ({
    kind: 'text_line',
    segment: 0,
    paragraph: 0,
    line: i,
    text: 'event-' + i,
    align: 'right',
    ownerHint: 'foe',
    acknowledged: true,
    hpChanges: row.changed
      ? [{side: 'p2', species: 'Smeargle', after: hp}]
      : [],
    protocolPatch: {
      index: i + 1,
      inserted: [row.line],
      removed: []
    }
  }));

  return {
    referenceSha256: REFERENCE_SHA,
    errors: [],
    unknown: [],
    rejectReason: '',
    protocol,
    raw: protocol.join('\n') + '\n',
    evidence: {
      steps,
      segments: [{
        index: 0,
        stepStart: 0,
        stepEnd: steps.length,
        initialOutput: ['|'],
        finalOutput: protocol
      }]
    },
    finalState: {
      monState: {
        p2: {
          kind: 'Map',
          entries: [['Smeargle',{hpAbs: hp}]]
        }
      }
    }
  };
}

test('confirmed faint does not require observed max HP', () => {
  const ledger = createObservationLedger({
    mySide: 0, previewSides
  });
  const result = ledger.stage({
    evidence: evidence(0,true),
    built: built(),
    label: 'unit-faint',
    rawHashes: hashes
  });
  assert.equal(result.sourceReport.missingMaxHp.length,0);
  assert.equal(result.sourceReport.faintedWithoutMaxHp.length,1);
  assert.equal(result.sourceReport.faintedWithoutMaxHp[0].species,'smeargle');
  assert.equal(
    result.sourceReport.inputBoundaries.filter(x=>x.blocksReady).length,0
  );
  assert.equal(
    result.observations.some(x=>x.species==='smeargle'),false
  );
  assert.equal(result.commit(),1);
});

test('zero HP without verified faint is not silently accepted', () => {
  const ledger = createObservationLedger({
    mySide: 0, previewSides
  });
  const result = ledger.stage({
    evidence: evidence(0,false),
    built: built(),
    label: 'unit-no-faint',
    rawHashes: hashes
  });
  assert.equal(result.sourceReport.missingMaxHp.length,1);
  assert.equal(
    result.sourceReport.inputBoundaries.some(x=>x.blocksReady),true
  );
  assert.throws(
    ()=>result.commit(),
    /unresolved source-input boundary/
  );
});

test('unknown positive HP remains a separate issue', () => {
  const ledger = createObservationLedger({
    mySide: 0, previewSides
  });
  const result = ledger.stage({
    evidence: evidence(88,false),
    built: built(),
    label: 'unit-living',
    rawHashes: hashes
  });
  assert.equal(result.sourceReport.missingMaxHp.length,1);
  assert.equal(result.sourceReport.faintedWithoutMaxHp.length,0);
});

test('real stopped match resumes offline through turn nine', () => {
  const traceFile = path.join(
    root,'data/pbs-traces',stopId+'.jsonl'
  );
  assert.ok(fs.existsSync(traceFile),'REAL_TRACE_REQUIRED:'+traceFile);

  const events = fs.readFileSync(traceFile,'utf8')
    .split(/\r?\n/).filter(Boolean).map(JSON.parse);

  const start = events.find(x=>x.type==='match_start');
  const preview = events.find(x=>x.type==='preview');
  const decisions = events.filter(x=>x.type==='decision');
  const stopped = events.find(x=>x.type==='observation_stopped');

  assert.ok(
    start && preview && stopped && decisions.length===8,
    'REAL_TRACE_INCOMPLETE'
  );
  assert.match(stopped.message,/PUBLIC_MAXIMUM_HP_NOT_OBSERVED/);

  const client = require('./pbs-client');
  const wasm = require(
    path.join(root,'crates/wasm/pkg-node/nc2000_wasm.js')
  );
  const dex = new wasm.Dex();
  const dexData = JSON.parse(
    fs.readFileSync(path.join(root,'data/gen2stadium2.json'),'utf8')
  );
  const poolJson = fs.readFileSync(
    path.join(root,'data/belief-pool-v3/belief-pool.json'),'utf8'
  );
  const sides = [
    preview.parsedPreview.p1Mons,
    preview.parsedPreview.p2Mons
  ];
  const side = preview.mySide;
  assert.ok(side===0 || side===1);

  const workBase = path.join(root,'pbs-work');
  fs.mkdirSync(workBase,{recursive:true});
  const workDir = fs.mkdtempSync(
    path.join(workBase,'immediate-faint-regression-')
  );

  let passed = false;
  try {
    const session = require('./pbs-castle-observation')
      .createObservationSession({
        repoRoot: root,
        workDir,
        client,
        wasm,
        dex,
        dexData,
        side,
        matchId: stopId,
        ownSets: start.team.sets,
        previewSides: sides,
        pickedRosterSlots: preview.pickedRosterSlots,
        seed: decisions[0].seed,
        c: decisions[0].c,
        poolJson,
        poolProvider: ({seenFoeMons,foeSide,spec}) =>
          client.buildDynamicBeliefPoolJson({
            foePreviewMons: sides[foeSide],
            foeName: preview.foeName,
            seenFoeMons,
            opponentDb: client.loadOpponentDb(),
            positionSpec: spec
          })
      });

    for (const [index,event] of [...decisions,stopped].entries()) {
      let prepared;
      try {
        prepared = session.prepare({
          rawTurnHtml: event.rawTurnHtml,
          rawPdatHtml: event.rawPdatHtml,
          rawLlogHtml: event.rawLlogHtml
        });

        assert.equal(
          prepared.status,'READY',
          'OFFLINE_PREPARE_FAILED_AT_'+index
        );

        if (index !== decisions.length) continue;

        const confirmed =
          prepared.report.sourceReport.faintedWithoutMaxHp;

        assert.ok(
          confirmed.some(x=>x.species==='smeargle' && x.hp===0),
          'SMEARGLE_NOT_CONFIRMED_FAINT'
        );

        const state = JSON.parse(prepared.searcher.stateView());
        const party = state.sides[1-side].party;
        const found = party.filter(x=>
          String(x.species).toLowerCase()
            .replace(/[^a-z0-9]/g,'')==='smeargle'
        );

        assert.equal(found.length,1,'SMEARGLE_IDENTITY');
        assert.equal(found[0].hp,0,'SMEARGLE_HP');
        assert.equal(found[0].fainted,true,'SMEARGLE_FAINT');

        const legal = prepared.built.expectedNativeInputs;
        assert.ok(legal.length>0,'NO_LEGAL_ACTIONS');

        const selection = client.chooseObservedAction(
          prepared,
          {isRandom:true,iterations:1,chooseIndex:()=>0}
        );

        assert.ok(
          legal.includes(selection.bestChoice),
          'CHOICE_NOT_LEGAL'
        );

        const options = prepared.built.ui.battleTurn.options;
        assert.equal(
          options.filter(x=>x.value===selection.mapping.value).length,
          1,
          'CASTLE_CHOICE_NOT_UNIQUE'
        );
        assert.equal(
          prepared.built.ui.actionType,'INPUT',
          'STOP_SCENE_CHANGED'
        );

        console.log('REAL_TRACE_TURN_9_READY=true');
        console.log('REAL_TRACE_FAINTED_SMEARGLE=true');
        console.log('REAL_TRACE_LEGAL_ACTION='+selection.mapping.value);
      } catch (error) {
        console.error('REAL_TRACE_FAILURE_AT_INDEX='+index);
        throw error;
      } finally {
        if (prepared?.searcher) prepared.searcher.free();
      }
    }
    passed = true;
  } finally {
    if (passed) {
      fs.rmSync(workDir,{recursive:true,force:true});
    } else {
      console.error('REAL_TRACE_EVIDENCE_DIR='+workDir);
    }
  }
});
