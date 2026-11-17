// Vite config for the nc2000 browser demo.
//
// Three things are non-standard:
//
// 1. The wasm pkg lives OUTSIDE the web root (../crates/wasm/pkg-web) and is
//    imported by relative path. `server.fs.allow` widens dev serving to the
//    repo root; the build follows the import and emits the .wasm as a hashed
//    asset (wasm-bindgen's `new URL('..._bg.wasm', import.meta.url)` is
//    bundler-visible).
//
// 2. Battle data (team files, prior, dex tables) is NEVER bundled: the app
//    fetches <base>data/* read-only at runtime. The same
//    middleware is installed in the dev server AND the preview (built dist)
//    server; the GH Pages build instead copies data/ into dist/data/
//    (.github/workflows/pages.yml), and any other production server must
//    map <base>data/ to the repo data/ dir.
//
// 3. The deploy base is env-driven: `NC2000_BASE=/nc2000-engine/` for the
//    GH Pages build (project pages live under a subpath), unset = `/` for
//    local dev/preview. Runtime data fetches follow via
//    `import.meta.env.BASE_URL` (src/data.ts).

import { defineConfig, type Plugin, type Connect } from "vite";
import path from "node:path";
import fs from "node:fs";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

import { compatiblePolicyKeys } from "./src/preview-prior.mjs";

const webDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(webDir, "..");
const dataDir = path.join(repoRoot, "data");
const base = process.env.NC2000_BASE ?? "/";
// Research builds execute live search exclusively; the default production
// build still requires a freshly baked, source-matching preview artifact.
const compiledPreviewEnabled = process.env.NC2000_DISABLE_COMPILED_PREVIEW !== "1";

function serveRepoData(): Plugin {
  // The app fetches `${BASE_URL}data/...`; accept both the based path and
  // a bare /data/ (robust when the dist is re-served under another base).
  // The resolved base wins over NC2000_BASE when a caller overrides it.
  let prefixes = [...new Set([`${base}data/`, "/data/"])];
  const handler: Connect.NextHandleFunction = (req, res, next) => {
    const url = (req.url ?? "").split("?")[0];
    const prefix = prefixes.find((p) => url.startsWith(p));
    if (!prefix) return next();
    const rel = decodeURIComponent(url.slice(prefix.length));
    const file = path.normalize(path.join(dataDir, rel));
    if (!file.startsWith(dataDir + path.sep) || !file.endsWith(".json")) {
      res.statusCode = 403;
      res.end("forbidden");
      return;
    }
    let stat;
    try {
      stat = fs.statSync(file);
    } catch {
      res.statusCode = 404;
      res.end("not found");
      return;
    }
    if (!stat.isFile()) {
      res.statusCode = 404;
      res.end("not found");
      return;
    }
    res.setHeader("Content-Type", "application/json");
    fs.createReadStream(file).pipe(res);
  };
  return {
    name: "serve-repo-data",
    configResolved(config) {
      prefixes = [...new Set([`${config.base}data/`, "/data/"])];
    },
    configureServer(server) {
      server.middlewares.use(handler);
    },
    configurePreviewServer(server) {
      server.middlewares.use(handler);
    },
  };
}

function readEpochs(): { engineEpoch: string; policyEpoch: string } {
  const replaySrc = fs.readFileSync(path.join(repoRoot, "crates/engine/src/replay.rs"), "utf8");
  const previewSrc = fs.readFileSync(path.join(repoRoot, "crates/bot/src/compiled_preview.rs"), "utf8");
  const engineMatch = replaySrc.match(/pub const ENGINE_EPOCH:\s*u32\s*=\s*(\d+)\s*;/);
  const policyMatch = previewSrc.match(/pub const POLICY_EPOCH:\s*&str\s*=\s*"([^"]+)"\s*;/);
  if (!engineMatch || !policyMatch) {
    throw new Error("Could not parse ENGINE_EPOCH / POLICY_EPOCH from Rust sources.");
  }
  return { engineEpoch: engineMatch[1], policyEpoch: policyMatch[1] };
}

function sha256File(relative: string): string {
  return createHash("sha256").update(fs.readFileSync(path.join(repoRoot, relative))).digest("hex");
}

