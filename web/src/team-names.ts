import { locale } from "./i18n";

export interface TeamNameEntry {
  ja: string;
  en: string;
}

export const CATALOG_TEAM_NAMES: Record<string, TeamNameEntry> = {
  // Nash equilibrium
  "apex-c08-surgical-142": {
    ja: "雨パル・めざ虫ガラ・冷ビスタ55カビ",
    en: "Rain Cloyster · HP-Bug Marowak 55 Snorlax",
  },
  "red-v10-s1105-r2-s0142": {
    ja: "雨パル・地震ヘラ55カビ",
    en: "Rain Cloyster · EQ Heracross 55 Snorlax",
  },
  "apex-c04-bug-3atkstar-boomgen": {
    ja: "爆破パル・爆破ゲン・波乗りスタ55カビ",
    en: "Explosion Cloyster · Boom Gengar 55 Snorlax",
  },
  "red-v10-s1105-r1-s0148": {
    ja: "爆破パル・めざ闘ヘラ55カビ",
    en: "Explosion Cloyster · HP-Fight 55 Snorlax",
  },
  "sample-08": {
    ja: "サンダータンク (爆破ナッシー / Sample 08)",
    en: "Zapdos Tank (Boom Exeggutor / Sample 08)",
  },
  "rental-cban-6": {
    ja: "サンダータンク (粉ナッシー / レンタル)",
    en: "Zapdos Tank (Sleep Powder / Rental)",
  },
  "hc75-3rd-bean": {
    ja: "サンダータンク (分身ミル・身代わり / ビーン3位)",
    en: "Zapdos Tank (Double Team / Bean 3rd)",
  },

  // Evolved
  "red-v5-r1-s0097": {
    ja: "55バンギヘラ",
    en: "55 Tyranitar Heracross",
  },
  "deme-r1-00-s0030": {
    ja: "55バンギライコウ",
    en: "55 Tyranitar Raikou",
  },
  "red-v10-s1105-r2-s0155": {
    ja: "55カビミル受けループ",
    en: "55 Snorlax Miltank Stall",
  },
  "sib-r1-n4-red-v6-r1-s0064-s07": {
    ja: "ナンス晴れファイヤー",
    en: "Wobbuffet Sun Moltres",
  },

  // Classic & community
  "sample-07": {
    ja: "55サンダーケンタロス",
    en: "55 Zapdos Tauros",
  },
  "rental-cban-8": {
    ja: "モーダルバンギ",
    en: "Modal Tyranitar",
  },
  "sample-12": {
    ja: "55カビゴンブラッキー",
    en: "55 Snorlax Umbreon",
  },
  "sample-17-orig": {
    ja: "55ポリゴン2昆布",
    en: "55 Porygon2 Phazing",
  },
  "nb-sample-13-0": {
    ja: "55カビゴン電磁波",
    en: "55 Snorlax Thunder Wave",
  },
  "rental-cban-13": {
    ja: "ポニョ (55フーディンガラ)",
    en: "Ponyo (55 Alakazam Marowak)",
  },
  "mjj-2014-thdrcomb": {
    ja: "サンダー昆布",
    en: "Zapdos Phazing",
  },
  "sample-11": {
    ja: "55ライコウナッシー",
    en: "55 Raikou Exeggutor",
  },
  "sample-06": {
    ja: "キチカビ3 (55ライコウカビゴンWA)",
    en: "Kichikabi 3 (55 Raikou Snorlax WA)",
  },
  "mjj-2010-garap-a": {
    ja: "ガラプラス",
    en: "Gala-Lapras",
  },
  "mjj-2020-rraikou": {
    ja: "ソード＆シールド (55ライコウガラ)",
    en: "Sword & Shield (55 Raikou Marowak)",
  },
  "sample-02": {
    ja: "55ポリゴン2ルージュラ",
    en: "55 Porygon2 Jynx",
  },
  "hc75-top8-tsuru": {
    ja: "つる 晴れエンテイ",
    en: "Tsuru Sun Entei",
  },
  "sample-09": {
    ja: "55ニドキングドーブル",
    en: "55 Nidoking Smeargle",
  },
  "sample-22-orig": {
    ja: "55カイリューキングドラ",
    en: "55 Dragonite Kingdra",
  },
  "sample-14": {
    ja: "55カビゴン滅び偽装",
    en: "55 Snorlax Perish Trap",
  },
  "pb-100": {
    ja: "カビゴンSA+フォレトス (わんおー)",
    en: "55 Snorlax SA + Forretress (Wan-oh)",
  },
  "pb-85": {
    ja: "寝言カビSA最終版 (甘えるバトンブラッキー)",
    en: "55 Snorlax Sleep Talk SA (Charm BP Umbreon)",
  },
  "pb-62": {
    ja: "55地震寝言ピント♀カビSA (いばみがサンダー)",
    en: "55 Scope Lens EQ/DE Snorlax SA (SubSwagger Zapdos)",
  },
  "pb-6": {
    ja: "55サンダーリキーWA (しゃわ)",
    en: "55 Zapdos Machamp WA (Shawa)",
  },
  "pb-96": {
    ja: "55ヘラクロス+エレブーWA",
    en: "55 Heracross Electabuzz WA",
  },
  "pb-89": {
    ja: "55サイドンタンクWA",
    en: "55 Rhydon Miltank WA",
  },
  "pb-46": {
    ja: "55エンテイ・メガニウム・バンギTA",
    en: "55 Entei Meganium Tyranitar TA",
  },
  "pb-13": {
    ja: "芸術は爆発だ！ (55ポリ2+5大爆発)",
    en: "Art is an Explosion! (55 Porygon2 + 5 Boom)",
  },
};

export function teamDisplayName(id: string): string | null {
  const entry = CATALOG_TEAM_NAMES[id];
  if (!entry) return null;
  return locale() === "ja" ? entry.ja : entry.en;
}
