// Semantic dependencies of the Bayesian preview game. Shared by CI and workers.
// Keep raw file hashes as provenance; never substitute today's prior for the
// distribution used by a bake. Both players' shell distributions matter.
const id = value => String(value ?? '').toLowerCase().replace(/[^a-z0-9]/g, '');
const stats = ['hp', 'atk', 'def', 'spa', 'spd', 'spe'];
const sorted = rows => rows.sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b), 'en'));
export const shellOf = rows => JSON.stringify(sorted(rows));

export function teamMaterial(sets, dex) {
  return sets.map(p => {
    const species = id(p.species);
    if (!dex.species[species]) throw Error(`Unknown species: ${species}`);
    // Unspecified random gender cannot establish a stable public shell.
    const gender = p.gender ?? dex.species[species].gender;
    if (!['M', 'F', 'N'].includes(gender)) throw Error('Explicit gender required');
    const item = id(p.item);
    if (item && !dex.items[item]) throw Error(`Unknown item: ${item}`);
    const moves = p.moves.map(m => {
      const name = id(m);
      if (!dex.moves[name]) throw Error(`Unknown move: ${name}`);
      return name.startsWith('hiddenpower') ? 'hiddenpower' : name;
    });
    return [species, p.level, gender, p.happiness ?? 255,
      stats.map(s => Math.min(31, Math.max(0, p.ivs?.[s] ?? 31)) & 30),
      stats.map(s => Math.min(255, Math.max(0, p.evs?.[s] ?? 0))), item, moves];
  }).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0], 'en'));
}
export const materialShell = material => shellOf(material.map(p => [p[0], p[1], p[2], !!p[6]]));

// Exact decimal ratios: no tolerance that could hide a small weight change.
function decimal(value) {
  if (!(value > 0) || !Number.isFinite(value)) throw Error('Invalid prior weight');
  const [mantissa, exponent = '0'] = String(value).split('e');
  const [whole, fraction = ''] = mantissa.split('.');
  const scale = fraction.length - Number(exponent);
  return [BigInt(whole + fraction) * 10n ** BigInt(Math.max(0, -scale)), Math.max(0, scale)];
}
const gcd = (a, b) => b ? gcd(b, a % b) : a;
export function priorDistributions(pool, dex) {
  const groups = new Map();
  for (const team of pool.teams) {
    const material = teamMaterial(team.sets, dex), shell = materialShell(material);
    if (!groups.has(shell)) groups.set(shell, []);
    groups.get(shell).push([JSON.stringify(material), decimal(team.weight ?? 1)]);
  }
  return Object.fromEntries([...groups].map(([shell, entries]) => {
    const scale = Math.max(...entries.map(([, [, s]]) => s));
    const weights = new Map();
    for (const [type, [n, s]] of entries) weights.set(type, (weights.get(type) ?? 0n) + n * 10n ** BigInt(scale - s));
    const divisor = [...weights.values()].reduce(gcd);
    const distribution = [...weights].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)
      .map(([type, weight]) => [type, String(weight / divisor)]);
    return [shell, JSON.stringify(distribution)];
  }));
}

export async function priorSnapshot(pool, dex, digest) {
  const distributions = priorDistributions(pool, dex);
  return Object.fromEntries(await Promise.all(Object.entries(distributions).map(async ([shell, value]) => [shell, await digest(value)])));
}

export function policyShells(policies, dex) {
  const species = Object.keys(dex.species).sort(); // engine Dex::Table ID order
  const publicShells = new Map();
  for (const key of Object.keys(policies)) {
    const [own] = JSON.parse(key);
    const roster = JSON.stringify(own.map(p => [p[0], p[1]]));
    const shell = shellOf(own.map(p => [species[p[0]], p[1], p[2], p[8] !== null]));
    publicShells.set(roster, publicShells.has(roster) && publicShells.get(roster) !== shell ? null : shell);
  }
  return Object.fromEntries(Object.keys(policies).map(key => {
    const [own, foe] = JSON.parse(key);
    return [key, [shellOf(own.map(p => [species[p[0]], p[1], p[2], p[8] !== null])), publicShells.get(JSON.stringify(foe))]];
  }));
}

export async function compatiblePolicyKeys(artifact, pool, dex, digest) {
  const proof = artifact.priorDependencies;
  if (proof?.version !== 1) throw Error('Missing semantic prior dependencies');
  const current = await priorSnapshot(pool, dex, digest);
  const dependencies = policyShells(artifact.policies, dex);
  return new Set(Object.entries(dependencies).filter(([, shells]) => shells.every(shell =>
    shell && proof.shells[shell] && proof.shells[shell] === current[shell])).map(([key]) => key));
}