// Soft preview cache: compiled previews accelerate Turn-0 Team Preview when
// `POLICY_EPOCH`, dex, search coefficient, and shell priors match; on any
// mismatch, builds warn and runtime seamlessly falls back to live search.
function verifyCompiledPreview(): Plugin {
  const { policyEpoch } = readEpochs();
  return {
    name: "verify-compiled-preview",
    apply: "build",
    enforce: "pre",
    async load(id) {
      if (!compiledPreviewEnabled || !id.endsWith('/data/compiled-preview-v1.json?raw')) return;
      const previewPath = path.join(dataDir, "compiled-preview-v1.json");
      if (!fs.existsSync(previewPath)) {
        return `export default ${JSON.stringify(JSON.stringify({ format: "nc2000-compiled-preview-v1", build: policyEpoch, c: 0.4, priorSha256: "", dexSha256: "", iterations: 0, policies: {} }))};`;
      }
      const artifact = JSON.parse(fs.readFileSync(previewPath, "utf8"));
      const profiles = JSON.parse(fs.readFileSync(path.join(dataDir, "search-profiles.json"), "utf8"));
      const dexSha = sha256File("data/gen2stadium2.json");
      if (artifact.status || artifact.format !== "nc2000-compiled-preview-v1" || artifact.build !== policyEpoch || artifact.c !== profiles.blind.c || artifact.dexSha256 !== dexSha) {
        artifact.policies = {};
        return `export default ${JSON.stringify(JSON.stringify(artifact))};`;
      }
      if (!artifact.priorDependencies) return;
      const prior = JSON.parse(fs.readFileSync(path.join(dataDir, "belief-pool-v3/belief-pool.json"), "utf8"));
      const dex = JSON.parse(fs.readFileSync(path.join(dataDir, "gen2stadium2.json"), "utf8"));
      const valid = await compatiblePolicyKeys(artifact, prior, dex,
        (text: string) => createHash("sha256").update(text).digest("hex"));
      // Only ship policy keys whose shell priors still match.
      artifact.policies = Object.fromEntries(Object.entries(artifact.policies ?? {}).filter(([key]) => valid.has(key)));
      return `export default ${JSON.stringify(JSON.stringify(artifact))};`;
    },
    async buildStart() {
      if (!compiledPreviewEnabled) return;
      const previewPath = path.join(dataDir, "compiled-preview-v1.json");
      if (!fs.existsSync(previewPath)) {
        this.warn("compiled-preview-v1.json not found; browser previews will use live search.");
        return;
      }
      const artifact = JSON.parse(fs.readFileSync(previewPath, "utf8"));
      const catalog = JSON.parse(fs.readFileSync(path.join(dataDir, "team-pool-v2/team-pool.json"), "utf8"));
      const profiles = JSON.parse(fs.readFileSync(path.join(dataDir, "search-profiles.json"), "utf8"));
      if (artifact.status) {
        this.warn(`compiled-preview-v1.json status=${artifact.status}; browser previews will use live search.`);
        return;
      }
      if (artifact.build !== policyEpoch) {
        this.warn(`compiled-preview-v1.json build=${artifact.build} != ${policyEpoch}; browser previews will use live search.`);
        return;
      }
      if (artifact.c !== profiles.blind.c) {
        this.warn(`compiled-preview-v1.json c=${artifact.c} != ${profiles.blind.c}; browser previews will use live search.`);
        return;
      }
      if (artifact.dexSha256 !== sha256File("data/gen2stadium2.json")) {
        this.warn("compiled-preview-v1.json dexSha256 differs; browser previews will use live search.");
        return;
      }
      const publicRosters = new Set(catalog.teams.map((t: { sets: { species: string; level: number }[] }) => JSON.stringify(t.sets.map(p => [p.species, p.level]))));
      if (artifact.format !== "nc2000-compiled-preview-v1" || artifact.matchups !== catalog.teams.length * publicRosters.size || Object.keys(artifact.policies ?? {}).length !== artifact.matchups) {
        this.warn("compiled-preview-v1.json coverage is incomplete; missing keys will use live search.");
        return;
      }
      if (artifact.priorDependencies) {
        const prior = JSON.parse(fs.readFileSync(path.join(dataDir, "belief-pool-v3/belief-pool.json"), "utf8"));
        const dex = JSON.parse(fs.readFileSync(path.join(dataDir, "gen2stadium2.json"), "utf8"));
        const valid = await compatiblePolicyKeys(artifact, prior, dex,
          (text: string) => createHash("sha256").update(text).digest("hex"));
        if (valid.size !== artifact.matchups)
          this.warn(`${artifact.matchups - valid.size} preview keys have changed shell priors and will use live search.`);
      }
    },
  };
}

function evaluatorBuildId(): string {
  const { engineEpoch, policyEpoch } = readEpochs();
  const hash = createHash("sha256");
  hash.update(`engine:${engineEpoch}|policy:${policyEpoch}|`);
  for (const relative of [
    "data/gen2stadium2.json",
    "data/live-search.json",
    "data/search-profiles.json",
  ]) {
    hash.update(relative).update(fs.readFileSync(path.join(repoRoot, relative)));
  }
  hash.update(process.env.VITE_NC2000_TEST_BUDGET ?? "production");
  return hash.digest("hex");
}

export default defineConfig({
  base,
  define: {
    __EVALUATOR_BUILD__: JSON.stringify(evaluatorBuildId()),
    __COMPILED_PREVIEW_ENABLED__: compiledPreviewEnabled,
  },
  plugins: [verifyCompiledPreview(), serveRepoData()],
  esbuild: {
    jsx: "automatic",
    jsxImportSource: "preact",
  },
  server: {
    fs: { allow: [repoRoot] },
  },
  worker: {
    format: "es",
    plugins: () => [verifyCompiledPreview()],
  },
  build: {
    target: "es2020",
  },
});
