#!/usr/bin/env node
'use strict';

// Offline planning and readiness checks only. No PBS network requests.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');

const ROOT = path.resolve(__dirname, '..');
const P2_FILES = [
  'tools/pbs-client.js',
  'tools/pbs-parity-audit.js',
  'tools/export-pdinput.js',
  'crates/wasm/pkg-node/nc2000_wasm.js',
  'crates/wasm/pkg-node/nc2000_wasm_bg.wasm',
  'crates/wasm/pkg-node/package.json',
  'data/gen2stadium2.json',
  'data/i18n-ja.json',
  'data/search-profiles.json',
  'data/live-search.json',
  'data/team-pool-v2/team-pool.json',
  'data/kodoku-r3/conclusion-equilibrium-deme.json',
  'data/belief-pool-v3/belief-pool.json',
  'data/compiled-preview-v1.json',
];

function fail(message) {
  console.error('ERROR=' + message);
  process.exit(1);
}
function shellQuote(value) {
  return "'" + String(value).replace(/'/g, "'\\''") + "'";
}
function positiveInt(value, name) {
  if (!/^[0-9]+$/.test(String(value)) || Number(value) < 1 || !Number.isSafeInteger(Number(value))) {
    fail('invalid ' + name + ': ' + value);
  }
  return Number(value);
}
function parseArgs(argv) {
  const opts = { mode: 'plan', teamCount: 10, games: 90, seed: 20261109,
    remote: '', key: '', remoteRoot: '/home/debian/nc2000-p2' };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--plan' || arg === '--preflight') {
      opts.mode = arg.slice(2);
      continue;
    }
    const valueKeys = {
      '--teams': 'teamCount', '--games': 'games', '--seed': 'seed',
      '--remote': 'remote', '--key': 'key', '--remote-root': 'remoteRoot',
    };
    const key = valueKeys[arg];
    if (!key || i + 1 === argv.length || argv[i + 1].startsWith('--')) {
      fail('unknown or incomplete option: ' + arg);
    }
    opts[key] = argv[++i];
  }
  for (const key of ['teamCount', 'games', 'seed']) opts[key] = positiveInt(opts[key], key);
  return opts;
}
// Plan selection uses ONLY the checked-in nc2000-engine team catalog.
// Score a team by the NEW species, moves and held items it contributes.
// This is coverage testing, not a claim about competitive team strength.
function featureId(value) {
  return String(value || '').normalize('NFD').toLowerCase().replace(/[^a-z0-9]/g, '');
}
const FEATURE_WEIGHTS = { species: 6, moves: 2, items: 3 };
const FEATURE_KINDS = Object.keys(FEATURE_WEIGHTS);

function makeFeatures(sets) {
  const features = { species: new Set(), moves: new Set(), items: new Set() };
  for (const mon of sets) {
    const species = featureId(mon.species);
    if (!species) fail('catalog contains a set with no species');
    features.species.add(species);
    for (const move of mon.moves || []) {
      const moveId = featureId(move);
      if (moveId) features.moves.add(moveId);
    }
    const itemId = featureId(mon.item);
    if (itemId) features.items.add(itemId);
  }
  return features;
}

function hasLegalThreePokemonSelection(sets) {
  for (let i = 0; i < sets.length; i++) {
    for (let j = i + 1; j < sets.length; j++) {
      for (let k = j + 1; k < sets.length; k++) {
        const sum = [sets[i], sets[j], sets[k]]
          .reduce((total, mon) => total + Number(mon.level ?? 50), 0);
        if (sum <= 155) return true;
      }
    }
  }
  return false;
}

function summarizeCoverage(teams) {
  const all = { species: new Set(), moves: new Set(), items: new Set() };
  for (const team of teams) {
    for (const kind of FEATURE_KINDS) {
      for (const value of team.features[kind]) all[kind].add(value);
    }
  }
  const result = {};
  let weighted = 0;
  for (const kind of FEATURE_KINDS) {
    result[kind] = all[kind].size;
    weighted += all[kind].size * FEATURE_WEIGHTS[kind];
  }
  result.weighted = weighted;
  return result;
}

