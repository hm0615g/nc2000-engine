'use strict';
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');

const HASH =
  '79dff80a4689bdb3869fa8649887f5108e4ebf6a30ff146b68d9fa2c69540e4e';
const CLIENT_HASH =
  '8a74c713d5564bb933ddb4f30c8b0cf8c401948fc8fde21a31b9b8c99382722a';
const sha = x => crypto.createHash('sha256').update(x).digest('hex');

function attrs(tag) {
  const out = {};
  const re = /([a-zA-Z_:][\w:.-]*)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;
  for (const m of tag.matchAll(re)) {
    const key = m[1].toLowerCase();
    assert.ok(!Object.hasOwn(out,key),'Duplicate attribute: '+key);
    out[key] = m[2] ?? m[3] ?? m[4] ?? '';
  }
  return out;
}

function readDecisionForm(html) {
  assert.equal(typeof html,'string','Raw decision HTML is required');
  const choices = [];

  for (const f of html.matchAll(
    /<form\b([^>]*)>([\s\S]*?)<\/form\s*>/gi
  )) {
    const fields = new Map();

    for (const input of f[2].matchAll(/<input\b([^>]*)>/gi)) {
      const a = attrs(input[1]);
      const key = (a.name || '').toUpperCase();
      if (!['ACTION','NOW'].includes(key)) continue;
      assert.ok(!fields.has(key),'Duplicate form field: '+key);
      fields.set(key,a.value ?? '');
    }

    const action = (fields.get('ACTION') || '').toUpperCase();
    if (!['INPUT','BATONN','CHANGE'].includes(action)) continue;

    assert.equal(
      (attrs(f[1]).method || '').toLowerCase(),
      'post',
      'Form method'
    );
    assert.ok(fields.has('NOW'),'NOW missing from decision form');

    const now = fields.get('NOW').trim();
    assert.ok(now === '' || /^\d+$/.test(now),'Invalid NOW');

    const completed = now === '' ? 0 : Number(now);
    assert.ok(
      Number.isSafeInteger(completed) && completed < 1000000,
      'NOW range'
    );

    choices.push({action, now, decisionTurn:completed+1});
  }

  assert.equal(
    choices.length,1,
    'Exactly one supported decision form required'
  );
  return choices[0];
}

function alignDecisionProtocol(evidence, rawTurnHtml, side) {
  assert.ok(side === 0 || side === 1,'Player side');
  assert.equal(evidence.referenceSha256,HASH,'Reference provenance');
  assert.equal(
    evidence.complete,false,
    'A finished battle is not a decision'
  );
  assert.deepEqual(evidence.errors,[],'Converter errors');
  assert.deepEqual(evidence.unknown,[],'Unknown source messages');
  assert.equal(evidence.rejectReason,'','Converter rejected source');

  const form = readDecisionForm(rawTurnHtml);
  const original = evidence.protocol;
  assert.ok(Array.isArray(original) && original.length,'Protocol missing');

  assert.deepEqual(
    original,
    evidence.raw.trimEnd().split(/\r?\n/),
    'Raw/protocol mismatch'
  );

  const markers = original.flatMap((line,index) => {
    const m = line.match(/^\|turn\|(\d+)$/);
    return m ? [{turn:Number(m[1]),index}] : [];
  });

  assert.ok(markers.length,'No turn marker');
  const last = markers.at(-1);
  const lines = original.slice();
  const edits = [];

  const result = {
    form,
    converterTurn:last.turn,
    ready:false,
    reason:null,
    edits,
    protocol:null,
    sourceRawSha256:sha(evidence.raw)
  };

  function requireGeneratedTail() {
    const segment = evidence.evidence.segments.filter(
      x => x.finalOutput.length > 2
    ).at(-1);

    assert.ok(segment,'No source segment for final upkeep');
    assert.equal(
      original.at(-1),'|upkeep',
      'No terminal converter upkeep'
    );

    const start = original.length - 1 - segment.finalOutput.length;
    assert.ok(start >= 0,'Invalid source segment length');

    assert.deepEqual(
      original.slice(start,-1),
      segment.finalOutput,
      'Terminal upkeep is not proven to follow the source segment'
    );

    return segment.index;
  }

  if (form.action === 'INPUT') {
    if (last.turn + 1 === form.decisionTurn) {
      const segment = requireGeneratedTail();
      const line = '|turn|' + form.decisionTurn;

      edits.push({
        operation:'append',
        index:lines.length,
        line,
        basis:'INPUT_FORM_NOW',
        sourceSegment:segment
      });

      lines.push(line);
    } else {
      assert.equal(
        last.turn,form.decisionTurn,
        'INPUT turn discontinuity'
      );
      assert.ok(
        lines.slice(last.index+1).every(
          x => x === '|' || x.startsWith('|t:|')
        ),
        'Already-current turn contains actions; do not guess its phase'
      );
    }
  } else if (form.action === 'BATONN') {
    assert.equal(
      last.turn,form.decisionTurn,
      'BATONN turn mismatch'
    );

    const segment = requireGeneratedTail();
    const moves = lines.slice(last.index+1,-1)
      .filter(x => x.startsWith('|move|'));
    const move = (moves.at(-1) || '').split('|');

    assert.ok(
      (move[2] || '').startsWith('p'+(side+1)+'a:'),
      'Baton actor mismatch'
    );
    assert.equal(
      (move[3] || '').toLowerCase().replace(/[^a-z]/g,''),
      'batonpass'
    );

    edits.push({
      operation:'remove',
      index:lines.length-1,
      line:'|upkeep',
      basis:'BATONN_FORM_MID_TURN',
      sourceSegment:segment
    });

    lines.pop();
  } else {
    assert.equal(
      last.turn,form.decisionTurn,
      'CHANGE turn mismatch'
    );
    result.reason = 'CHANGE_RESIDUAL_PHASE_NOT_ESTABLISHED';
    return result;
  }

  const gameplay = xs => xs.filter(
    x => !/^\|(?:turn|upkeep)(?:\||$)/.test(x)
  );

  assert.deepEqual(
    gameplay(lines),gameplay(original),
    'Battle events changed'
  );

  result.ready = true;
  result.protocol = lines;
  return result;
}

