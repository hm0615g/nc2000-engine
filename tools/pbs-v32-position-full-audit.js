'use strict';

// Offline Castle v32 -> ProtocolTracker -> PositionSpec comparison.
// Uses saved real Castle logs. Does not access the PBS server.
// Reusable function: exportCandidate(record, protocolLines).

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');

const REPO = path.resolve(__dirname, '..');

const REFROOT =
  '/tmp/pbs-converter-reference-5XPOEo';

const RUN =
  '/home/hm/pbs-selfplay-runs/g1-1794173773869-47088a';

const HASH =
  '79dff80a4689bdb3869fa8649887f5108e4ebf6a30ff146b68d9fa2c69540e4e';

const wasm = require(
  path.join(REPO, 'crates/wasm/pkg-node/nc2000_wasm.js')
);

const dexData = JSON.parse(
  fs.readFileSync(
    path.join(REPO, 'data/gen2stadium2.json'),
    'utf8'
  )
);

const poolJson = fs.readFileSync(
  path.join(
    REPO,
    'data/belief-pool-v3/belief-pool.json'
  ),
  'utf8'
);

const dex = new wasm.Dex();

const norm = x => String(x || '')
  .normalize('NFD')
  .toLowerCase()
  .replace(/[^a-z0-9]/g, '');

const moveId = x => norm(x)
  .replace(/^hiddenpower.+/, 'hiddenpower');

function makeRequest(record) {
  const spec = record.spec;
  const side = record.mySide;
  const me = spec.sides[side];

  assert.ok(
    (side === 0 || side === 1) &&
    Array.isArray(spec.own_sets)
  );

  const party =
    Array.isArray(record.pickedRosterSlots) &&
    record.pickedRosterSlots.length === 3
      ? [...record.pickedRosterSlots]
      : Array.isArray(me.party) && me.party.length
        ? [...me.party]
        : [];

  if (
    !party.length &&
    Number.isInteger(me.active)
  ) {
    party.push(me.active);
  }

  for (
    let i = 0;
    i < me.mons.length && party.length < 3;
    i++
  ) {
    if (
      me.mons[i].appeared &&
      !party.includes(i)
    ) {
      party.push(i);
    }
  }

  for (
    let i = 0;
    i < me.mons.length && party.length < 3;
    i++
  ) {
    if (!party.includes(i)) {
      party.push(i);
    }
  }

  assert.equal(
    party.length,
    3,
    'Expected three picked Pokemon'
  );

  assert.equal(
    new Set(party).size,
    3,
    'Duplicate party slot'
  );

  const pdat = record.pdat?.mons || [];

  const pokemon = party.map(slot => {
    const mon = me.mons[slot];
    const own = spec.own_sets[slot];

    assert.ok(
      mon && own,
      'Missing own Pokemon in slot ' + slot
    );

    const observed = pdat.find(
      x => norm(x.speciesId) === norm(mon.species)
    );

    const species =
      dexData.species[norm(mon.species)]?.name ||
      mon.species;

    const maxHp =
      observed?.maxHp ||
      mon.hp_den ||
      100;

    const hp =
      observed?.hp ??
      mon.hp_exact ??
      Math.max(
        1,
        Math.round(
          (mon.hp_num || 100) *
          maxHp /
          (mon.hp_den || 100)
        )
      );

    const status =
      observed?.status ||
      mon.status ||
      '';

    return {
      ident:
        'p' + (side + 1) + ': ' + species,

      details:
        species +
        ', L' +
        (mon.level || own.level || 50) +
        (mon.gender ? ', ' + mon.gender : ''),

      condition:
        mon.fainted || hp <= 0
          ? '0 fnt'
          : hp + '/' + maxHp +
            (status ? ' ' + status : ''),

      active: slot === me.active,

      item:
        mon.item_now ??
        observed?.item ??
        own.item ??
        ''
    };
  });

  const request = {
    side: {
      name: 'p' + (side + 1),
      id: 'p' + (side + 1),
      pokemon
    },
    rqid: spec.turn || 1
  };

  if (
    record.forceSwitch ||
    spec.force_switch
  ) {
    request.forceSwitch = [true];
  } else {
    const active = me.mons[me.active];

    assert.ok(
      active,
      'No own active Pokemon'
    );

    const rows = [];

    if (
      Array.isArray(active.pp) &&
      active.pp.length
    ) {
      for (const row of active.pp) {
        rows.push({
          id: moveId(row.move),
          move: row.move,
          pp: row.pp,
          maxpp: row.maxpp,
          disabled: Boolean(row.disabled)
        });
      }
    } else {
      const observed = pdat.find(
        x => norm(x.speciesId) ===
          norm(active.species)
      );

      const moves =
        observed?.moves?.length
          ? observed.moves
          : (
              spec.own_sets[me.active]?.moves || []
            ).map(x => ({
              moveId: x,
              pp: null
            }));

      for (const row of moves) {
        const id = moveId(
          row.moveId || row.move
        );

        const pp =
          dexData.moves[id]?.pp || 1;

        rows.push({
          id,
          move: dexData.moves[id]?.name || id,
          pp: row.pp ?? pp,
          maxpp: pp,
          disabled: false
        });
      }
    }

    request.active = [{
      trapped: Boolean(spec.trapped),
      moves: rows
    }];
  }

  return request;
}

