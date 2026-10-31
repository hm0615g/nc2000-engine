import artifactJson from "../../data/compiled-preview-v1.json?raw";
import forkAttestationJson from "../../data/compiled-preview-fork-attestation-v1.json?raw";

interface CompiledPreview {
  format: string;
  build: string;
  c: number;
  priorSha256: string;
  dexSha256: string;
  iterations: number;
  policies: Record<string, string[]>;
}

interface ForkAttestation {
  format: string;
  kind: string;
  artifactSha256: string;
  artifactBuild: string;
  currentBuild: string;
  policySha256: string;
  policyCount: number;
}

const artifact = JSON.parse(artifactJson) as CompiledPreview;
const forkAttestation = JSON.parse(forkAttestationJson) as ForkAttestation;

let forkAttestationValid = false;

// Every catalog team occurs as an exact own team in the complete artifact.
// Recover its public gender/item-presence facts without inspecting a human's
// hidden set. Ambiguous public rosters conservatively use live search.
const publicRosters = new Map<string, string | null>();
for (const key of Object.keys(artifact.policies)) {
  const [own] = JSON.parse(key) as [unknown[][]];
  const roster = JSON.stringify(own.map(p => [p[0], p[1]]));
  const facts = JSON.stringify(own.map(p => [p[0], p[1], p[2], p[8] !== null]));
  publicRosters.set(
    roster,
    publicRosters.has(roster) && publicRosters.get(roster) !== facts
      ? null
      : facts,
  );
}

async function digest(text: string): Promise<string> {
  const hash = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text),
  );

  return Array.from(
    new Uint8Array(hash),
    n => n.toString(16).padStart(2, "0"),
  ).join("");
}

export async function compiledPreviewMatches(
  poolJson: string,
  dexJson: string,
): Promise<boolean> {
  forkAttestationValid = false;

  if (artifact.format !== "nc2000-compiled-preview-v1") return false;

  const [
    prior,
    dex,
    artifactSha256,
    policySha256,
  ] = await Promise.all([
    digest(poolJson),
    digest(dexJson),
    digest(artifactJson),
    digest(JSON.stringify(artifact.policies)),
  ]);

  const inputsMatch =
    prior === artifact.priorSha256 &&
    dex === artifact.dexSha256;

  forkAttestationValid =
    inputsMatch &&
    forkAttestation.format ===
      "nc2000-compiled-preview-fork-attestation-v1" &&
    forkAttestation.kind === "diagnostic-only-source-delta-v1" &&
    forkAttestation.artifactSha256 === artifactSha256 &&
    forkAttestation.artifactBuild === artifact.build &&
    forkAttestation.policySha256 === policySha256 &&
    forkAttestation.policyCount === Object.keys(artifact.policies).length;

  return inputsMatch;
}

function compiledPreviewBuildMatches(build: string): boolean {
  if (build === artifact.build) return true;

  return (
    forkAttestationValid &&
    build === forkAttestation.currentBuild
  );
}

/** Both the key and build identity come from the engine. The key contains
 * only our own set and the opponent's public roster, never their hidden set.
 * Sampling the precomputed seeds preserves search-seed diversity. */
export function compiledPreviewPick(
  build: string,
  key: string | undefined,
  seed: number,
  c: number,
  cacheInfo?: string,
): string | undefined {
  if (
    !compiledPreviewBuildMatches(build) ||
    c !== artifact.c ||
    key === undefined ||
    !cacheInfo
  ) {
    return undefined;
  }

  const [, foe] = JSON.parse(key);
  const observed = JSON.parse(cacheInfo).opponentPublic as unknown[][];

  if (
    publicRosters.get(JSON.stringify(foe)) !==
    JSON.stringify(observed.map(p => p.slice(0, 4)))
  ) {
    return undefined;
  }

  const choices = artifact.policies[key];

  return choices?.length
    ? choices[(seed >>> 0) % choices.length]
    : undefined;
}
