'use strict';

const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const {integrateV32Position} = require('./pbs-v32-semantic-integration');

const ROOT = '/tmp/pbs-converter-reference-5XPOEo';
const RUN = '/home/hm/pbs-selfplay-runs/g1-1794173773869-47088a';
const EXPECT =
  '79dff80a4689bdb3869fa8649887f5108e4ebf6a30ff146b68d9fa2c69540e4e';

assert.equal(
  crypto.createHash('sha256')
    .update(fs.readFileSync(
      path.join(__dirname,'pbs-converter-v32.reference.js')
    ))
    .digest('hex'),
  EXPECT
);

const inv = JSON.parse(fs.readFileSync(
  path.join(ROOT,'v32-conversion-inventory.json'),
  'utf8'
));

const wasm = require(path.join(
  __dirname,'..','crates/wasm/pkg-node/nc2000_wasm.js'
));

const pool = fs.readFileSync(
  path.join(__dirname,'..','data/belief-pool-v3/belief-pool.json'),
  'utf8'
);

const dex = new wasm.Dex();
const traces = {};

for (const side of ['p1','p2']) {
  const dir = path.join(RUN,side,'traces');
  const filenames = fs.readdirSync(dir)
    .filter(x => x.endsWith('.jsonl'));

  assert.equal(filenames.length,1);

  traces[side] = fs.readFileSync(
    path.join(dir,filenames[0]),'utf8'
  )
    .split(/\r?\n/)
    .filter(Boolean)
    .map(JSON.parse)
    .filter(x => x.type === 'decision');
}

const stats = {
  snapshots:0,
  accepted:0,
  rejected:0,
  applied:0,
  fieldChanges:0,
  boostChanges:0,
  chargingChanges:0,
  lockedChanges:0,
  otherChanges:0,
  conflicts:0,
  engineRejected:0
};

const notes = [];

function same(a,b) {
  return JSON.stringify(a) === JSON.stringify(b);
}

function preserve(a,b,where) {
  assert.ok(same(a,b),where);
}

function checkAuthority(before,after) {
  for (const k of [
    'schema','side','turn','upkeep_this_turn',
    'own_sets','weather','team_preview',
    'force_switch','trapped'
  ]) {
    preserve(before[k],after[k],k);
  }

  for (let s=0;s<2;s++) {
    for (const k of [
      'active','party','pending_bp','acted_this_turn',
      'fainted_this_turn','fainted_last_turn'
    ]) {
      preserve(
        before.sides[s][k],
        after.sides[s][k],
        `sides[${s}].${k}`
      );
    }

    for (let i=0;i<before.sides[s].mons.length;i++) {
      const a = before.sides[s].mons[i];
      const b = after.sides[s].mons[i];

      for (const k of [
        'species','level','gender','name','item_flag',
        'appeared','appear_count','switch_in_turn',
        'active','fainted','hp_num','hp_den',
        'hp_exact','pp','item_now','hp_type_mask'
      ]) {
        preserve(
          a[k],b[k],
          `sides[${s}].mons[${i}].${k}`
        );
      }

      if (a.active || s === before.side) {
        preserve(
          a.status,b.status,
          `sides[${s}].mons[${i}].status`
        );
      }
    }
  }
}

console.log('=== V32 OFFLINE INTEGRATION GATE ===');

for (const row of inv.snapshots) {
  stats.snapshots++;

  if (row.name.includes('-FINAL')) continue;

  const n = Number(
    row.name.match(/-D(\d+)\.html$/i)?.[1]
  );

  const record = traces[row.side][n-1];

  assert.ok(
    record?.spec && record.rawLlogHtml,
    row.name+':missing saved trace'
  );

  try {
    const before = record.spec;

    const {spec:after,report} = integrateV32Position({
      spec:before,
      rawLlogHtml:record.rawLlogHtml,
      pdat:record.pdat,
      pickedRosterSlots:record.pickedRosterSlots
    });

    assert.ok(
      report.status === 'OK' ||
      report.status === 'OK_WITH_CONFLICTS',
      'oracle skipped '+report.status
    );

    assert.notStrictEqual(after,before);
    checkAuthority(before,after);

    let checker;

    try {
      checker = new wasm.ProtocolSearcher(
        dex,record.mySide,pool,1,0.4
      );

      checker.setPosition(JSON.stringify(after));
    } catch (e) {
      stats.engineRejected++;
      throw new Error('ENGINE_REJECTED:'+String(e?.message||e));
    } finally {
      if (checker?.free) checker.free();
    }

    stats.accepted++;
    stats.fieldChanges += report.changes.length;

    if (report.applied) stats.applied++;

    stats.conflicts += report.conflicts.length;

    for (const ch of report.changes) {
      if (ch.path.endsWith('.boosts')) {
        stats.boostChanges++;
      } else if (ch.path.endsWith('.charging')) {
        stats.chargingChanges++;
      } else if (ch.path.endsWith('.locked')) {
        stats.lockedChanges++;
      } else {
        stats.otherChanges++;
      }
    }

    notes.push({
      case:row.name,
      changes:report.changes,
      conflicts:report.conflicts
    });

    console.log(
      'CASE='+row.name+
      ' ACCEPTED=YES'+
      ' CHANGES='+report.changes.length+
      ' BOOST_CHANGES='+report.changes.filter(
        x => x.path.endsWith('.boosts')
      ).length+
      ' CONFLICTS='+report.conflicts.length
    );

    for (const conflict of report.conflicts.slice(0,2)) {
      console.log(' CONFLICT='+conflict);
    }
  } catch (e) {
    stats.rejected++;

    console.log(
      'CASE='+row.name+
      ' ACCEPTED=NO ERROR='+
      String(e?.stack||e).split('\n').slice(0,8).join(' / ')
    );
  }
}

console.log('=== SUMMARY ===');

for (const [k,v] of Object.entries(stats)) {
  console.log(k.toUpperCase()+'='+v);
}

const output = path.join(
  RUN,'diagnostics',
  'pbs-v32-semantic-integration-gate.json'
);

fs.writeFileSync(
  output,
  JSON.stringify({
    converterSha256:EXPECT,
    stats,
    notes
  },null,2)+'\n',
  {mode:0o600}
);

console.log('DETAILS='+output);

const ok =
  stats.snapshots === 19 &&
  stats.accepted === 17 &&
  stats.rejected === 0 &&
  stats.engineRejected === 0 &&
  stats.boostChanges === 5;

console.log('OFFLINE_GATE='+(ok?'PASS':'FAIL'));
console.log('LIVE_BATTLE_TEST=NOT_STARTED');
console.log('P2_UNCHANGED=YES');
console.log('PUSH=NOT_ATTEMPTED');

if (!ok) process.exitCode=1;
