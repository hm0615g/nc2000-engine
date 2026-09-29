// App shell: engine + meta pool loading and the select -> game screen
// switch. A Game instance is keyed by game number so rematch / new-teams
// remount it cleanly.
//
// M12 product policy: strength is fixed at max (30k iterations — ponder
// hides the wait) and the information policy is OPEN TEAM SHEET — both
// sides' sets are public, only selection (which 3 of 6 + lead, until
// revealed) is hidden. No settings.
//
// M18 adds two things, neither of them a setting on the screen. The
// information mode (open / blind, see info-mode.ts) is read once from the
// URL at module load — the query string is the only door, and nothing the
// user can press moves it; a GameSpec still carries the mode its game
// started under, which is the guarantee game.tsx is written against. The
// team pool is state: one file replaces the pool everywhere it is read
// (team-pool.ts). The bundled pool is held next to the active one so going
// back to it is a state change rather than a second trip to the network —
// and, since the swap is now blind-only, so that open mode has the
// untouched pool to play no matter what the user loaded (see `activePool`).
//
// Three team files, three questions (docs/TEAM-POOL-REBUILD-PLAN.md):
// which team the bot brings (the ordinary own-team pool, team-pool-v1:
// teams classified strong; or, on `?nash`, the solved mixture,
// meta-nash-v2), what the bot assumes about the team it faces (the shipped
// opponent prior, belief-pool-v2, on every blind door), and which teams a
// human can pick from the lists (the bundled pool, meta-pool-v0, or the
// user's pool file under `?blind`). None of them is read in another's place.
// The bot's draw lives in one place, `drawOpponent`, because the start
// screen and the rematch must roll identically. `?nash` has nothing
// configurable: it pins the bundled lists as open mode does and never passes
// a belief prior table.

import { useEffect, useState } from "preact/hooks";
import { loadEngine } from "./engine";
import {
  fetchBeliefPool,
  fetchDexJson,
  fetchI18nJa,
  fetchNashArtifact,
  fetchOwnPool,
  fetchPool,
} from "./data";
import { loadSetDex } from "./set-info";
import { randomPoolTeam, type SelectedTeam } from "./pool-pick";
import { drawNashTeam, parseNashArtifact, type NashMix } from "./nash-mix";
import { drawOwnTeam, parseOwnPool, type OwnPool } from "./own-pool";
import { infoModeOf, readDoor, type Door, type InfoMode } from "./info-mode";
import {
  clearStoredPool,
  loadStoredPool,
  parsePoolText,
  type LoadedPool,
} from "./team-pool";
import { loadStoredPrior, type StoredPrior } from "./belief-prior";
import { StartScreen } from "./select";
import { Game } from "./game";
import { Solver } from "./solver";
import { loadJaNames, locale, setLocale, ui, type Locale } from "./i18n";

/** `SelectedTeam` moved to pool-pick.ts, next to the pool draw that builds
 * one; re-exported here so the existing `from "./app"` imports keep
 * working. */
export type { SelectedTeam } from "./pool-pick";

/** This page load's door and the information mode it implies. Read at
 * module scope because that is the truth about them: the query string cannot
 * change without a navigation, and holding either in state would suggest
 * something here could flip it. */
const DOOR: Door = readDoor();
const MODE: InfoMode = infoModeOf(DOOR);
const NASH = DOOR === "nash";
/** The study board is not a way to play: it replaces the whole screen, so it
 * is checked before any of the game state below is consulted. */
const SOLVER = DOOR === "solver";

interface GameSpec {
  human: SelectedTeam;
  bot: SelectedTeam;
  n: number;
  /** The information mode this game runs under, frozen at start. */
  mode: InfoMode;
}

/** The pool this browser was handed in an earlier session — re-validated,
 * never trusted: it is text the user picked by hand, saved by an older
 * build, against a validator that may since have moved. A file that no
 * longer parses is dropped without a word: boot must not hang on a stale
 * preference, and there is nowhere honest to report a file the user is not
 * loading right now. The bundled pool then stands, as it did before.
 *
 * Dropped *and deleted*, though. This runs after the engine is up, so a
 * failure here is a verdict on the record, not on the browser: it will fail
 * the same way on every future load, costing a full validator pass each
 * time, while the only control that could remove it — the panel's reset
 * button — is disabled exactly when the bundled pool is in play. A record
 * that cannot be adopted and cannot be cleared is unreachable forever, so
 * the read is what clears it. */
