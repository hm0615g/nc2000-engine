// Add dependencies only while the bake's original input files still match.
// This is a metadata migration, never a remeasurement or a new bake.
import fs from 'node:fs';
import crypto from 'node:crypto';
import { priorSnapshot } from '../web/src/preview-prior.mjs';
const digest = value => crypto.createHash('sha256').update(value).digest('hex');
const read = p => JSON.parse(fs.readFileSync(p, 'utf8'));
const paths = process.argv.slice(2);
if (!paths.length) throw Error('Usage: node tools/preview-prior-metadata.mjs ARTIFACT...');
const priorPath = 'data/belief-pool-v3/belief-pool.json';
const shells = await priorSnapshot(read(priorPath), read('data/gen2stadium2.json'), digest);
for (const path of paths) {
  const artifact = read(path);
  if (artifact.status) throw Error(`Cannot migrate retired artifact: ${path}`);
  for (const [input, expected] of Object.entries(artifact.inputs)) {
    if (digest(fs.readFileSync(input)) !== expected) throw Error(`Stale bake input: ${input}`);
  }
  if ((artifact.priorSha256 || artifact.inputs[priorPath]) !== digest(fs.readFileSync(priorPath))) throw Error('Prior mismatch');
  artifact.priorDependencies = { version: 1, shells };
  fs.writeFileSync(path, JSON.stringify(artifact) + '\n');
}