module.exports = {readDecisionForm,alignDecisionProtocol};

function audit() {
  const repo = path.resolve(__dirname,'..');
  const root = '/tmp/pbs-converter-reference-5XPOEo';
  const run =
    '/home/hm/pbs-selfplay-runs/g1-1794173773869-47088a';

  const protectedPaths = [
    'tools/pbs-client.js',
    'tools/pbs-converter-v32.reference.js',
    'tools/pbs-v32-source-reader.js',
    'tools/pbs-v32-migration-manifest.json'
  ].map(x => path.join(repo,x));

  const before = protectedPaths.map(
    x => sha(fs.readFileSync(x))
  );

  assert.equal(before[0],CLIENT_HASH,'Installed client changed');
  assert.equal(before[1],HASH,'Reference changed');

  const reader = require(
    path.join(repo,'tools/pbs-v32-source-reader')
  ).createReader({
    dictionaryPath:path.join(root,'local-ja-dict-generated.js')
  });

  const {exportCandidate} = require(
    path.join(repo,'tools/pbs-v32-position-full-audit')
  );

  const inventory = JSON.parse(fs.readFileSync(
    path.join(root,'v32-conversion-inventory.json'),
    'utf8'
  ));

  const records = {};

  for (const side of ['p1','p2']) {
    const dir = path.join(run,side,'traces');
    const files = fs.readdirSync(dir)
      .filter(x => x.endsWith('.jsonl'));

    assert.equal(files.length,1,'Trace count for '+side);

    records[side] = fs.readFileSync(
      path.join(dir,files[0]),'utf8'
    ).split(/\r?\n/)
      .filter(Boolean)
      .map(JSON.parse)
      .filter(x => x.type === 'decision');
  }

  const counts = {
    snapshots:0,
    rawMatches:0,
    decisions:0,
    input:0,
    batonn:0,
    change:0,
    aligned:0,
    turnMarkersAdded:0,
    generatedUpkeepRemoved:0,
    phaseChecks:0,
    reviewRequired:0,
    failures:0
  };

  console.log('=== DECISION BOUNDARY AUDIT ===');

  for (const row of inventory.snapshots) {
    try {
      assert.equal(row.status,'converted');

      const evidence = reader.parseFile(
        path.join(root,row.side,'input',row.name)
      );

      counts.snapshots++;

      assert.equal(
        evidence.raw,
        fs.readFileSync(row.raw_path,'utf8'),
        'Raw parity'
      );
      counts.rawMatches++;

      if (/-FINAL\.html$/i.test(row.name)) continue;

      counts.decisions++;

      const n = Number(
        row.name.match(/-D(\d+)\.html$/i)?.[1]
      );
      const r = records[row.side][n-1];

      assert.ok(
        r && r.rawTurnHtml && r.spec,
        'Decision trace missing'
      );
      assert.equal(r.spec.side,r.mySide);

      const aligned = alignDecisionProtocol(
        evidence,r.rawTurnHtml,r.mySide
      );

      counts[aligned.form.action.toLowerCase()]++;

      assert.equal(
        aligned.form.action,r.phase,
        'Saved/raw form action mismatch'
      );

      if (!aligned.ready) {
        counts.reviewRequired++;

        console.log(
          'CASE='+row.name+
          ' ACTION='+aligned.form.action+
          ' TURN='+aligned.form.decisionTurn+
          ' STATUS=REVIEW_REQUIRED'+
          ' REASON='+aligned.reason
        );

        console.log(
          'SOURCE_TAIL='+
          JSON.stringify(evidence.protocol.slice(-8))
        );
        continue;
      }

      const forced = aligned.form.action !== 'INPUT';

      const requestRecord = {
        ...r,
        forceSwitch:forced,
        spec:{
          ...r.spec,
          force_switch:forced,
          turn:aligned.form.decisionTurn
        }
      };

      const imported = exportCandidate(
        requestRecord,aligned.protocol
      ).spec;

      assert.equal(
        imported.turn,aligned.form.decisionTurn,
        'Imported turn'
      );
      assert.equal(
        imported.upkeep_this_turn,false,
        'Decision has premature upkeep'
      );
      assert.equal(
        imported.force_switch,forced,
        'Imported request phase'
      );

      if (forced) {
        assert.equal(
          imported.sides[r.mySide].pending_bp,true,
          'Missing pending Baton Pass'
        );
        assert.equal(
          imported.sides[r.mySide].acted_this_turn,true,
          'Baton actor did not act'
        );
      } else {
        for (const side of imported.sides) {
          assert.equal(
            side.acted_this_turn,false,
            'Normal input already acted'
          );
          assert.equal(
            side.pending_bp,false,
            'Normal input retains pending Baton Pass'
          );
        }
      }

      counts.aligned++;
      counts.phaseChecks++;

      counts.turnMarkersAdded += aligned.edits.filter(
        x => x.operation === 'append'
      ).length;

      counts.generatedUpkeepRemoved += aligned.edits.filter(
        x => x.operation === 'remove'
      ).length;

      console.log(
        'CASE='+row.name+
        ' ACTION='+aligned.form.action+
        ' CONVERTER_TURN='+aligned.converterTurn+
        ' DECISION_TURN='+imported.turn+
        ' UPKEEP='+imported.upkeep_this_turn+
        ' STATUS=ALIGNED'
      );

    } catch (error) {
      counts.failures++;
      console.error(
        'CASE='+row.name+
        ' ERROR='+String(error.stack || error)
      );
    }
  }

  assert.deepEqual(
    protectedPaths.map(x => sha(fs.readFileSync(x))),
    before,
    'Existing files changed'
  );

  for (const [key,value] of Object.entries(counts)) {
    console.log(
      key.replace(/[A-Z]/g,x=>'_'+x).toUpperCase()+
      '='+value
    );
  }

  const pass =
    counts.snapshots === 19 &&
    counts.rawMatches === 19 &&
    counts.decisions === 17 &&
    counts.input === 14 &&
    counts.batonn === 1 &&
    counts.change === 2 &&
    counts.aligned === 15 &&
    counts.phaseChecks === 15 &&
    counts.turnMarkersAdded === 12 &&
    counts.generatedUpkeepRemoved === 1 &&
    counts.reviewRequired === 2 &&
    counts.failures === 0;

  console.log(
    'SUPPORTED_BOUNDARY_GATE='+(pass?'PASS':'FAIL')
  );
  console.log('CHANGE_AUTOMATIC_IMPORT=BLOCKED');
  console.log('LIVE_POSITION_CONNECTED=NO');
  console.log('SEMANTIC_MIGRATION_COMPLETE=NO');
  console.log('EXISTING_SOURCE_MODIFIED=NO');
  console.log('P2_UNCHANGED=YES');
  console.log('PBS_CONNECTION=NOT_ATTEMPTED');
  console.log('BATTLE_TEST=NOT_STARTED');
  console.log('GIT_COMMIT=NOT_ATTEMPTED');
  console.log('GIT_PUSH=NOT_ATTEMPTED');

  if (!pass) process.exitCode = 1;
}

if (require.main === module) {
  try {
    audit();
  } catch (e) {
    console.error(e.stack || e);
    process.exitCode = 1;
  }
}
