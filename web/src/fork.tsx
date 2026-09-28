// `?fork`: repeated games from one recorded decision point. The human keeps
// their original side; the bot's first action is one of the fork's arms,
// assigned in shuffled blocks and not shown until the turn resolves, and
// every later decision is the ladder searcher's. Results persist per fork
// document in this browser and export as tools/summarize-counterfactual.py
// rows (scores from the bot's side, like the bot-vs-bot tool's).

import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import {
  forkBattle,
  legalChoices,
  loadEngine,
  needsChoice,
  randomSeed32,
  readFork,
  stateView,
  takeNewLog,
  type Battle,
} from "./engine";
import { fetchFork, fetchI18nJa, fetchPool } from "./data";
import { loadJaNames, moveName, setLocale, speciesName, toId } from "./i18n";
import { BotWorker } from "./bot";
import { Narrator } from "./narrate";
import { ActiveCard, FieldStrip } from "./battle-ui";
import { ChoiceButtons, LogPane, ThinkChip, ThinkingBar } from "./game";
import { searchProfile } from "./search-profile";
import { sha256 } from "./evaluate-hash";
import { sheetMon } from "./set-info";
import { announce, announceAssertive } from "./announcer";
import { download, fisherP, pct, wilson } from "./fork-stats";
import { ArenaPanel } from "./fork-arena";
import type { Choice, ForkInfo, LogEntry, StateView } from "./types";
import "./fork.css";

type BotOutcome = "win" | "loss" | "tie";

interface GameRecord {
  game: number;
  arm: number;
  input: string;
  label: string;
  battleSeed: number;
  botSeed: number;
  /** The bot's result; null while the game is being played. */
  outcome: BotOutcome | null;
  forfeit: boolean;
  finalTurn: number | null;
  started: string;
  finished: string | null;
}

interface Session {
  key: string;
  arms: number;
  sessionSeed: number;
  budget: number;
  /** Arms still to be dealt from the current shuffled block. */
  block: number[];
  games: GameRecord[];
  revealed: boolean;
}

interface LoadedFork {
  json: string;
  info: ForkInfo;
  key: string;
  armNames: string[];
  humanSets: unknown[];
}

const STORE = "nc2000-fork:";

function restore(key: string, arms: number): Session | null {
  try {
    const raw = localStorage.getItem(STORE + key);
    if (!raw) return null;
    const s = JSON.parse(raw) as Session;
    if (s.key !== key || s.arms !== arms || !Array.isArray(s.games)) return null;
    const now = new Date().toISOString();
    for (const g of s.games) {
      if (g.outcome === null) {
        g.outcome = "win";
        g.forfeit = true;
        g.finished = now;
      }
    }
    return s;
  } catch {
    return null;
  }
}

function persist(s: Session) {
  try {
    localStorage.setItem(STORE + s.key, JSON.stringify(s));
  } catch {
    /* results stay in memory; export still works */
  }
}