function exportCandidate(record, protocolLines) {
  assert.ok(
    Array.isArray(protocolLines) &&
    protocolLines.length > 0
  );

  let searcher;

  try {
    searcher = new wasm.ProtocolSearcher(
      dex,
      record.mySide,
      poolJson,
      1,
      0.4
    );

    searcher.setOwnTeam(
      JSON.stringify(record.spec.own_sets)
    );

    searcher.pushLines(
      JSON.stringify(protocolLines)
    );

    const request = makeRequest(record);

    const needsChoice = searcher.onRequest(
      JSON.stringify(request)
    );

    assert.equal(
      needsChoice,
      true,
      'Decision was not accepted'
    );

    const json = searcher.positionSpec();

    assert.ok(
      json,
      'PositionSpec was not exported'
    );

    return {
      spec: JSON.parse(json),
      request
    };
  } finally {
    if (
      searcher &&
      typeof searcher.free === 'function'
    ) {
      searcher.free();
    }
  }
}

module.exports = {
  makeRequest,
  exportCandidate
};

// Generic recursive JSON comparison.
// No PositionSpec field is excluded.

function diff(a, b, here = '', out = []) {
  if (Object.is(a, b)) {
    return out;
  }

  if (
    Array.isArray(a) &&
    Array.isArray(b)
  ) {
    if (a.length !== b.length) {
      out.push({
        path: here + '.length',
        old: a.length,
        candidate: b.length
      });
    }

    for (
      let i = 0;
      i < Math.min(a.length, b.length);
      i++
    ) {
      diff(
        a[i],
        b[i],
        here + '[' + i + ']',
        out
      );
    }
  } else if (
    a &&
    b &&
    !Array.isArray(a) &&
    !Array.isArray(b) &&
    typeof a === 'object' &&
    typeof b === 'object'
  ) {
    const keys = [
      ...new Set([
        ...Object.keys(a),
        ...Object.keys(b)
      ])
    ].sort();

    for (const key of keys) {
      const place = here
        ? here + '.' + key
        : key;

      if (
        !Object.hasOwn(a, key) ||
        !Object.hasOwn(b, key)
      ) {
        out.push({
          path: place,
          old: Object.hasOwn(a, key)
            ? a[key]
            : '<absent>',
          candidate: Object.hasOwn(b, key)
            ? b[key]
            : '<absent>'
        });
      } else {
        diff(
          a[key],
          b[key],
          place,
          out
        );
      }
    }
  } else {
    out.push({
      path: here || '(root)',
      old: a,
      candidate: b
    });
  }

  return out;
}

// These arrays represent unordered collections.
// Arrays where order matters, such as party,
// boosts and PP slots, are NOT sorted.

