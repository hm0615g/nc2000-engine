# Compact replays and counterfactual forks

The bot battle screen exposes **棋譜 → 棋譜をコピー** during play and automatically
shows the export panel after a game. Copy contains one `NC2-…` code, with no title,
human-readable log, URL or other surrounding text. **別の手を試す** opens the same
record in a new tab. Everything runs in the static page, including compression.

`?fork` accepts a pasted code, a URL, or HTML/text containing a code. It lists the
recorded bot decisions, shows the exact active HP, and offers the legal moves and
switches at that point. The selected alternative can start a human game or a paired
bot comparison. URL entry accepts `?fork&kifu=NC2-…` and `?fork#kifu=NC2-…`; exported
links use the fragment. The advanced JSON interface remains at `?fork&advanced`,
and named hosted definitions remain at `?fork=NAME`.

## Another UI using this engine

Normal WASM `Battle` instances record committed choices automatically. Keep using
`applyChoice`; no separate log collector or HTML parser is needed on the exporter.

```js
const battle = new Battle(dex, JSON.stringify(p1), JSON.stringify(p2), initialSeed);
battle.applyChoice(0, "team 123");
battle.applyChoice(1, "team 456");
// Continue using applyChoice for each requested decision.
const code = battle.exportKifu(false, 1);
```

The first argument to `exportKifu` is open team sheets (`true`) or blind (`false`);
the second is the zero-based bot side. Initial teams and seed, preview level caps,
information mode and committed choices are included. An input submitted by only
one side of a simultaneous request is not yet committed and is excluded.

For an HTML log, insert `code` as a visible text node, for example inside a `pre`.
The importer can extract it from the surrounding HTML. HTML without a replay code
cannot reconstruct the missing battle state. Engines receiving arbitrary state
mutations outside `applyChoice` are not replay sources; `Battle.fromFork` therefore
does not offer export (a fork changes the future RNG seed).

```js
const replay = new Kifu(dex, code);
const { scenes } = JSON.parse(replay.scenes());
const round = scenes[0].round;
const { view, choices, played, log } = JSON.parse(replay.scene(round));
const alternative = choices.find(c => c.input !== played.input);
const forkJson = replay.fork(round, alternative.input, poolJson);
const restarted = Battle.fromFork(dex, forkJson, futureSeed);
const bot = ProtocolSearcher.fromFork(dex, forkJson, poolJson, botSeed, 0.4);
```

Free WASM objects when finished. Native callers use `replay::Recorder` and
`replay::Replay` from `nc2000-engine`.

## Exactness and compatibility

Decoding reruns the original seed and committed choices. The fork retains exact
HP, PP, hidden status durations, remaining actions in an interrupted turn, and
every other battle field. Only the RNG for future events is reseeded. Both bots'
information sets are rebuilt from their own player-visible histories; the true
opponent state is not injected into a blind agent. Comparisons use the same future
seed for both candidate arms of each trial. Search uses the current bot and pool.

Codes carry a format version, an engine-source/dex fingerprint and a checksum.
A different engine or dex is rejected, rather than reconstructing an approximate
board. UI-only forks sharing the engine and dex can exchange codes. Preserve an
appropriate engine build if old records must remain readable after rule changes.

The binary stores packed effective teams and legal-choice indexes, then chooses
raw DEFLATE only when it is shorter. It uses URL-safe Base64 without padding.
For the current engine's 1000-turn cap, six-mon rosters, three selections per side,
and 24-byte nicknames, a conservative bound is **2090 ASCII characters**, including
the `NC2-` prefix. The decoder bounds payload size and decompression before replay.

Tests cover every decision in the 30-game conformance corpus, unspecified-gender
initialization, input order, unfinished simultaneous requests, a 1000-turn Baton
Pass game, corrupted/version-mismatched records, exact fork state, and browser
copy/import/human/bot flows.
