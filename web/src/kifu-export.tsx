import { useEffect, useState } from "preact/hooks";
import type { Battle } from "./engine";
import type { InfoMode } from "./info-mode";
import type { StateView } from "./types";
import { copyKifu, kifuUrl } from "./kifu-code";
import "./kifu.css";

export function KifuExport(props: { battle: Battle; mode: InfoMode; state: StateView }) {
  const [code, setCode] = useState("");
  const [notice, setNotice] = useState("");
  const [manual, setManual] = useState(false);
  useEffect(() => {
    setManual(false);
    setNotice("");
    try {
      setCode(props.battle.exportKifu(props.mode === "open", 1));
    } catch (error) {
      console.error("kifu export", error);
      setCode("");
      setNotice("この対戦の棋譜を準備できませんでした。");
    }
  }, [props.battle, props.mode, props.state]);

  async function copy() {
    const copied = await copyKifu(code);
    setManual(!copied);
    setNotice(copied ? "棋譜をコピーしました。" : "下の欄から棋譜をコピーできます。");
  }

  return <section class="kifu-tool kp-export-shell" aria-labelledby="kifu-export-title">
    <div class="kp-card">
      <h2 id="kifu-export-title">この対戦を残す・試し直す</h2>
      <p class="kp-muted">棋譜を残しておくと、あとで好きな場面から別の手を試せます。</p>
      <div class="kp-actions">
        <button class="primary" disabled={!code} onClick={() => void copy()}>棋譜をコピー</button>
        {code && <a class="kp-button" href={kifuUrl(code)} target="_blank" rel="noopener">別の手を試す<span class="sr-only">（新しいタブ）</span></a>}
      </div>
      <p class="kp-status" role="status">{notice}</p>
      {manual && <div class="kp-copy-fallback">
        <label for="kifu-copy-code">コピーする棋譜</label>
        <textarea id="kifu-copy-code" readOnly value={code} rows={4} onFocus={e => e.currentTarget.select()} />
        <p class="kp-muted">欄を選ぶと全文が選択されます。端末の「コピー」を使ってください。</p>
      </div>}
    </div>
  </section>;
}