function buildSchedule(opts) {
  const source = 'data/team-pool-v2/team-pool.json';
  const catalog = JSON.parse(fs.readFileSync(path.join(ROOT, source), 'utf8'));
  if (!Array.isArray(catalog.teams)) fail('team catalog has no teams array');

  const knownIds = new Set();
  const candidates = [];
  for (const entry of catalog.teams) {
    if (!entry || typeof entry.id !== 'string' || !entry.id ||
        !Array.isArray(entry.sets) || entry.sets.length !== 6 || knownIds.has(entry.id)) {
      continue;
    }
    knownIds.add(entry.id);
    if (!hasLegalThreePokemonSelection(entry.sets)) continue;
    const features = makeFeatures(entry.sets);
    const rosterKey = entry.sets.map(mon => featureId(mon.species)).sort().join('|');
    candidates.push({ id: entry.id, features, rosterKey });
  }

  const rosterCount = new Set(candidates.map(t => t.rosterKey)).size;
  if (rosterCount < opts.teamCount || opts.teamCount < 2) {
    fail('not enough distinct legal six-Pokemon rosters for ' + opts.teamCount + ' teams');
  }

  const selected = [];
  const usedRosterKeys = new Set();
  const covered = { species: new Set(), moves: new Set(), items: new Set() };
  while (selected.length < opts.teamCount) {
    let best = null;
    let bestGain = -1;
    for (const candidate of candidates) {
      if (usedRosterKeys.has(candidate.rosterKey)) continue;
      let gain = 0;
      for (const kind of FEATURE_KINDS) {
        for (const feature of candidate.features[kind]) {
          if (!covered[kind].has(feature)) gain += FEATURE_WEIGHTS[kind];
        }
      }
      // Stable tie-break, independent of the catalog's row order.
      if (gain > bestGain || (gain === bestGain && best && candidate.id < best.id)) {
        bestGain = gain;
        best = candidate;
      }
    }
    if (!best) fail('diversity selection exhausted candidates');
    selected.push(best);
    usedRosterKeys.add(best.rosterKey);
    for (const kind of FEATURE_KINDS) {
      for (const feature of best.features[kind]) covered[kind].add(feature);
    }
  }

  const baseline = candidates.slice(0, opts.teamCount);
  const ids = selected.map(t => t.id);
  const pairs = [];
  for (let i = 0; i < ids.length; i++) {
    for (let j = i + 1; j < ids.length; j++) {
      pairs.push([ids[i], ids[j]]);
      pairs.push([ids[j], ids[i]]);
    }
  }
  // Deterministically reorder pairs; never synthesize or edit party sets.
  let state = opts.seed >>> 0;
  const random = () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  for (let i = pairs.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    [pairs[i], pairs[j]] = [pairs[j], pairs[i]];
  }

  return {
    source,
    catalogCandidates: candidates.length,
    selectionMethod: 'weighted-new-coverage-with-distinct-species-rosters',
    weights: FEATURE_WEIGHTS,
    selectedCoverage: summarizeCoverage(selected),
    baselineCoverage: summarizeCoverage(baseline),
    ids,
    uniquePairs: pairs.length,
    games: Array.from({ length: opts.games }, (_, i) => ({
      number: i + 1,
      p1: pairs[i % pairs.length][0],
      p2: pairs[i % pairs.length][1],
      p1Seed: opts.seed + i * 2,
      p2Seed: opts.seed + i * 2 + 1,
    })),
  };
}
function checkLocalFiles() {
  for (const rel of P2_FILES) {
    if (!fs.existsSync(path.join(ROOT, rel))) fail('missing local file: ' + rel);
  }
  execFileSync(process.execPath, ['tools/pbs-client.js', '--help'], { cwd: ROOT, stdio: 'ignore' });
  console.log('LOCAL_CLIENT_HELP=PASS');
  console.log('LOCAL_P2_FILES=' + P2_FILES.length);
}
function sha256File(rel) {
  return crypto.createHash('sha256').update(fs.readFileSync(path.join(ROOT, rel))).digest('hex');
}
function checkRemote(opts) {
  if (!opts.remote || !opts.key) fail('--preflight requires --remote and --key');
  if (!fs.existsSync(opts.key)) fail('SSH key not found');
  const script = [
    'set -e',
    'cd ' + shellQuote(opts.remoteRoot),
    './bin/node --version',
    './bin/node tools/pbs-client.js --help >/dev/null',
    'echo REMOTE_CLIENT_HELP=PASS',
    'sha256sum -- ' + P2_FILES.map(shellQuote).join(' '),
  ].join('; ');
  const out = execFileSync('ssh', [
    '-i', opts.key, '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=15',
    opts.remote, script,
  ], { encoding: 'utf8', maxBuffer: 2 * 1024 * 1024 });
  const lines = out.trim().split(/\r?\n/);
  if (lines[0] !== process.version) fail('remote Node.js version differs: ' + lines[0]);
  if (lines[1] !== 'REMOTE_CLIENT_HELP=PASS') fail('remote client --help failed');
  const hashes = new Map();
  for (const line of lines.slice(2)) {
    const match = line.match(/^([a-f0-9]{64})\s+(.+)$/);
    if (!match) fail('unexpected remote checksum output: ' + line);
    hashes.set(match[2], match[1]);
  }
  for (const rel of P2_FILES) {
    if (hashes.get(rel) !== sha256File(rel)) fail('remote file checksum mismatch: ' + rel);
  }
  console.log('REMOTE_NODE_VERSION=' + lines[0]);
  console.log('REMOTE_CLIENT_HELP=PASS');
  console.log('REMOTE_FILE_HASHES=PASS (' + P2_FILES.length + ' files)');
}
function main() {
  const opts = parseArgs(process.argv.slice(2));
  console.log('MODE=' + opts.mode);
  console.log('LIVE_BATTLES=DISABLED');
  checkLocalFiles();
  const schedule = buildSchedule(opts);
  console.log('TEAM_SOURCE=' + schedule.source);
  console.log('TEAM_SELECTION=' + schedule.selectionMethod);
  console.log('CATALOG_CANDIDATES=' + schedule.catalogCandidates);
  console.log('FEATURE_WEIGHTS=' + JSON.stringify(schedule.weights));
  console.log('FIRST_N_BASELINE_COVERAGE=' + JSON.stringify(schedule.baselineCoverage));
  console.log('DIVERSE_SELECTION_COVERAGE=' + JSON.stringify(schedule.selectedCoverage));
  console.log('DIVERSITY_WEIGHTED_GAIN=' +
    (schedule.selectedCoverage.weighted - schedule.baselineCoverage.weighted));
  console.log('SELECTED_TEAMS=' + schedule.ids.join(','));
  console.log('UNIQUE_ORDERED_PAIRINGS=' + schedule.uniquePairs);
  console.log('PLANNED_GAMES=' + schedule.games.length);
  console.log('P1_MODE=host / normal search');
  console.log('P2_MODE=join / random legal choices');
  for (const row of schedule.games.slice(0, 10)) {
    console.log('PLAN_GAME_' + row.number + '=' + row.p1 + ' vs ' + row.p2 + ' seed=' + row.p1Seed);
  }
  if (schedule.games.length > 10) console.log('MORE_GAMES=' + (schedule.games.length - 10));
  if (opts.mode === 'preflight') checkRemote(opts);
  console.log('PBS_CONNECTION=NOT_ATTEMPTED');
  console.log('BATTLE_TEST=NOT_STARTED');
  console.log('OFFLINE_PREFLIGHT=PASS');
}
if (require.main === module) {
  try { main(); } catch (err) { fail(err.message || String(err)); }
}
module.exports = { buildSchedule, checkLocalFiles, checkRemote };
