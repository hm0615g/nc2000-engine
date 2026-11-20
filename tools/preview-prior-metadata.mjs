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
const dexPath = 'data/gen2stadium2.json';
const priorSha = digest(fs.readFileSync(priorPath));
const dexSha = digest(fs.readFileSync(dexPath));
const shells = await priorSnapshot(read(priorPath), read(dexPath), digest);
for (const path of paths) {
  const artifact = read(path);
  if (artifact.status) throw Error(`Cannot migrate retired artifact: ${path}`);
  if (artifact.dexSha256 && artifact.dexSha256 !== dexSha) throw Error('Dex mismatch');
  if (artifact.priorSha256 && artifact.priorSha256 !== priorSha) throw Error('Prior mismatch');
  artifact.priorDependencies = { version: 1, shells };
  fs.writeFileSync(path, JSON.stringify(artifact) + '\n');
}