const UNORDERED = new Set([
  'uses',
  'volatiles',
  'conditions',
  'revealed_moves'
]);

function normalized(value, key = '') {
  if (Array.isArray(value)) {
    const items = value.map(
      x => normalized(x)
    );

    if (UNORDERED.has(key)) {
      items.sort(
        (a, b) =>
          JSON.stringify(a).localeCompare(
            JSON.stringify(b)
          )
      );
    }

    return items;
  }

  if (
    value &&
    typeof value === 'object'
  ) {
    return Object.fromEntries(
      Object.entries(value).map(
        ([k, v]) => [k, normalized(v, k)]
      )
    );
  }

  return value;
}

function audit() {
  const {createBridge} = require(
    './pbs-converter-bridge'
  );

  const reference = path.join(
    __dirname,
    'pbs-converter-v32.reference.js'
  );

  assert.equal(
    crypto.createHash('sha256')
      .update(fs.readFileSync(reference))
      .digest('hex'),
    HASH,
    'Reference SHA256 mismatch'
  );

  console.log('REFERENCE_SHA256=PASS');

  const inventory = JSON.parse(
    fs.readFileSync(
      path.join(
        REFROOT,
        'v32-conversion-inventory.json'
      ),
      'utf8'
    )
  );

  const bridge = createBridge({
    dictionaryPath: path.join(
      REFROOT,
      'local-ja-dict-generated.js'
    )
  });

  const records = {};

  for (const side of ['p1', 'p2']) {
    const folder = path.join(
      RUN,
      side,
      'traces'
    );

    const found = fs.readdirSync(folder)
      .filter(x => x.endsWith('.jsonl'));

    assert.equal(
      found.length,
      1,
      'Trace count for ' + side
    );

    records[side] = fs.readFileSync(
      path.join(folder, found[0]),
      'utf8'
    )
      .split(/\r?\n/)
      .filter(Boolean)
      .map(JSON.parse)
      .filter(x => x.type === 'decision');
  }

  const tally = {
    snapshots: 0,
    rawByteMatches: 0,
    decisions: 0,
    exported: 0,
    importerErrors: 0,
    rawFieldDifferences: 0,
    semanticFieldDifferences: 0,
    exactPositionMatches: 0,
    semanticPositionMatches: 0
  };

  const report = {
    converterSha256: HASH,
    count: tally,
    cases: [],
    fieldGroups: {}
  };

  const groups = new Map();

  console.log(
    '=== FULL FIELD REAL-LOG COMPARISON ==='
  );

  for (const row of inventory.snapshots) {
    assert.equal(
      row.status,
      'converted'
    );

    const input = path.join(
      REFROOT,
      row.side,
      'input',
      row.name
    );

    const oracle = bridge.parseFile(input);

    assert.equal(
      oracle.raw,
      fs.readFileSync(row.raw_path, 'utf8'),
      'Raw parity: ' + row.name
    );

    assert.equal(
      oracle.errors.length,
      0,
      'Converter error: ' + row.name
    );

    assert.equal(
      oracle.unknown.length,
      0,
      'Unknown log line: ' + row.name
    );

    tally.snapshots++;
    tally.rawByteMatches++;

    if (row.name.endsWith('-FINAL.html')) {
      continue;
    }

    tally.decisions++;

    const match = row.name.match(
      /-D(\d+)\.html$/i
    );

    assert.ok(
      match,
      'Invalid decision file name: ' +
      row.name
    );

    const index = Number(match[1]);

    const record =
      records[row.side][index - 1];

    assert.ok(
      record?.spec?.own_sets &&
      record?.rawLlogHtml,
      'Saved trace missing: ' + row.name
    );

    const result = {
      name: row.name,
      status: 'FAILED',
      error: null,
      rawDiffs: [],
      semanticDiffs: [],
      candidateSpec: null,
      request: null
    };

    try {
      const candidate = exportCandidate(
        record,
        oracle.protocol
      );

      result.candidateSpec = candidate.spec;
      result.request = candidate.request;

      result.rawDiffs = diff(
        record.spec,
        candidate.spec
      );

      result.semanticDiffs = diff(
        normalized(record.spec),
        normalized(candidate.spec)
      );

      result.status = 'EXPORTED';

      tally.exported++;

      tally.rawFieldDifferences +=
        result.rawDiffs.length;

      tally.semanticFieldDifferences +=
        result.semanticDiffs.length;

      if (!result.rawDiffs.length) {
        tally.exactPositionMatches++;
      }

      if (!result.semanticDiffs.length) {
        tally.semanticPositionMatches++;
      }

      for (const change of result.semanticDiffs) {
        const key = change.path.replace(
          /\[\d+\]/g,
          '[]'
        );

        groups.set(
          key,
          (groups.get(key) || 0) + 1
        );
      }
    } catch (error) {
      result.error = String(
        error?.stack || error
      )
        .split('\n')
        .slice(0, 3)
        .join(' / ');

      tally.importerErrors++;
    }

    report.cases.push(result);

    console.log(
      'CASE=' + row.name +
      ' STATUS=' + result.status +
      ' RAW_DIFF=' + result.rawDiffs.length +
      ' SEMANTIC_DIFF=' +
        result.semanticDiffs.length +
      (
        result.error
          ? ' ERROR=' +
            result.error.slice(0, 250)
          : ''
      )
    );
  }

  const ranked = [...groups].sort(
    (a, b) =>
      b[1] - a[1] ||
      a[0].localeCompare(b[0])
  );

  report.fieldGroups =
    Object.fromEntries(ranked);

  console.log(
    '=== ALL DIFFERING FIELD PATHS ==='
  );

  for (const [key, count] of ranked) {
    console.log(
      'FIELD=' + key +
      ' COUNT=' + count
    );
  }

  console.log(
    '=== FIRST 40 SEMANTIC DIFFERENCES ==='
  );

  let shown = 0;

  for (const item of report.cases) {
    for (const d of item.semanticDiffs) {
      if (shown >= 40) break;

      console.log(
        'DIFF=' + item.name +
        ' PATH=' + d.path +
        ' CURRENT=' +
          JSON.stringify(d.old).slice(0, 180) +
        ' V32=' +
          JSON.stringify(d.candidate).slice(0, 180)
      );

      shown++;
    }

    if (shown >= 40) break;
  }

  const reportPath = path.join(
    RUN,
    'diagnostics',
    'pbs-v32-full-state-audit.json'
  );

  fs.mkdirSync(
    path.dirname(reportPath),
    {recursive: true}
  );

  fs.writeFileSync(
    reportPath,
    JSON.stringify(report, null, 2) + '\n',
    {mode: 0o600}
  );

  console.log('=== RESULT ===');

  for (const [key, value] of Object.entries(tally)) {
    console.log(
      key.replace(/([A-Z])/g, '_$1').toUpperCase() +
      '=' + value
    );
  }

  console.log(
    'DIFFERING_FIELD_GROUPS=' +
    ranked.length
  );

  console.log(
    'AUDIT_JSON=' + reportPath
  );

  const ok =
    tally.snapshots === 19 &&
    tally.rawByteMatches === 19 &&
    tally.decisions === 17 &&
    tally.exported === 17 &&
    tally.importerErrors === 0;

  console.log(
    'RESULT=' +
    (
      ok
        ? 'FULL_STATE_AUDIT_PASS'
        : 'FULL_STATE_AUDIT_REVIEW_REQUIRED'
    )
  );

  console.log(
    'ALL_CONVERTER_RULES_PROVED=NO'
  );

  console.log('PBS_CLIENT_MODIFIED=NO');
  console.log('P2_UNCHANGED=YES');
  console.log('PBS_CONNECTION=NOT_ATTEMPTED');
  console.log('BATTLE_TEST=NOT_STARTED');
  console.log('GIT_PUSH=NOT_ATTEMPTED');

  if (!ok) {
    process.exitCode = 1;
  }
}

if (require.main === module) {
  audit();
}
