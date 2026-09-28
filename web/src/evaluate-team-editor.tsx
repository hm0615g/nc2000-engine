import { useId, useMemo, useRef, useState } from "preact/hooks";
import { itemName, moveName, speciesName } from "./i18n";
import { parsePsExport } from "./ps-import";

export interface EditorDex {
  species: Record<string, { name: string }>;
  moves: Record<string, { name: string }>;
  items: Record<string, { name: string }>;
}
interface EditorSet {
  species: string;
  level?: number;
  item?: string;
  moves?: string[];
  happiness?: number;
  ivs?: Record<string, number>;
  evs?: Record<string, number>;
  [key: string]: unknown;
}
interface Choice {
  value: string;
  label: string;
}
const normalize = (text: string) =>
  text
    .normalize("NFKC")
    .toLowerCase()
    .replace(/[ぁ-ゖ]/g, (char) =>
      String.fromCharCode(char.charCodeAt(0) + 0x60),
    );
const newMember = (): EditorSet => ({
  species: "",
  level: 50,
  item: "",
  moves: [],
});

function parseEditor(text: string): EditorSet[] | null {
  if (!text.trim()) return [newMember()];
  try {
    const parsed = /^\s*[\[{]/.test(text)
      ? JSON.parse(text)
      : parsePsExport(text);
    if (parsed.findings?.length) return null;
    const sets = Array.isArray(parsed) ? parsed : parsed.sets;
    if (
      !Array.isArray(sets) ||
      !sets.length ||
      sets.length > 6 ||
      sets.some(
        (s) =>
          !s ||
          typeof s !== "object" ||
          typeof s.species !== "string" ||
          (s.moves !== undefined &&
            (!Array.isArray(s.moves) ||
              s.moves.some((m: unknown) => typeof m !== "string") ||
              s.moves.length > 4)),
      )
    )
      return null;
    return sets;
  } catch {
    return null;
  }
}

function NameChoice({
  label,
  caption,
  value,
  choices,
  onChange,
  placeholder,
}: {
  label: string;
  caption: string;
  value: string;
  choices: Choice[];
  onChange: (value: string) => void;
  placeholder: string;
}) {
  const id = useId();
  const box = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [highlight, setHighlight] = useState(0);
  const [query, setQuery] = useState<string | null>(null);
  const display =
    choices.find((c) => normalize(c.value) === normalize(value))?.label ??
    value;
  const text = query ?? display;
  const matches = useMemo(() => {
    const term = normalize(text);
    return choices
      .filter(
        (c) =>
          normalize(c.label).includes(term) ||
          normalize(c.value).includes(term),
      )
      .slice(0, 8);
  }, [text, choices]);
  function pick(choice: Choice) {
    onChange(choice.value);
    setQuery(null);
    setOpen(false);
    setHighlight(0);
  }
  return (
    <div
      class="eval-name-choice"
      ref={box}
      onBlurCapture={(e) => {
        if (!box.current?.contains(e.relatedTarget as Node | null)) {
          setOpen(false);
          setQuery(null);
        }
      }}
    >
      <label htmlFor={id}>{caption}</label>
      <input
        id={id}
        aria-label={label}
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={open}
        aria-controls={`${id}-options`}
        aria-activedescendant={
          open && matches[highlight] ? `${id}-${highlight}` : undefined
        }
        autoComplete="off"
        placeholder={placeholder}
        value={text}
        onFocus={() => {
          setOpen(true);
          setHighlight(0);
        }}
        onInput={(e) => {
          const entered = e.currentTarget.value;
          setQuery(entered);
          setOpen(true);
          setHighlight(0);
          const exact = choices.find(
            (c) =>
              normalize(c.label) === normalize(entered) ||
              normalize(c.value) === normalize(entered),
          );
          onChange(exact?.value ?? entered);
        }}
        onKeyDown={(e) => {
          if (e.isComposing) return;
          if (e.key === "Escape") {
            setOpen(false);
            setQuery(null);
          }
          if (
            (e.key === "ArrowDown" || e.key === "ArrowUp") &&
            matches.length
          ) {
            e.preventDefault();
            setOpen(true);
            setHighlight(
              (n) =>
                (n + (e.key === "ArrowDown" ? 1 : matches.length - 1)) %
                matches.length,
            );
          }
          if (e.key === "Enter" && open && matches[highlight]) {
            e.preventDefault();
            pick(matches[highlight]);
          }
        }}
      />
      {open && (
        <div
          class="eval-name-options"
          id={`${id}-options`}
          role="listbox"
          aria-label={`${label}の候補`}
        >
          {matches.map((c, i) => (
            <button
              key={c.value}
              id={`${id}-${i}`}
              type="button"
              role="option"
              aria-selected={i === highlight}
              tabIndex={-1}
              onPointerDown={(e) => e.preventDefault()}
              onClick={() => pick(c)}
            >
              {c.label}
            </button>
          ))}
          {!matches.length && <p>候補がありません。名前を確かめてください。</p>}
        </div>
      )}
    </div>
  );
}

export function TeamEditor({
  text,
  onChange,
  dex,
  name,
}: {
  text: string;
  onChange: (text: string) => void;
  dex: EditorDex;
  name: string;
}) {
  const [active, setActive] = useState(0);
  const sets = useMemo(() => parseEditor(text), [text]);
  const choices = useMemo(() => {
    const list = (kind: keyof EditorDex, label: (value: string) => string) =>
      Object.values(dex[kind])
        .map((v) => ({ value: v.name, label: label(v.name) }))
        .sort((a, b) => a.label.localeCompare(b.label, "ja"));
    return {
      species: list("species", speciesName),
      moves: list("moves", moveName),
      items: [{ value: "", label: "なし" }, ...list("items", itemName)],
    };
  }, [dex]);
  if (!sets)
    return (
      <p class="eval-warning">
        読み込んだ内容を確認できません。「テキストから読み込む」で内容を直してください。
      </p>
    );
  const index = Math.min(active, sets.length - 1);
  const member = sets[index];
  const key = `${name}${index + 1}匹目`;
  const replace = (next: EditorSet[]) => onChange(JSON.stringify(next));
  const patch = (updates: Partial<EditorSet>) =>
    replace(sets.map((s, i) => (i === index ? { ...s, ...updates } : s)));
  const moves = member.moves ?? [];
  const dv = (stat: string) => Math.floor((member.ivs?.[stat] ?? 31) / 2);
  function setDv(stat: string, value: number) {
    const ivs = {
      hp: 30,
      atk: 30,
      def: 30,
      spa: 30,
      spd: 30,
      spe: 30,
      ...member.ivs,
      [stat]: value * 2,
    };
    ivs.spd = ivs.spa;
    ivs.hp =
      ((Math.floor(ivs.atk / 2) % 2) * 8 +
        (Math.floor(ivs.def / 2) % 2) * 4 +
        (Math.floor(ivs.spe / 2) % 2) * 2 +
        (Math.floor(ivs.spa / 2) % 2)) *
      2;
    patch({ ivs });
  }
  return (
    <div class="eval-team-editor">
      <div class="eval-members" aria-label={`${name}のメンバー`}>
        {sets.map((s, i) => (
          <button
            type="button"
            key={i}
            class={index === i ? "selected" : ""}
            aria-pressed={index === i}
            onClick={() => setActive(i)}
          >
            {i + 1}. {s.species ? speciesName(s.species) : "未入力"}
          </button>
        ))}
        {sets.length < 6 && (
          <button
            type="button"
            onClick={() => {
              setActive(sets.length);
              replace([...sets, newMember()]);
            }}
          >
            ＋ ポケモンを追加
          </button>
        )}
      </div>
      <div class="eval-member-form" key={key}>
        <NameChoice
          label={`${key}のポケモン`}
          caption="ポケモン"
          value={member.species}
          choices={choices.species}
          placeholder="例：カビゴン"
          onChange={(species) =>
            patch({
              species,
              ...(member.name === member.species ? { name: species } : {}),
            })
          }
        />
        <div class="eval-member-basics">
          <label>
            レベル
            <input
              aria-label={`${key}のレベル`}
              type="number"
              min="1"
              max="100"
              value={member.level ?? 55}
              onInput={(e) =>
                patch({
                  level: e.currentTarget.value
                    ? Number(e.currentTarget.value)
                    : 0,
                })
              }
            />
          </label>
          <NameChoice
            label={`${key}の持ち物`}
            caption="持ち物"
            value={member.item ?? ""}
            choices={choices.items}
            placeholder="なし／名前で検索"
            onChange={(item) => patch({ item })}
          />
        </div>
        <div class="eval-moves">
          {[0, 1, 2, 3].map((slot) => (
            <NameChoice
              key={slot}
              label={`${key}の技${slot + 1}`}
              caption={`技${slot + 1}`}
              value={moves[slot] ?? ""}
              choices={choices.moves}
              placeholder="技の名前で検索"
              onChange={(value) => {
                const next = [...moves];
                while (next.length < 4) next.push("");
                next[slot] = value;
                patch({ moves: next });
              }}
            />
          ))}
        </div>
        <details>
          <summary>個体値・育成状態など</summary>
          <p class="eval-muted">
            指定しない場合は、個体値・育成状態・なつき度を最大として計算します。めざめるパワーの種類を指定した場合は、個体値を自動で合わせます。
          </p>
          <div class="eval-detail-fields">
            {[
              ["atk", "こうげき"],
              ["def", "ぼうぎょ"],
              ["spe", "すばやさ"],
              ["spa", "とくしゅ"],
            ].map(([stat, label]) => (
              <label key={stat}>
                {label}の個体値
                <input
                  aria-label={`${key}の${label}の個体値`}
                  type="number"
                  min="0"
                  max="15"
                  value={dv(stat)}
                  onInput={(e) => setDv(stat, Number(e.currentTarget.value))}
                />
              </label>
            ))}
            <label>
              なつき度
              <input
                aria-label={`${key}のなつき度`}
                type="number"
                min="0"
                max="255"
                value={member.happiness ?? 255}
                onInput={(e) =>
                  patch({ happiness: Number(e.currentTarget.value) })
                }
              />
            </label>
            <label>
              育成状態
              <select
                aria-label={`${key}の育成状態`}
                value={
                  !member.evs ||
                  Object.values(member.evs).every((v) => v === 255)
                    ? "max"
                    : Object.values(member.evs).every((v) => v === 0)
                      ? "none"
                      : "custom"
                }
                onChange={(e) => {
                  const v = e.currentTarget.value === "max" ? 255 : 0;
                  patch({
                    evs: { hp: v, atk: v, def: v, spa: v, spd: v, spe: v },
                  });
                }}
              >
                <option value="max">十分に育成（最大）</option>
                <option value="none">育成前（0）</option>
                <option value="custom" disabled>
                  読み込んだ個別の設定
                </option>
              </select>
            </label>
          </div>
        </details>
        {sets.length > 1 && (
          <button
            type="button"
            class="ghost"
            onClick={() => {
              replace(sets.filter((_, i) => i !== index));
              setActive(Math.max(0, index - 1));
            }}
          >
            このポケモンを外す
          </button>
        )}
      </div>
    </div>
  );
}
