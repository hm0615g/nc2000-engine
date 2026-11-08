'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');
const { computeEntropyNorm, normalizeAction } = require('./diagnose-position');

const REPO = path.resolve(__dirname, '..');

test('action normalization handles varied formats', () => {
  assert.equal(normalizeAction('move bodyslam'), 'move bodyslam');
  assert.equal(normalizeAction('move:bodyslam'), 'move bodyslam');
  assert.equal(normalizeAction('move_bodyslam'), 'move bodyslam');
  assert.equal(normalizeAction('  Switch:2  '), 'switch 2');
});

test('entropy norm computes correctly', () => {
  // Single action -> 0 entropy
  assert.equal(computeEntropyNorm([{ visits: 100, frac: 1.0 }]), 0);

  // Equal 2 actions -> 1.0 entropy
  const equalTwo = [
    { visits: 50, frac: 0.5 },
    { visits: 50, frac: 0.5 },
  ];
  assert(Math.abs(computeEntropyNorm(equalTwo) - 1.0) < 1e-4);

  // Skewed actions -> 0 < entropy < 1
  const skewed = [
    { visits: 90, frac: 0.9 },
    { visits: 10, frac: 0.1 },
  ];
  const ent = computeEntropyNorm(skewed);
  assert(ent > 0 && ent < 1.0);
});

test('diagnose-position CLI --self-check outputs valid JSON', () => {
  const stdout = execFileSync('node', ['tools/diagnose-position.js', '--self-check'], {
    cwd: REPO,
    encoding: 'utf8',
  }).trim();
  const res = JSON.parse(stdout);
  assert.equal(res.status, 'OK');
  assert.equal(res.wasm_loaded, true);
  assert.equal(res.manifest_present, true);
});

test('bundle-core-sdk generates valid manifest and bundle', () => {
  const stdout = execFileSync('node', ['tools/bundle-core-sdk.js'], {
    cwd: REPO,
    encoding: 'utf8',
  }).trim();
  const res = JSON.parse(stdout);
  assert.equal(res.status, 'ok');
  assert.equal(typeof res.git_sha, 'string');
  assert(res.files_count >= 11);

  const manifestPath = path.join(REPO, 'dist/nc2000-core-sdk/engine-manifest.json');
  assert(fs.existsSync(manifestPath));
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  assert.equal(manifest.schema_version, 'nc2000-sdk-v1');
  assert.equal(manifest.position_schema, 'nc2000-position-v1');
  assert(typeof manifest.wasm_sha256 === 'string');
});
