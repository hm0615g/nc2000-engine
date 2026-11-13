'use strict';

// Offline integration bridge for the exact PBS converter v32.

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const Module = require('node:module');
const assert = require('node:assert/strict');

const REFERENCE_SHA =
  '79dff80a4689bdb3869fa8649887f5108e4ebf6a30ff146b68d9fa2c69540e4e';

const DEFAULT_REFERENCE = path.join(
  __dirname,
  'pbs-converter-v32.reference.js'
);

const DEFAULT_REPO = path.resolve(__dirname, '..');

function createBridge({
  referencePath = DEFAULT_REFERENCE,
  repoPath = DEFAULT_REPO,
  dictionaryPath
} = {}) {

  const bytes = fs.readFileSync(referencePath);

  const actualSha = crypto.createHash('sha256')
    .update(bytes).digest('hex');

  assert.equal(
    actualSha,
    REFERENCE_SHA,
    'Reference converter must be exact v32'
  );

  let source = bytes.toString('utf8');

  const anchor =
    'return {battleId, stem, raw: lines.join';

  assert.equal(
    source.split(anchor).length,
    2,
    'Converter return anchor changed'
  );

  // Export the original parser without editing its source file.
  // Expose its internal state in addition to raw.log.
  source = source.replace(
    anchor,
    'return {oracleState: state, battleId, stem, raw: lines.join'
  );

  source +=
    '\nmodule.exports = { parseBattle, loadJaDict };\n';

  const mod = new Module(referencePath, module);

  mod.filename = referencePath;
  mod.paths = Module._nodeModulePaths(
    path.dirname(referencePath)
  );

  mod._compile(source, referencePath);

  const converter = mod.exports;

  if (dictionaryPath) {
    converter.loadJaDict(dictionaryPath);
  } else {
    const ja = JSON.parse(
      fs.readFileSync(
        path.join(repoPath, 'data/i18n-ja.json'),
        'utf8'
      )
    );

    const dex = JSON.parse(
      fs.readFileSync(
        path.join(repoPath, 'data/gen2stadium2.json'),
        'utf8'
      )
    );

    const names = {
      pokemon: {},
      moves: {},
      items: {}
    };

    for (const [group, table] of [
      ['species', 'pokemon'],
      ['moves', 'moves'],
      ['items', 'items']
    ]) {
      for (const [id, japanese] of Object.entries(ja[group])) {
        const english = dex[group][id]?.name;

        if (english) {
          names[table][english] = japanese;
        }
      }
    }

    const os = require('node:os');

    const tmp = fs.mkdtempSync(
      path.join(os.tmpdir(), 'pbs-v32-ja-')
    );

    try {
      const filename = path.join(tmp, 'ja.js');

      fs.writeFileSync(
        filename,
        'window.NC2000_JA_NAMES=' +
        JSON.stringify(names) +
        ';\n',
        {mode: 0o600}
      );

      converter.loadJaDict(filename);
    } finally {
      fs.rmSync(tmp, {
        recursive: true,
        force: true
      });
    }
  }

  const args = {
    battlePrefix:
      'battle-gen2nintendocup2000noohkostadium2strict-pbs',
    missingTimestampPolicy: 'synthetic',
    includeIncomplete: true,
    allowNoDict: true
  };

  function parseFile(file) {
    const output = converter.parseBattle(file, args, '');
    const state = output.oracleState;

    const health = {};

    for (const side of ['p1', 'p2']) {
      health[side] = {};

      for (
        const [species, mon] of
        state.monState[side].entries()
      ) {
        health[side][species] = {
          hpAbs: mon.hpAbs,
          maxHp: mon.maxHp,
          status: mon.status,
          fainted: mon.fainted,
          active: mon.active,
          itemUsed: mon.itemUsed,
          substitute: mon.substitute,
          trapped: mon.trapped,
          leechSeededBy: mon.leechSeededBy
        };
      }
    }

    const protocol = output.raw
      .split(/\r?\n/)
      .filter(Boolean);

    const eventCounts = {};

    for (const line of protocol) {
      if (!line.startsWith('|')) continue;

      const type = line.split('|')[1];

      eventCounts[type] =
        (eventCounts[type] || 0) + 1;
    }

    return {
      raw: output.raw,
      warnings: output.warnings,
      errors: output.errors,
      unknown: output.unknown,
      statAudit: output.statAudit,
      automaticRepairs: output.automaticRepairs,
      complete: output.complete,
      health,
      active: {...state.active},
      spikes: {...state.spikes},
      perish: JSON.parse(JSON.stringify(state.perish)),
      protocol,
      eventCounts
    };
  }

  function parseHtml(html) {
    let prepared = String(html || '');

    // PBS may leave its preview table cell unclosed.
    const openings = [
      ...prepared.matchAll(
        /<td\s+valign="middle"\s*>/gi
      )
    ].map(x => x.index);

    if (
      openings.length === 2 &&
      !/<\/td\s*>\s*$/i.test(
        prepared.slice(openings[0], openings[1])
      )
    ) {
      prepared =
        prepared.slice(0, openings[1]) +
        '</TD>' +
        prepared.slice(openings[1]);
    }

    // Remove non-battle navigation links only.
    prepared = prepared.replace(
      /<p\b[^>]*>[\s\S]*?<\/p>/gi,
      paragraph =>
        /ACTION=PDAT/i.test(paragraph) &&
        /ACTION=SLOG/i.test(paragraph)
          ? ''
          : paragraph
    );

    const os = require('node:os');

    const dir = fs.mkdtempSync(
      path.join(os.tmpdir(), 'pbs-v32-html-')
    );

    try {
      const filename = path.join(
        dir,
        'pbs-current.html'
      );

      fs.writeFileSync(
        filename,
        prepared,
        'utf8'
      );

      return parseFile(filename);
    } finally {
      fs.rmSync(dir, {
        recursive: true,
        force: true
      });
    }
  }

  return {
    parseFile,
    parseHtml
  };
}

module.exports = {
  createBridge,
  REFERENCE_SHA
};
