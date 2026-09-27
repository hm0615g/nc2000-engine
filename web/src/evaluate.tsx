import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { loadEngine, randomSeed32 } from "./engine";
import { fetchBeliefPool, fetchI18nJa, fetchNashArtifact } from "./data";
import { loadJaNames, setLocale, speciesName } from "./i18n";
import {
  appendPair,
  normalizeWeights,
  resultsCsv,
  sameConfig,
  summarize,
  type EvaluationConfig,
  type EvaluationRun,
  type EvaluationTeam,
  type WorkerResponse,
} from "./evaluate-core";
import {
  importDistribution,
  readOpponents,
  readTeam,
  sha256,
  type OpponentDraft,
} from "./evaluate-input";
import { loadRuns, saveRun } from "./evaluate-store";
import { searchProfile } from "./search-profile";
import "./evaluate.css";

function download(name: string, text: string, type = "application/json") {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
const percent = (v: number | null) =>
  v === null ? "—" : `${(v * 100).toFixed(1)}%`;
function Findings({ team }: { team: EvaluationTeam }) {
  return (
    <>
      {team.relaxed && (
        <p class="eval-warning">
          このパーティは選出の合計155制限を解除します。
        </p>
      )}
      {team.warnings.length > 0 ? (
        <ul class="eval-findings eval-warning">
          {team.warnings.map((w, i) => (
            <li key={i}>警告: {w}</li>
          ))}
        </ul>
      ) : (
        <p class="eval-muted">大会ルールの検証を通過しました。</p>
      )}
      {team.fixes.length > 0 && (
        <details>
          <summary>入力の補完・正規化（{team.fixes.length}件）</summary>
          <ul class="eval-findings">
            {team.fixes.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </details>
      )}
    </>
  );
}
function ResultTable({ run }: { run: EvaluationRun }) {
  return (
    <div class="eval-table-wrap">
      <table>
        <thead>
          <tr>
            {[
              "対戦相手",
              "試合",
              "勝",
              "負",
              "引分",
              "打切",
              "勝率",
              "平均得点",
              "95%区間",
            ].map((t) => (
              <th key={t}>{t}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {[null, ...run.config.opponents.map((e) => e.id)].map((id) => {
            const s = summarize(
              id === null
                ? run.pairs
                : run.pairs.filter((p) => p.opponent === id),
            );
            return (
              <tr key={id ?? "overall"}>
                <th>{id ?? "総合"}</th>
                <td>{s.games}</td>
                <td>{s.win}</td>
                <td>{s.loss}</td>
                <td>{s.tie}</td>
                <td>{s.cap}</td>
                <td>{percent(s.winRate)}</td>
                <td>{percent(s.mean)}</td>
                <td>{s.interval ? s.interval.map(percent).join("–") : "—"}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export function Evaluate() {
  const [ready, setReady] = useState(false);
  const [defaults, setDefaults] = useState<OpponentDraft[]>([]);
  const [entries, setEntries] = useState<OpponentDraft[]>([]);
  const [party, setParty] = useState("");
  const [iterations, setIterations] = useState(3000);
  const [games, setGames] = useState("32");
  const [belief, setBelief] = useState({ json: "", hash: "" });
  const [error, setError] = useState("");
  const [storageError, setStorageError] = useState("");
  const [run, setRun] = useState<EvaluationRun | null>(null);
  const [history, setHistory] = useState<EvaluationRun[]>([]);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState("");
  const [notice, setNotice] = useState("");
  const worker = useRef<Worker | null>(null);
  const currentRun = useRef<EvaluationRun | null>(null);
  const writes = useRef(Promise.resolve());
  const alive = useRef(true);
  const generation = useRef(0);

  function selectRun(saved: EvaluationRun) {
    currentRun.current = saved;
    setRun(saved);
    setParty(JSON.stringify(saved.config.player.sets, null, 2));
    setEntries(
      saved.config.opponents.map((e) => ({
        id: e.id,
        weight: String(e.weight),
        text: JSON.stringify(e.sets, null, 2),
      })),
    );
    setIterations(saved.config.iterations);
    setError("");
    setProgress("");
    setNotice("");
  }
  useEffect(() => {
    setLocale("ja");
    document.title = "NC2000 — パーティ強度測定";
    void (async () => {
      try {
        const [, nash, pool] = await Promise.all([
          loadEngine(),
          fetchNashArtifact(),
          fetchBeliefPool(),
          loadJaNames(fetchI18nJa),
        ]);
        const draft = importDistribution(nash);
        const hash = await sha256(pool.poolJson);
        if (!alive.current) return;
        setDefaults(draft);
        setEntries(draft);
        setBelief({ json: pool.poolJson, hash });
        try {
          const saved = await loadRuns();
          if (!alive.current) return;
          setHistory(saved);
          if (saved[0]) selectRun(saved[0]);
        } catch (e) {
          setStorageError(`端末内の保存を読み込めません: ${String(e)}`);
        }
        setReady(true);
      } catch (e) {
        setError(String(e));
      }
    })();
    return () => {
      alive.current = false;
      worker.current?.terminate();
    };
  }, []);

  const validation = useMemo(() => {
    if (!ready)
      return {
        player: null,
        opponents: null,
        playerError: "",
        opponentError: "",
      };
    let player: EvaluationTeam | null = null;
    let opponents: ReturnType<typeof readOpponents> | null = null;
    let playerError = "",
      opponentError = "";
    try {
      player = readTeam(party);
    } catch (e) {
      playerError = String(e).replace(/^Error: /, "");
    }
    try {
      opponents = readOpponents(entries);
    } catch (e) {
      opponentError = String(e).replace(/^Error: /, "");
    }
    return { player, opponents, playerError, opponentError };
  }, [ready, party, entries]);
  const testBudget =
    import.meta.env.MODE === "test"
      ? Number(import.meta.env.VITE_NC2000_TEST_BUDGET)
      : NaN;
  const budget =
    Number.isSafeInteger(testBudget) && testBudget > 0
      ? testBudget
      : iterations;
  const config: EvaluationConfig | null =
    validation.player && validation.opponents
      ? {
          build: __EVALUATOR_BUILD__,
          beliefHash: belief.hash,
          player: validation.player,
          opponents: validation.opponents,
          iterations: budget,
          c: searchProfile("blind").c,
          turnLimit: 500,
        }
      : null;
  const count = Number(games);
  const validCount =
    Number.isSafeInteger(count) && count > 0 && count % 2 === 0;
  const compatible = !!(config && run && sameConfig(config, run.config));
  let weights: { weight: number }[] = [];
  try {
    weights = normalizeWeights(
      entries.map((e) => ({
        weight: e.weight.trim() ? Number(e.weight) : NaN,
      })),
    );
  } catch {}

  function retain(next: EvaluationRun) {
    currentRun.current = next;
    setRun(next);
    setHistory((list) =>
      [next, ...list.filter((r) => r.id !== next.id)].sort((a, b) =>
        b.createdAt.localeCompare(a.createdAt),
      ),
    );
    writes.current = writes.current
      .then(() => saveRun(next))
      .then(() => setStorageError(""))
      .catch((e) =>
        setStorageError(
          `自動保存できません。JSON出力で結果を保存してください: ${String(e)}`,
        ),
      );
    return writes.current;
  }
  function stop(
    message = "停止しました。途中ペアは集計せず、再開時に同じseedでやり直します。",
  ) {
    generation.current++;
    worker.current?.terminate();
    worker.current = null;
    setRunning(false);
    setProgress("");
    setNotice(message);
  }
  async function launch(next: EvaluationRun) {
    if (worker.current) return;
    const launchGeneration = ++generation.current;
    setRunning(true);
    setError("");
    setNotice("");
    setProgress("WASM対戦を準備しています…");
    await retain(next);
    if (!alive.current || generation.current !== launchGeneration) return;
    try {
      const w = new Worker(new URL("./evaluate-worker.ts", import.meta.url), {
        type: "module",
      });
      worker.current = w;
      w.onerror = (event) => {
        setError(`対戦Worker: ${event.message}`);
        stop("エラーで停止しました。完了ペアの結果は保持しています。");
      };
      w.onmessage = (event: MessageEvent<WorkerResponse>) => {
        if (worker.current !== w) return;
        const msg = event.data;
        if (msg.type === "progress")
          setProgress(
            `ペア ${msg.pair + 1} / ${next.targetPairs} · ${msg.game}戦目 · ターン ${msg.turn} · P${msg.side + 1} 思考 ${msg.iterations.toLocaleString()} / ${next.config.iterations.toLocaleString()}`,
          );
        if (msg.type === "pair") {
          try {
            const updated = appendPair(currentRun.current!, msg.pair);
            void retain(updated).then(() => {
              if (worker.current === w) w.postMessage({ type: "ack" });
            });
          } catch (e) {
            setError(String(e));
            stop("結果の集計中に停止しました。");
          }
        }
        if (msg.type === "done") stop("指定した試合数の計測が完了しました。");
        if (msg.type === "error") {
          setError(msg.message);
          stop("エラーで停止しました。完了ペアの結果は保持しています。");
        }
      };
      w.postMessage({ type: "start", run: next, beliefJson: belief.json });
    } catch (e) {
      setError(String(e));
      stop("計測を開始できませんでした。");
    }
  }
  function newRun() {
    if (!config || !validCount || running) return;
    void launch({
      version: 1,
      id: crypto.randomUUID(),
      createdAt: new Date().toISOString(),
      config,
      seed: randomSeed32(),
      targetPairs: count / 2,
      pairs: [],
    });
  }
  function resume(add: boolean) {
    if (!run || !compatible || running || (add && !validCount)) return;
    void launch({
      ...run,
      targetPairs: add ? run.pairs.length + count / 2 : run.targetPairs,
    });
  }
  async function importFile(
    file: File | undefined,
    apply: (text: string) => void,
  ) {
    if (!file) return;
    try {
      apply(await file.text());
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }
  function edit(index: number, patch: Partial<OpponentDraft>) {
    setEntries((list) =>
      list.map((e, i) => (i === index ? { ...e, ...patch } : e)),
    );
  }
  function exportDistribution() {
    if (!validation.opponents) return;
    download(
      "opponent-distribution.json",
      JSON.stringify(
        {
          teams: validation.opponents.map(({ id, weight, sets }) => ({
            id,
            weight,
            sets,
          })),
        },
        null,
        2,
      ),
    );
  }

  return (
    <main class="evaluate">
      <header>
        <p class="eval-eyebrow">NC2000 / BLIND</p>
        <h1>パーティの強さを調べる</h1>
        <p>
          自分のパーティを、指定した相手分布と自動対戦。計算と保存はこの端末で行います。
        </p>
      </header>
      {error && (
        <p class="eval-error" role="alert">
          {error}
        </p>
      )}
      {storageError && (
        <p class="eval-warning" role="alert">
          {storageError}
        </p>
      )}
      {!ready && !error && <p role="status">対戦データを読み込んでいます…</p>}
      {history.length > 0 && (
        <section class="eval-panel">
          <label>
            保存済みの計測{" "}
            <select
              aria-label="保存済みの計測"
              disabled={running}
              value={run?.id}
              onChange={(e) => {
                const saved = history.find(
                  (r) => r.id === e.currentTarget.value,
                );
                if (saved) selectRun(saved);
              }}
            >
              {history.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.createdAt.slice(0, 16).replace("T", " ")} UTC ·{" "}
                  {r.pairs.length * 2}戦 ·{" "}
                  {r.config.iterations.toLocaleString()}反復
                </option>
              ))}
            </select>
          </label>
        </section>
      )}
      <fieldset disabled={!ready || running}>
        <div class="eval-columns">
          <section class="eval-panel">
            <h2>自分のパーティ</h2>
            <p>Showdownテキスト、またはパーティJSON</p>
            <textarea
              aria-label="自分のパーティ"
              value={party}
              onInput={(e) => setParty(e.currentTarget.value)}
              placeholder={
                "Snorlax @ Leftovers\nLevel: 55\n- Double-Edge\n- Curse\n- Rest\n- Sleep Talk"
              }
            />
            <label class="eval-file">
              パーティを読み込む
              <input
                aria-label="パーティファイル"
                type="file"
                accept=".txt,.json"
                onChange={(e) => {
                  void importFile(e.currentTarget.files?.[0], setParty);
                  e.currentTarget.value = "";
                }}
              />
            </label>
            <p class="eval-muted">
              大会ルール違反は警告を表示し、対戦に参加できます。
            </p>
            {party.trim() && validation.playerError && (
              <p class="eval-error" role="alert">
                {validation.playerError}
              </p>
            )}
            {validation.player && <Findings team={validation.player} />}
          </section>
          <section class="eval-panel">
            <h2>対戦相手の分布</h2>
            <p>初期値：ナッシュ混合。重みは自動で正規化します。</p>
            {entries.map((t, i) => (
              <div class="eval-entry" key={i}>
                <div class="eval-opponent">
                  <strong>{t.id || `相手${i + 1}`}</strong>
                  <input
                    aria-label={`${t.id || i + 1} の重み`}
                    type="number"
                    min="0"
                    step="any"
                    value={t.weight}
                    onInput={(e) => edit(i, { weight: e.currentTarget.value })}
                  />
                  <span>{weights[i] ? percent(weights[i].weight) : "—"}</span>
                </div>
                {validation.opponents?.[i] && (
                  <p class="eval-roster">
                    {(
                      validation.opponents[i].sets as {
                        species: string;
                        level: number;
                      }[]
                    )
                      .map((m) => `${speciesName(m.species)} ${m.level}`)
                      .join(" / ")}
                  </p>
                )}
                <details>
                  <summary>{t.id || "相手"} の構成を編集</summary>
                  <label>
                    名前{" "}
                    <input
                      aria-label={`相手${i + 1}の名前`}
                      value={t.id}
                      onInput={(e) => edit(i, { id: e.currentTarget.value })}
                    />
                  </label>
                  <textarea
                    aria-label={`相手${i + 1}のパーティ`}
                    value={t.text}
                    onInput={(e) => edit(i, { text: e.currentTarget.value })}
                  />
                  <label class="eval-file">
                    パーティを置換
                    <input
                      aria-label={`相手${i + 1}のファイル`}
                      type="file"
                      accept=".txt,.json"
                      onChange={(e) => {
                        void importFile(e.currentTarget.files?.[0], (text) =>
                          edit(i, { text }),
                        );
                        e.currentTarget.value = "";
                      }}
                    />
                  </label>
                  <button
                    onClick={() =>
                      setEntries(entries.filter((_, j) => i !== j))
                    }
                  >
                    この相手を削除
                  </button>
                </details>
                {validation.opponents?.[i] &&
                  validation.opponents[i].warnings.length > 0 && (
                    <Findings team={validation.opponents[i]} />
                  )}
                {validation.opponents?.[i] &&
                  !validation.opponents[i].warnings.length &&
                  validation.opponents[i].fixes.length > 0 && (
                    <details>
                      <summary>
                        入力の補完・正規化（
                        {validation.opponents[i].fixes.length}件）
                      </summary>
                      <ul class="eval-findings">
                        {validation.opponents[i].fixes.map((f, j) => (
                          <li key={j}>{f}</li>
                        ))}
                      </ul>
                    </details>
                  )}
              </div>
            ))}
            {validation.opponentError && (
              <p class="eval-error" role="alert">
                {validation.opponentError}
              </p>
            )}
            <div class="eval-actions">
              <button
                onClick={() => {
                  let n = entries.length + 1;
                  while (entries.some((e) => e.id === `custom-${n}`)) n++;
                  setEntries([
                    ...entries,
                    { id: `custom-${n}`, weight: "1", text: "" },
                  ]);
                }}
              >
                相手を追加
              </button>
              <button onClick={() => setEntries(defaults)}>
                ナッシュ混合に戻す
              </button>
            </div>
            <div class="eval-actions">
              <label class="eval-file">
                分布JSONを読み込む
                <input
                  aria-label="分布JSONファイル"
                  type="file"
                  accept=".json"
                  onChange={(e) => {
                    void importFile(e.currentTarget.files?.[0], (text) =>
                      setEntries(importDistribution(text)),
                    );
                    e.currentTarget.value = "";
                  }}
                />
              </label>
              <button
                disabled={!validation.opponents}
                onClick={exportDistribution}
              >
                分布JSONを出力
              </button>
            </div>
            <p class="eval-muted">
              分布は出場抽選だけに使用。AIの相手推測には配布済みbelief
              poolを使います。
            </p>
          </section>
        </div>
        <section class="eval-panel">
          <h2>計測設定</h2>
          <div class="eval-actions">
            <label>
              試合数{" "}
              <input
                aria-label="試合数"
                type="number"
                value={games}
                min={2}
                step={2}
                onInput={(e) => setGames(e.currentTarget.value)}
              />
            </label>
            <label>
              1判断の探索量{" "}
              <select
                aria-label="探索量"
                value={iterations}
                onChange={(e) => setIterations(Number(e.currentTarget.value))}
              >
                {[3000, 10000, 27000].map((n) => (
                  <option key={n} value={n}>
                    {n.toLocaleString()}
                  </option>
                ))}
                {![3000, 10000, 27000].includes(iterations) && (
                  <option value={iterations}>
                    {iterations.toLocaleString()}（保存値）
                  </option>
                )}
              </select>
            </label>
          </div>
          {!validCount && (
            <p class="eval-error">試合数は2以上の偶数にしてください。</p>
          )}
          {Number.isFinite(testBudget) && (
            <p class="eval-warning">
              テストビルド：実際の探索量は{budget}反復です。
            </p>
          )}
          <p class="eval-muted">
            2戦ずつ先後を入れ替え、両側に同じ探索量を使います。追加計測では指定した試合数を加算します。
          </p>
        </section>
      </fieldset>
      <section class="eval-panel">
        <h2>計測と結果</h2>
        <div class="eval-actions">
          <button
            class="eval-start"
            disabled={!config || !validCount || running}
            onClick={newRun}
          >
            {run ? "別の計測を開始" : "計測を開始"}
          </button>
          {run && (
            <>
              <button
                disabled={
                  !compatible || running || run.pairs.length >= run.targetPairs
                }
                onClick={() => resume(false)}
              >
                再開
              </button>
              <button
                disabled={!compatible || running || !validCount}
                onClick={() => resume(true)}
              >
                追加計測
              </button>
            </>
          )}
          {running && <button onClick={() => stop()}>停止</button>}
        </div>
        {run && !compatible && ready && (
          <p class="eval-warning">
            入力・探索設定・ビルドが保存時と異なるため、この結果への追加はできません。現在の条件で別の計測を開始できます。
          </p>
        )}
        {run && (
          <p>
            {run.pairs.length * 2} / {run.targetPairs * 2}戦完了 ·{" "}
            {run.config.iterations.toLocaleString()}反復
          </p>
        )}
        {run && (
          <progress
            aria-label="完了した試合"
            max={run.targetPairs}
            value={run.pairs.length}
          />
        )}
        <p class="eval-progress" role="status">
          {progress || notice}
        </p>
        {run ? (
          <>
            <ResultTable run={run} />
            <div class="eval-actions">
              <button
                onClick={() =>
                  download(
                    `evaluation-${run.id}.json`,
                    JSON.stringify(run, null, 2),
                  )
                }
              >
                結果JSONを出力
              </button>
              <button
                onClick={() =>
                  download(
                    `evaluation-${run.id}.csv`,
                    resultsCsv(run),
                    "text/csv;charset=utf-8",
                  )
                }
              >
                結果CSVを出力
              </button>
            </div>
            <details>
              <summary>この結果の条件と警告</summary>
              <p>
                開始: {run.createdAt} · seed: {run.seed}
              </p>
              <p class="eval-fingerprint">
                ビルド: {run.config.build}
                <br />
                belief pool: {run.config.beliefHash}
              </p>
              <h3>自分のパーティ</h3>
              <Findings team={run.config.player} />
              {run.config.opponents.map((t) => (
                <div key={t.id}>
                  <h3>
                    {t.id} · {percent(t.weight)}
                  </h3>
                  <Findings team={t} />
                </div>
              ))}
            </details>
          </>
        ) : (
          <p class="eval-empty">
            パーティを入力して計測を開始すると、総合成績と相手別成績がここに表示されます。
          </p>
        )}
        <p class="eval-muted">
          勝率＝勝ち数／試合数。平均得点は勝ち1・負け0・引き分け／500ターン打ち切り0.5。95%区間は独立した対戦ペアの平均から正規近似で計算します（2ペア未満は非表示）。少数試行・途中経過・追加計測の区間は参考値です。
        </p>
        <p class="eval-muted">
          このblind
          AIが指定分布に対して出す成績を測定します。完了した2戦ペアごとに自動保存します。ページを閉じた間は計算しません。
        </p>
      </section>
    </main>
  );
}