function shuffledArms(n: number): number[] {
  const a = Array.from({ length: n }, (_, i) => i);
  for (let i = n - 1; i > 0; i--) {
    const j = randomSeed32() % (i + 1);
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

const scoreOf = (o: BotOutcome) => (o === "win" ? 1 : o === "loss" ? 0 : 0.5);

function forkName(): string | null {
  const v = new URLSearchParams(location.search).get("fork")?.trim() ?? "";
  return v === "" || v === "1" || v.toLowerCase() === "true" ? null : v;
}

function armNames(json: string, info: ForkInfo): string[] {
  const b = forkBattle(json, 0);
  try {
    const legal = legalChoices(b, info.botSide);
    return info.arms.map((arm) => {
      const c = legal.find((x) => x.input === arm.input);
      if (c?.kind === "move") return moveName(c.name);
      if (c?.kind === "switch") return `交代: ${speciesName(c.species)}`;
      return arm.input;
    });
  } finally {
    b.free();
  }
}

export function Fork() {
  const [poolJson, setPoolJson] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [fork, setFork] = useState<LoadedFork | null>(null);
  const [session, setSession] = useState<Session | null>(null);
  const [current, setCurrent] = useState<GameRecord | null>(null);
  const [paste, setPaste] = useState("");

  useEffect(() => {
    setLocale("ja", false);
    document.title = "NC2000 — 反実仮想フォーク対局";
    void (async () => {
      try {
        const [, pool] = await Promise.all([
          loadEngine(),
          fetchPool(),
          loadJaNames(fetchI18nJa),
        ]);
        setPoolJson(pool.poolJson);
        const name = forkName();
        if (name) await load(await fetchFork(name));
      } catch (e) {
        setError(String(e));
      }
    })();
  }, []);

  async function load(json: string) {
    try {
      const info = readFork(json);
      const doc = JSON.parse(json) as { opponent_team: unknown[] };
      const key = await sha256(json);
      const loaded: LoadedFork = {
        json,
        info,
        key,
        armNames: armNames(json, info),
        humanSets: doc.opponent_team,
      };
      const s: Session = restore(key, info.arms.length) ?? {
        key,
        arms: info.arms.length,
        sessionSeed: randomSeed32(),
        budget: searchProfile(info.info).iterations,
        block: [],
        games: [],
        revealed: false,
      };
      persist(s);
      setError(null);
      setFork(loaded);
      setSession(s);
    } catch (e) {
      setError(`フォーク定義を読み込めません: ${String(e)}`);
    }
  }

  function update(s: Session) {
    const next = { ...s, games: [...s.games] };
    persist(next);
    setSession(next);
    return next;
  }

  function startGame() {
    if (!fork || !session) return;
    const block = session.block.length > 0 ? session.block : shuffledArms(session.arms);
    const arm = block[0];
    const record: GameRecord = {
      game: session.games.length,
      arm,
      input: fork.info.arms[arm].input,
      label: fork.info.arms[arm].label,
      battleSeed: randomSeed32(),
      botSeed: randomSeed32(),
      outcome: null,
      forfeit: false,
      finalTurn: null,
      started: new Date().toISOString(),
      finished: null,
    };
    update({ ...session, block: block.slice(1), games: [...session.games, record] });
    setCurrent(record);
  }

  function finish(game: number, outcome: BotOutcome, finalTurn: number, forfeit: boolean) {
    setSession((s) => {
      if (!s) return s;
      const games = s.games.map((g) =>
        g.game === game && g.outcome === null
          ? { ...g, outcome, forfeit, finalTurn, finished: new Date().toISOString() }
          : g,
      );
      const next = { ...s, games };
      persist(next);
      return next;
    });
  }

  if (error && !fork) {
    return (
      <main class="fork">
        <p class="fork-error">{error}</p>
        {poolJson && <Loader paste={paste} setPaste={setPaste} onLoad={load} />}
      </main>
    );
  }
  if (!poolJson) {
    return (
      <div class="center-screen">
        <div class="loading-pulse">読み込み中…</div>
      </div>
    );
  }
  if (!fork || !session) {
    return (
      <main class="fork">
        <ForkIntro />
        <Loader paste={paste} setPaste={setPaste} onLoad={load} />
      </main>
    );
  }
  if (current) {
    return (
      <ForkGame
        key={current.game}
        fork={fork}
        record={current}
        poolJson={poolJson}
        budget={session.budget}
        onFinish={(o, turn) => finish(current.game, o, turn, false)}
        onForfeit={(turn) => {
          finish(current.game, "win", turn, true);
          setCurrent(null);
        }}
        onNext={startGame}
        onBack={() => setCurrent(null)}
      />
    );
  }
  return (
    <main class="fork">
      <ForkIntro />
      <ForkSummary
        fork={fork}
        session={session}
        onStart={startGame}
        onReveal={() => update({ ...session, revealed: true })}
        onClear={() => {
          if (!confirm("このフォークの記録をすべて消去しますか？")) return;
          const s: Session = {
            ...session,
            sessionSeed: randomSeed32(),
            block: [],
            games: [],
            revealed: false,
          };
          update(s);
        }}
      />
      <ArenaPanel
        info={fork.info}
        json={fork.json}
        poolJson={poolJson}
        armNames={fork.armNames}
        fileKey={fork.key}
      />
    </main>
  );
}

function ForkIntro() {
  return (
    <header class="fork-intro">
      <p class="fork-eyebrow">反実仮想フォーク対局</p>
      <h1>記録された局面から、bot と繰り返し対戦する</h1>
      <p>
        対戦記録の1局面から対局を再開し、bot の初手の候補ごとに勝率を比べます。比べ方は2つあります。あなたが元の対戦と同じ側を持って
        bot と対戦するか、bot 同士で対戦させるかです。どちらも 2手目以降は bot がラダーと同じ仕組みで考えます。
      </p>
      <p class="fork-muted">
        あなたとの対戦では、bot の初手は候補手のどれか1つに固定され、どれになるかは対局ごとに無作為に決まり、ターンが進むまで分かりません。途中でやめた対局は投了（bot
        の勝ち）として記録します。記録はこのブラウザにだけ保存されます。
      </p>
    </header>
  );
}

function Loader(props: {
  paste: string;
  setPaste: (v: string) => void;
  onLoad: (json: string) => Promise<void>;
}) {
  return (
    <section class="fork-panel">
      <h2>フォーク定義を読み込む</h2>
      <p class="fork-muted">
        <code>fork_bundle</code> で作った <code>nc2000-fork-v1</code> の JSON を選ぶか、貼り付けてください。
      </p>
      <input
        type="file"
        accept=".json,application/json"
        aria-label="フォーク定義ファイル"
        onChange={async (e) => {
          const file = (e.currentTarget as HTMLInputElement).files?.[0];
          if (file) await props.onLoad(await file.text());
        }}
      />
      <textarea
        rows={6}
        placeholder='{"schema": "nc2000-fork-v1", ...}'
        value={props.paste}
        onInput={(e) => props.setPaste((e.currentTarget as HTMLTextAreaElement).value)}
      />
      <button
        class="primary"
        disabled={props.paste.trim() === ""}
        onClick={() => void props.onLoad(props.paste)}
      >
        貼り付けた定義を読み込む
      </button>
    </section>
  );
}

function ForkSummary(props: {
  fork: LoadedFork;
  session: Session;
  onStart: () => void;
  onReveal: () => void;
  onClear: () => void;
}) {
  const { fork, session } = props;
  const info = fork.info;
  const done = session.games.filter((g) => g.outcome !== null);
  const humanWins = done.filter((g) => g.outcome === "loss").length;
  const ties = done.filter((g) => g.outcome === "tie").length;
  const perArm = info.arms.map((_, arm) => {
    const games = done.filter((g) => g.arm === arm);
    const wins = games.filter((g) => g.outcome === "win").length;
    const losses = games.filter((g) => g.outcome === "loss").length;
    const armTies = games.filter((g) => g.outcome === "tie").length;
    const score = games.reduce((a, g) => a + scoreOf(g.outcome!), 0);
    return {
      n: games.length,
      wins,
      losses,
      ties: armTies,
      forfeits: games.filter((g) => g.forfeit).length,
      rate: games.length ? score / games.length : null,
      ci: wilson(score, games.length),
    };
  });
  const base = perArm[0];

  function exportRows() {
    const rows = done.map((g) =>
      JSON.stringify({
        fork: info.label,
        turn: info.turn,
        trial: g.game,
        seed: session.sessionSeed,
        battle_seed: g.battleSeed,
        bot_seed: g.botSeed,
        action: g.input,
        arm_label: g.label,
        reply: "human",
        tail: "protocol",
        policy: `fork/${info.info}/protocol-vs-human`,
        iters: session.budget,
        foe_iters: 0,
        score: scoreOf(g.outcome!),
        outcome: g.outcome,
        final_turn: g.finalTurn,
        forfeit: g.forfeit,
        started: g.started,
        finished: g.finished,
      }),
    );
    download(`fork-${fork.key.slice(0, 12)}.jsonl`, rows.join("\n") + "\n");
  }

  return (
    <>
      <section class="fork-panel">
        <h2>{info.label || "フォーク"}</h2>
        <dl class="fork-facts">
          <dt>開始局面</dt>
          <dd>ターン {info.turn}</dd>
          <dt>あなた</dt>
          <dd>p{2 - info.botSide} 側（元の対戦と同じ）</dd>
          <dt>bot の情報</dt>
          <dd>
            {info.info === "blind"
              ? "あなたの構築を知らない（ラダーと同じ）"
              : "あなたの構築を知っている（オープンシート）"}
          </dd>
          <dt>対戦での bot の思考量</dt>
          <dd>1手あたり {session.budget.toLocaleString()} 回</dd>
          <dt>bot の初手の候補</dt>
          <dd>
            <ul class="fork-arms">
              {info.arms.map((a, i) => (
                <li key={i}>
                  {fork.armNames[i]}
                  {a.label && <span class="fork-arm-label">{a.label}</span>}
                </li>
              ))}
            </ul>
          </dd>
        </dl>
        <button class="primary fork-start" onClick={props.onStart}>
          {done.length === 0 ? "対局を始める" : "次の対局"}
        </button>
      </section>

      <section class="fork-panel" data-testid="human-results">
        <h2>あなたとの対戦結果</h2>
        <p>
          {done.length} 局終了 — あなたの {humanWins} 勝 {done.length - humanWins - ties} 敗
          {ties > 0 && ` ${ties} 分`}
        </p>
        {!session.revealed ? (
          <>
            <p class="fork-muted">
              候補ごとの結果は伏せてあります。対局中に意識しないよう、予定の局数を終えてから表示してください。
            </p>
            <button onClick={props.onReveal} disabled={done.length === 0}>
              候補ごとの結果を表示
            </button>
          </>
        ) : (
          <div class="fork-table-wrap">
            <table class="fork-table">
              <thead>
                <tr>
                  <th>bot の初手</th>
                  <th>局数</th>
                  <th>bot 勝</th>
                  <th>bot 負</th>
                  <th>分</th>
                  <th>bot 勝率 (95%)</th>
                  <th>{info.arms[0].label || "1番目"} との差</th>
                </tr>
              </thead>
              <tbody>
                {perArm.map((r, i) => {
                  const binary = r.ties === 0 && base.ties === 0;
                  const diff =
                    i > 0 && r.rate !== null && base.rate !== null ? r.rate - base.rate : null;
                  return (
                    <tr key={i}>
                      <td>
                        {fork.armNames[i]}
                        {info.arms[i].label && (
                          <span class="fork-arm-label">{info.arms[i].label}</span>
                        )}
                      </td>
                      <td>{r.n}</td>
                      <td>{r.wins}</td>
                      <td>{r.losses}</td>
                      <td>{r.ties}</td>
                      <td>
                        {r.rate === null
                          ? "—"
                          : `${pct(r.rate)} (${pct(r.ci[0])}–${pct(r.ci[1])})`}
                      </td>
                      <td>
                        {diff === null
                          ? "—"
                          : `${diff >= 0 ? "+" : ""}${pct(diff)}${
                              binary && r.n > 0 && base.n > 0
                                ? `  p=${fisherP(r.wins, r.losses, base.wins, base.losses).toPrecision(2)}`
                                : ""
                            }`}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            <p class="fork-muted">
              区間は Wilson の95%区間、p は Fisher の正確検定（両側）です。途中でやめた対局（
              {perArm.reduce((a, r) => a + r.forfeits, 0)} 局）は bot の勝ちに含みます。
            </p>
          </div>
        )}
        <div class="fork-actions">
          <button onClick={exportRows} disabled={done.length === 0}>
            結果を書き出す (JSONL)
          </button>
          <button class="ghost" onClick={props.onClear} disabled={session.games.length === 0}>
            記録を消去
          </button>
        </div>
        {session.revealed && done.length > 0 && (
          <ol class="fork-games">
            {done.map((g) => (
              <li key={g.game}>
                {fork.armNames[g.arm]} —{" "}
                {g.outcome === "loss" ? "あなたの勝ち" : g.outcome === "win" ? "bot の勝ち" : "引き分け"}
                {g.forfeit && "（投了）"}
                {g.finalTurn !== null && `・ターン ${g.finalTurn}`}
              </li>
            ))}
          </ol>
        )}
      </section>
    </>
  );
}

interface Request {
  needs: [boolean, boolean];
  picks: [string | null, string | null];
  committed: boolean;
}

function ForkGame(props: {
  fork: LoadedFork;
  record: GameRecord;
  poolJson: string;
  budget: number;
  onFinish: (outcome: BotOutcome, finalTurn: number) => void;
  onForfeit: (finalTurn: number) => void;
  onNext: () => void;
  onBack: () => void;
}) {
  const { info, json, humanSets } = props.fork;
  const BOT = info.botSide;
  const HUMAN = 1 - BOT;
  const blind = info.info === "blind";
  const [phase, setPhase] = useState<"init" | "battle" | "end">("init");
  const [view, setView] = useState<StateView | null>(null);
  const [log, setLog] = useState<LogEntry[]>([
    { kind: "turn", text: `ターン ${info.turn} から再開` },
  ]);
  const [humanChoices, setHumanChoices] = useState<Choice[] | null>(null);
  const [humanWaiting, setHumanWaiting] = useState(false);
  const [thinking, setThinking] = useState<{ done: number; budget: number } | null>(null);
  const [outcome, setOutcome] = useState<BotOutcome | null>(null);

  const battleRef = useRef<Battle | null>(null);
  const botRef = useRef<BotWorker | null>(null);
  const reqRef = useRef<Request | null>(null);
  const aliveRef = useRef(true);
  const armPendingRef = useRef(true);
  const narrator = useMemo(() => new Narrator(HUMAN), [HUMAN]);

  useEffect(() => {
    aliveRef.current = true;
    const bot = new BotWorker();
    botRef.current = bot;
    const battle = forkBattle(json, props.record.battleSeed);
    battleRef.current = battle;
    void bot
      .newFork(json, props.record.battleSeed, {
        poolJson: props.poolJson,
        side: BOT,
        seed: props.record.botSeed,
        mode: info.info,
      })
      .then(() => {
        if (!aliveRef.current) return;
        drain();
        startRequest();
      });
    return () => {
      aliveRef.current = false;
      bot.terminate();
      battle.free();
      battleRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function drain() {
    const battle = battleRef.current!;
    const lines = takeNewLog(battle);
    if (lines.length > 0) {
      const entries = narrator.render(lines);
      if (entries.length > 0) {
        setLog((prev) => [...prev, ...entries]);
        const speak = entries
          .filter((e) => e.kind !== "result")
          .map((e) => e.text)
          .join(" ");
        if (speak) announce(speak);
      }
    }
    setView(stateView(battle));
  }

  function startRequest() {
    if (!aliveRef.current) return;
    const battle = battleRef.current!;
    const needs = needsChoice(battle);
    if (!needs[0] && !needs[1]) {
      const v = stateView(battle);
      const o: BotOutcome =
        v.outcome === "tie" ? "tie" : v.outcome === `p${BOT + 1}` ? "win" : "loss";
      setOutcome(o);
      setPhase("end");
      setHumanChoices(null);
      announceAssertive(o === "loss" ? "あなたの勝ち" : o === "win" ? "bot の勝ち" : "引き分け");
      props.onFinish(o, v.turn);
      return;
    }
    const req: Request = { needs, picks: [null, null], committed: false };
    reqRef.current = req;
    setHumanWaiting(false);
    setPhase("battle");
    if (needs[HUMAN]) {
      const legal = legalChoices(battle, HUMAN);
      if (legal.length === 1) {
        req.picks[HUMAN] = legal[0].input;
        setHumanChoices(null);
      } else {
        setHumanChoices(legal);
      }
    } else {
      setHumanChoices(null);
    }
    if (needs[BOT]) decideBot(req);
    maybeCommit(req);
  }

  function decideBot(req: Request) {
    if (armPendingRef.current) {
      armPendingRef.current = false;
      req.picks[BOT] = props.record.input;
      return;
    }
    const legal = legalChoices(battleRef.current!, BOT);
    if (legal.length === 1) {
      req.picks[BOT] = legal[0].input;
      return;
    }
    void searchBot(req, legal);
  }

  async function searchBot(req: Request, legal: Choice[]) {
    const ponder = req.needs[HUMAN] && req.picks[HUMAN] === null;
    setThinking({ done: 0, budget: props.budget });
    const r = await botRef.current!.search(
      BOT,
      props.budget,
      randomSeed32(),
      ponder,
      (done, b) => {
        if (aliveRef.current && req === reqRef.current) setThinking({ done, budget: b });
      },
    );
    if (!aliveRef.current || req !== reqRef.current) return;
    setThinking(null);
    req.picks[BOT] = r.best ?? legal[0].input;
    maybeCommit(req);
  }

  function humanPick(input: string) {
    const req = reqRef.current;
    if (!req || req.committed || req.picks[HUMAN] !== null) return;
    req.picks[HUMAN] = input;
    setHumanChoices(null);
    setHumanWaiting(true);
    if (req.needs[BOT] && req.picks[BOT] === null) botRef.current!.flush();
    maybeCommit(req);
  }

  function maybeCommit(req: Request) {
    if (req.committed) return;
    for (const side of [0, 1]) if (req.needs[side] && req.picks[side] === null) return;
    req.committed = true;
    const picks: [number, string][] = [];
    for (const side of [0, 1]) if (req.needs[side]) picks.push([side, req.picks[side]!]);
    try {
      for (const [side, input] of picks) battleRef.current!.applyChoice(side, input);
    } catch (e) {
      console.error("applyChoice failed:", e, picks);
      return;
    }
    botRef.current!.apply(picks);
    setHumanWaiting(false);
    setThinking(null);
    drain();
    setTimeout(() => {
      if (aliveRef.current) startRequest();
    }, 0);
  }

  if (!view) {
    return (
      <div class="center-screen">
        <div class="loading-pulse">局面を準備中…</div>
      </div>
    );
  }

  const mine = view.sides[HUMAN];
  const foe = view.sides[BOT];
  const activeMine = mine.active !== null ? mine.party[mine.active] : null;
  const activeFoe = foe.active !== null ? foe.party[foe.active] : null;
  const initialItem = (species: string) => {
    const s = (humanSets as { species?: string }[]).find(
      (x) => toId(x.species ?? "") === toId(species),
    );
    return s ? sheetMon(s).item : null;
  };

  return (
    <main class="screen battle-screen fork-battle">
      <header class="battle-header">
        <span class="turn-label">
          フォーク #{props.record.game + 1}・ターン {view.turn}
        </span>
        {humanChoices && <ThinkChip thinking={thinking} />}
        {phase !== "end" && (
          <button
            class="ghost quit-btn"
            onClick={() => {
              if (confirm("この対局を投了しますか？ bot の勝ちとして記録されます。"))
                props.onForfeit(view.turn);
            }}
          >
            投了
          </button>
        )}
      </header>

      <div class="arena">
        {activeFoe && (
          <ActiveCard
            poke={blind ? { ...activeFoe, item: "?" } : activeFoe}
            mine={false}
            extra={`残り${foe.pokemonLeft}`}
            initialItem={null}
          />
        )}
        <FieldStrip
          weather={view.field.weather}
          pseudo={view.field.pseudoWeather}
          mineConds={mine.sideConditions}
          foeConds={foe.sideConditions}
        />
        {activeMine && (
          <ActiveCard poke={activeMine} mine={true} initialItem={initialItem(activeMine.species)} />
        )}
      </div>

      <LogPane log={log} />

      <section class="choice-panel" aria-label="あなたの行動">
        {phase === "end" && outcome ? (
          <div class="end-banner">
            <h2 class={`end-text ${outcome === "loss" ? "win" : "lose"}`}>
              {outcome === "loss" ? "あなたの勝ち" : outcome === "win" ? "bot の勝ち" : "引き分け"}
            </h2>
            <p class="fork-muted">
              この対局の bot の初手: {props.fork.armNames[props.record.arm]}
              {props.record.label && `（${props.record.label}）`}
            </p>
            <div class="end-actions">
              <button class="primary" onClick={props.onNext}>
                次の対局
              </button>
              <button class="ghost" onClick={props.onBack}>
                結果一覧へ
              </button>
            </div>
          </div>
        ) : humanChoices ? (
          <ChoiceButtons
            choices={humanChoices}
            onPick={humanPick}
            party={mine.party}
            sets={humanSets}
          />
        ) : (
          <ThinkingBar thinking={thinking} waiting={humanWaiting} />
        )}
      </section>
    </main>
  );
}
