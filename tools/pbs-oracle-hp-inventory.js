'use strict';

const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');

const {createBridge} = require('./pbs-converter-bridge');
const {
  planOracleBenchHp,
  applyPlannedBenchHp
} = require('./pbs-oracle-hp-evidence');

const ROOT = '/tmp/pbs-converter-reference-5XPOEo';
const RUN = '/home/hm/pbs-selfplay-runs/g1-1794173773869-47088a';

const inventory = JSON.parse(
  fs.readFileSync(
    path.join(ROOT, 'v32-conversion-inventory.json'),
    'utf8'
  )
);

const bridge = createBridge({
  dictionaryPath: path.join(
    ROOT, 'local-ja-dict-generated.js'
  )
});

const totals = {
  snapshots: 0,
  byteSame: 0,
  decisionSnapshots: 0,
  oracleErrors: 0,
  oracleUnknown: 0,
  topChecks: 0,
  conflicts: 0,
  benchCandidates: 0,
  benchPlannedChanges: 0,
  benchSkippedNoMax: 0
};

const changes = [];
const conflicts = [];
const warnings = new Map();
const decisionsBySide = {};

for (const side of ['p1', 'p2']) {
  const dir = path.join(RUN, side, 'traces');

  const names = fs.readdirSync(dir)
    .filter(x => x.endsWith('.jsonl'));

  assert.equal(names.length, 1);

  decisionsBySide[side] = fs.readFileSync(
    path.join(dir, names[0]), 'utf8'
  ).split(/\r?\n/)
    .filter(Boolean)
    .map(JSON.parse)
    .filter(x => x.type === 'decision');
}

for (const row of inventory.snapshots) {
  assert.equal(row.status, 'converted');

  const input = path.join(
    ROOT, row.side, 'input', row.name
  );

  const oracle = bridge.parseFile(input);

  const expected = fs.readFileSync(
    row.raw_path, 'utf8'
  );

  totals.snapshots++;

  if (oracle.raw === expected) totals.byteSame++;

  totals.oracleErrors += oracle.errors.length;
  totals.oracleUnknown += oracle.unknown.length;

  for (const warning of oracle.warnings || []) {
    const short = String(warning).slice(0, 160);
    warnings.set(short, (warnings.get(short) || 0) + 1);
  }

  if (row.name.includes('FINAL')) continue;

  const match = row.name.match(/-D(\d+)\.html$/i);
  assert.ok(match);

  const n = Number(match[1]);
  const record = decisionsBySide[row.side][n - 1];

  assert.ok(record?.rawLlogHtml && record?.spec);

  totals.decisionSnapshots++;

  const foeSide = 1 - record.mySide;
  const seen = record.seenFoeMonsAfter;
  const roster = record.spec.sides[foeSide].mons;
  const top = record.battleTurn?.sides?.[foeSide] ?? null;

  const plan = planOracleBenchHp({
    oracle,
    foeSide,
    previewMons: roster,
    seenFoeMons: seen,
    top
  });

  totals.topChecks += plan.topChecks;
  totals.conflicts += plan.conflicts.length;
  totals.benchPlannedChanges += plan.changes.length;

  totals.benchSkippedNoMax += plan.skipped.filter(
    x => x.reason === 'no_independently_observed_max_hp'
  ).length;

  totals.benchCandidates +=
    plan.changes.length +
    plan.skipped.filter(
      x => x.reason === 'already_agrees'
    ).length;

  for (const change of plan.changes) {
    changes.push({snapshot: row.name, ...change});
  }

  for (const conflict of plan.conflicts) {
    conflicts.push({snapshot: row.name, conflict});
  }

  const clone = JSON.parse(JSON.stringify(seen));

  const applied = applyPlannedBenchHp(clone, plan);

  assert.equal(
    applied,
    plan.conflicts.length ? 0 : plan.changes.length
  );

  for (let i = 0; i < clone.length; i++) {
    assert.equal(clone[i].maxHp, seen[i].maxHp);
  }

  console.log(
    'CASE=' + row.name +
    ' TOP_CHECKS=' + plan.topChecks +
    ' HP_CHANGES=' + plan.changes.length +
    ' CONFLICTS=' + plan.conflicts.length
  );
}

console.log('=== WARNING CATEGORIES ===');

for (const [warning, n] of warnings) {
  console.log(
    'WARNING_COUNT=' + n +
    ' WARNING=' + warning
  );
}

console.log('=== BENCH HP CANDIDATE CHANGES ===');

for (const change of changes.slice(0, 35)) {
  console.log('CHANGE=' + JSON.stringify(change));
}

console.log('=== CONFLICTS ===');

for (const conflict of conflicts.slice(0, 35)) {
  console.log('CONFLICT=' + JSON.stringify(conflict));
}

console.log('=== RESULT ===');

for (const [key, value] of Object.entries(totals)) {
  console.log(key.toUpperCase() + '=' + value);
}

const parityPass =
  totals.snapshots === 19 &&
  totals.byteSame === 19 &&
  totals.decisionSnapshots === 17;

const observationPass =
  parityPass &&
  totals.oracleErrors === 0 &&
  totals.oracleUnknown === 0 &&
  totals.conflicts === 0;

console.log(
  'RAW_PARITY=' + (parityPass ? 'PASS' : 'FAIL')
);

console.log(
  'HP_OBSERVATION_GATE=' +
  (observationPass ? 'PASS' : 'REVIEW_REQUIRED')
);

console.log('PBS_CONNECTION=NOT_ATTEMPTED');
console.log('BATTLE_TEST=NOT_STARTED');

if (!observationPass) process.exitCode = 1;