function restoreStoredPool(): LoadedPool | null {
  try {
    const stored = loadStoredPool();
    if (!stored) return null;
    const parsed = parsePoolText(stored.json);
    if (!parsed.ok) {
      clearStoredPool();
      return null;
    }
    return { name: stored.name, pool: parsed.pool, poolJson: parsed.poolJson };
  } catch {
    clearStoredPool();
    return null;
  }
}

export function App() {
  const [status, setStatus] = useState<"loading" | "error" | "ready">(
    "loading",
  );
  const [error, setError] = useState("");
  // The pool the user chose, and the bundled one it can always fall back to.
  // The same object until a file is loaded, but two references: "use the
  // bundled pool" has to work after a swap, and the bundled pool is already
  // in memory — refetching it to get it back would be the one path that can
  // fail offline. Which of the two is actually played is `activePool`, below.
  const [bundled, setBundled] = useState<LoadedPool | null>(null);
  const [loadedPool, setLoadedPool] = useState<LoadedPool | null>(null);
  const [game, setGame] = useState<GameSpec | null>(null);
  // The solved mixture, on the `?nash` door only. Null everywhere else, and
  // never null on a nash page that got past boot: a failure to load or
  // validate it fails the page (see the boot effect).
  const [nashMix, setNashMix] = useState<NashMix | null>(null);
  // The bot's ordinary own-team pool; null only on the solver door, which
  // plays no game.
  const [ownPool, setOwnPool] = useState<OwnPool | null>(null);
  // The shipped opponent prior: the blind searcher's candidate set only, on
  // every blind door and the solver. Loading a pool file under `?blind`
  // changes the lists and the draw, never this.
  const [beliefJson, setBeliefJson] = useState<string | null>(null);
  const [loc, setLoc] = useState<Locale>(locale());
  // A table the user once picked by hand; nothing here ever fetches one on
  // its own (crates/bot/src/prior.rs:491).
  const [prior, setPrior] = useState<StoredPrior | null>(loadStoredPrior);

  useEffect(() => {
    void (async () => {
      try {
        // JP name tables and the set-sheet dex load alongside the engine;
        // both swallow failures (missing tables just mean English names /
        // sheets without move meta).
        // The nash artifact rides along in the same wave — one door's
        // 11 KB, fetched only on that door, never on the product page.
        const [, pd, , , nashText, beliefPd, ownText] = await Promise.all([
          loadEngine(),
          fetchPool(),
          loadJaNames(fetchI18nJa),
          loadSetDex(fetchDexJson),
          NASH ? fetchNashArtifact() : Promise.resolve(""),
          // The solver reasons with the same shipped prior every blind bot
          // uses: identification is the whole of what the opponent panel's
          // "moves shown" field buys, and this table is what it is bought
          // against.
          MODE === "blind" || SOLVER ? fetchBeliefPool() : Promise.resolve(null),
          SOLVER ? Promise.resolve("") : fetchOwnPool(),
        ]);
        const bundledPool: LoadedPool = {
          name: null,
          pool: pd.pool,
          poolJson: pd.poolJson,
        };
        setBundled(bundledPool);
        // Parsed after the engine is up, because validating the mixture is
        // a wasm call. Strict: a mixture that cannot play takes the page
        // down with it rather than leaving `?nash` running as plain blind
        // under a name that promises otherwise.
        if (NASH) {
          const parsed = parseNashArtifact(nashText);
          if (!parsed.ok) throw new Error(parsed.errors.join("; "));
          setNashMix(parsed.mix);
        }
        if (MODE === "blind" || SOLVER) {
          setBeliefJson(beliefPd ? beliefPd.poolJson : null);
        }
        if (!SOLVER) {
          const own = parseOwnPool(ownText);
          if (!own.ok) throw new Error(own.errors.join("; "));
          setOwnPool(own.pool);
        }
        // Only now: re-validating a stored pool runs the wasm validator,
        // which the engine load above is what makes available. Restored in
        // either mode — the record belongs to the user, not to the mode, so
        // coming back to `?blind` finds the file still loaded, and a record
        // that has gone bad gets swept whichever door they came in by.
        setLoadedPool(restoreStoredPool() ?? bundledPool);
        setStatus("ready");
      } catch (e) {
        setError(String(e));
        setStatus("error");
      }
    })();
  }, []);

  if (status === "loading") {
    return (
      <div class="center-screen">
        <div class="loading-pulse">{ui().loadingEngine}</div>
      </div>
    );
  }
  if (
    status === "error" ||
    !loadedPool ||
    !bundled ||
    (NASH && !nashMix) ||
    ((MODE === "blind" || SOLVER) && !beliefJson) ||
    (!SOLVER && !ownPool)
  ) {
    return (
      <div class="center-screen">
        <div class="error-box">
          <strong>{ui().failedLoad}</strong>
          <div>{error}</div>
        </div>
      </div>
    );
  }

  // The list pool: the teams on the start screen, and under `?blind` with a
  // loaded file also the bot's draw. A loaded file stays in state — it is
  // the user's, and `?blind` will find it again — but only blind mode plays
  // it: whoever opens `/` gets the bundled lists, every time. Keeping the
  // file live in open mode while hiding the control that loaded it would
  // leave a stored pool quietly rewriting the public team lists, with no
  // sign of why and nothing on screen to undo it.
  // Nash sits on the bundled side of this line with open mode: the mode is
  // a fixed configuration or it is not the conclusion, so a pool file the
  // user once loaded through `?blind` must not quietly redefine what the
  // bot brings here. `?blind` still finds that file, untouched.
  const activePool = MODE === "blind" && !NASH && !SOLVER ? loadedPool : bundled;

  // The study board takes over the page. Nothing below it runs: there is no
  // game, no opponent draw, and no team selection — a position is typed,
  // not played into.
  if (SOLVER) {
    return (
      <Solver
        pool={bundled.pool}
        poolJson={beliefJson!}
        locale={loc}
        onLocale={(l) => {
          setLocale(l);
          setLoc(l);
        }}
      />
    );
  }

  /** The bot's team when nobody pinned one, in one place because every
   * caller must roll the same way: Start (blind, and open with "Random"),
   * and every blind rematch. Nash samples the solved mixture; a pool file
   * the user loaded under `?blind` is drawn uniformly, because that file is
   * what they chose to face; otherwise the ordinary own-team pool. */
  const drawOpponent = (): SelectedTeam =>
    NASH && nashMix
      ? drawNashTeam(nashMix)
      : activePool.name !== null
        ? randomPoolTeam(activePool.pool)
        : drawOwnTeam(ownPool!);

  if (!game) {
    return (
      <StartScreen
        loadedPool={activePool}
        bundledPool={bundled}
        onPool={setLoadedPool}
        locale={loc}
        onLocale={(l) => {
          setLocale(l);
          setLoc(l);
        }}
        mode={MODE}
        nash={NASH}
        nashMix={nashMix}
        drawOpponent={drawOpponent}
        botDrawCount={
          activePool.name !== null ? activePool.pool.teams.length : ownPool!.teams.length
        }
        prior={prior}
        onPrior={setPrior}
        onStart={(human, bot) => setGame({ human, bot, n: 1, mode: MODE })}
      />
    );
  }

  return (
    <Game
      key={game.n}
      // The searcher's candidate pool. Blind: the shipped prior, reaching
      // ONLY the belief (a blind game never fetches pair tables, game.tsx).
      // Open: the list pool, which the pinned belief never consults.
      poolJson={game.mode === "blind" && beliefJson ? beliefJson : activePool.poolJson}
      // Baked artifacts are indexed by the bundled pool's rank order, so a
      // swapped pool's indices name different teams entirely. Open mode is
      // never custom by construction, which is what gives the public build
      // its pair tables back.
      poolIsCustom={activePool.name !== null}
      humanTeam={game.human}
      botTeam={game.bot}
      mode={game.mode}
      // The prior only ever reaches a blind game: in open mode the searcher
      // pins the human's real team and refuses the table outright. Nash is
      // blind and still gets none — the mode ships one configuration, and a
      // table left in storage by an earlier `?blind` visit is exactly the
      // kind of invisible state it must not inherit.
      priorJson={
        game.mode === "blind" && !NASH ? prior?.json : undefined
      }
      // Blind rematch redraws the opponent: replaying a lost battle against
      // the team you just watched play would hand the human the very
      // information blind mode withholds. Open mode keeps the same foe.
      onRematch={() =>
        setGame((g) =>
          g === null
            ? g
            : {
                ...g,
                n: g.n + 1,
                bot: g.mode === "blind" ? drawOpponent() : g.bot,
              },
        )
      }
      onNewTeams={() => setGame(null)}
    />
  );
}
