// Import the serious-team subset of 魔人島「俺のパーティ軍団」 into the team
// inventory (docs/TEAM-POOL-REBUILD-PLAN.md step 1).
//
//   node tools/import-majinjima.js [--fetch]
//
// --fetch  downloads the index and every linked page into
//          tmp/majinjima-cache/ (raw EUC-JP bytes, gitignored) before parsing;
//          without it the cache must already exist.
//
// Output: data/team-inventory-v1/sources/majinjima.json — per index entry
// the exact table rows, the decoded hidden `C` field, the mapped + validated
// engine sets, every deviation/assumption, and a content fingerprint. No
// article prose or battle logs are copied.
//
// The index row colour is the author's own genre classification; this takes
// the 「かなりガチなパーティ」(#ffaaaa) rows only. An entry's `(一撃)` summary
// marks the OHKO-allowed rule, which the format rejects: those are recorded
// with an eligibility decision, not dropped.
//
// Page data are untrusted input: nothing here executes page content.
'use strict';
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const REPO = path.join(__dirname, '..');
const BASE = 'https://majinjima.ma-jide.com/party/';
const CACHE = path.join(REPO, 'tmp', 'majinjima-cache');
const OUT = path.join(REPO, 'data', 'team-inventory-v1', 'sources', 'majinjima.json');
const wasm = require(path.join(REPO, 'crates/wasm/pkg-node/nc2000_wasm.js'));
const i18n = JSON.parse(fs.readFileSync(path.join(REPO, 'data/i18n-ja.json'), 'utf8'));
const dexJson = JSON.parse(fs.readFileSync(path.join(REPO, 'data/gen2stadium2.json'), 'utf8'));

const SERIOUS = '#ffaaaa';

// ------------------------------------------------------------------ fetch
async function fetchAll() {
	fs.mkdirSync(CACHE, { recursive: true });
	const get = async rel => {
		const res = await fetch(BASE + rel);
		if (!res.ok) throw new Error(`${rel}: HTTP ${res.status}`);
		const buf = Buffer.from(await res.arrayBuffer());
		const file = path.join(CACHE, rel);
		fs.mkdirSync(path.dirname(file), { recursive: true });
		fs.writeFileSync(file, buf);
		await new Promise(r => setTimeout(r, 1000));
		return buf;
	};
	const index = await get('index.htm');
	const pages = new Set(parseIndex(decode(index)).map(e => e.page));
	for (const p of pages) await get(p);
}

// ------------------------------------------------------------------ text
const decode = buf => new TextDecoder('euc-jp').decode(buf);
const stripTags = s => s.replace(/<[^>]+>/g, '').replace(/&nbsp;/g, ' ').trim();

/** Cloudflare injects a per-request token script; the fingerprint must not
 * change when only that does. */
function contentFingerprint(html) {
	const normalized = html.replace(/<script[\s\S]*?<\/script>/gi, '').replace(/\s+/g, ' ');
	return crypto.createHash('sha256').update(normalized).digest('hex');
}

// Pages spell the prolonged-sound mark with assorted dashes and pad cells.
function norm(s) {
	return s.replace(/[−―—–‐‑－ｰ-]/g, 'ー').replace(/[\s　]+/g, '').normalize('NFKC');
}

// ------------------------------------------------------------------ index
function parseIndex(html) {
	const out = [];
	for (const row of html.split(/<tr align="center">/i).slice(1)) {
		const cell = /<td bgcolor="(#[0-9a-f]{6})">([\s\S]*?)<\/td>/i.exec(row);
		if (!cell || cell[1].toLowerCase() !== SERIOUS) continue;
		const link = /<a Href="([^"]+)">([\s\S]*?)<\/a>/i.exec(cell[2]);
		if (!link) continue; // the genre legend row
		const tds = [...row.matchAll(/<td[^>]*>([\s\S]*?)<\/td>/gi)].map(m => stripTags(m[1]));
		const [page, anchor] = link[1].split('#');
		out.push({
			href: link[1],
			page,
			anchor: anchor || null,
			name: stripTags(link[2]),
			members: tds[1].replace(/\s+/g, ' '),
			logCount: tds[2],
			summary: tds[3],
			ohkoRule: tds[3].includes('(一撃)'),
		});
	}
	return out;
}

// ------------------------------------------------------------------ page
/** Team tables in page order, each with its section heading/anchor and the
 * hidden `C` field of the form that immediately follows it (if any). */
function parsePage(html) {
	const tables = [];
	const re = /<table[^>]*>([\s\S]*?)<\/table>/gi;
	let m;
	while ((m = re.exec(html))) {
		const rows = [];
		for (const tr of m[1].split(/<tr[^>]*>/i).slice(1)) {
			const cells = [...tr.matchAll(/<td[^>]*>([\s\S]*?)(?=<td|<\/tr|$)/gi)]
				.map(c => stripTags(c[1].replace(/<\/td>/gi, '')));
			const nonEmpty = cells.filter(c => c !== '');
			if (nonEmpty.length >= 7 && /^\d{2}$/.test(nonEmpty[0])) rows.push(nonEmpty);
		}
		if (rows.length !== 6) continue;
		const before = html.slice(0, m.index);
		const heads = [...before.matchAll(/<h[12][^>]*>([\s\S]*?)<\/h[12]>/gi)];
		const anchors = [...before.matchAll(/<a name="?([^" >]+)"?/gi)];
		const after = html.slice(re.lastIndex, re.lastIndex + 3000);
		const nextTable = after.search(/<table/i);
		const c = /name="C" type="HIDDEN" value="([^"]+)"/i.exec(nextTable < 0 ? after : after.slice(0, nextTable));
		tables.push({
			heading: heads.length ? stripTags(heads[heads.length - 1][1]) : null,
			anchor: anchors.length ? anchors[anchors.length - 1][1] : null,
			rows,
			cField: c ? c[1] : null,
		});
	}
	return tables;
}

/** The `C` field is psense's rental-utility encoding, one record per mon:
 * dex, level, 4 DV hex digits (Atk Def Spe Spc), 5 stat-exp values in
 * sqrt/4 units (HP Atk Def Spe Spc; 63 = maximum), 4 move numbers + 1,
 * 4 PP, 4 PP-up counts, max HP, an unused 0, gen-2 item index + 1. */
function decodeC(c) {
	const recs = c.split('_x_');
	if (recs[0] !== '1') throw new Error(`C field prefix ${recs[0]}`);
	return recs.slice(1).map(r => {
		const t = r.split('_');
		if (t.length !== 26) throw new Error(`C record has ${t.length} fields`);
		const n = i => parseInt(t[i], 10);
		return {
			dex: n(0), level: n(1),
			dvs: { atk: parseInt(t[2], 16), def: parseInt(t[3], 16), spe: parseInt(t[4], 16), spc: parseInt(t[5], 16) },
			statExp: { hp: n(6), atk: n(7), def: n(8), spe: n(9), spc: n(10) },
			moveNums: [n(11), n(12), n(13), n(14)].map(x => x - 1).filter(x => x > 0),
			pp: [n(15), n(16), n(17), n(18)],
			ppUps: [n(19), n(20), n(21), n(22)],
			maxHp: n(23),
			itemCode: n(25),
		};
	});
}

// ------------------------------------------------------------------ names
const byNum = kind => {
	const m = new Map();
	for (const [id, e] of Object.entries(dexJson[kind])) if (e.num > 0 && !m.has(e.num)) m.set(e.num, id);
	return m;
};
const speciesByNum = byNum('species');
const movesByNum = new Map();
for (const [id, e] of Object.entries(dexJson.moves)) {
	if (e.num > 0 && !id.startsWith('hiddenpower')) movesByNum.set(e.num, id);
}
movesByNum.set(237, 'hiddenpower');
const invert = table => {
	const m = new Map();
	for (const [id, jp] of Object.entries(table)) m.set(norm(jp), id);
	return m;
};
const speciesByJp = invert(i18n.species);
const movesByJp = invert(i18n.moves);
const itemsByJp = invert(i18n.items);

// Kanji/abbreviated spellings the site uses that the game's kana table
// lacks. Only needed where no `C` field supplies the number.
const MOVE_ALIASES = {
	'10万ボトル': 'thunderbolt', // chourou: source typo of 10万ボルト
	'冷凍ビール': 'icebeam', // recurring joke spelling of 冷凍ビーム
	'アルコール': 'encore', // recurring joke spelling of アンコール
	'大文学': 'fireblast', // garatank: typo of 大文字
	'バントタッチ': 'batonpass', // gyk_raikou: typo of バトンタッチ
	'呪い': 'curse', '堪える': 'endure', '滅びの唄': 'perishsong', '炎のパンチ': 'firepunch',
};
const ITEM_ALIASES = {
	'食べ残し': 'leftovers', '奇跡の実': 'miracleberry', '薄荷の実': 'mintberry',
	'麻痺治しの実': 'przcureberry', '黄金の実': 'goldberry', '光の粉': 'brightpowder',
	'太いホネ': 'thickclub', '破壊の遺伝子': 'berserkgene', 'ピントレンズ': 'scopelens',
	'先制のツメ': 'quickclaw', '水玉リボン': 'polkadotbow', 'ピンクのリボン': 'pinkbow',
	'木の実ジュース': 'berryjuice', '黒帯': 'blackbelt', '銀の粉': 'silverpowder',
	'曲がったスプーン': 'twistedspoon', '神秘の雫': 'mysticwater', '磁石': 'magnet',
	'ヒントレンズ': 'scopelens', // chourou: source typo of ピントレンズ
};
// Where the visible table and the hidden encoded field disagree, the choice
// is made here, per table, with its reason; an unlisted disagreement fails
// the import. Keys are `<page>#<team-table index>`.
const RESOLUTIONS = {
	'2012/goinkyo.htm#0': {
		moves: { alakazam: { 1: 'toxic' } },
		reason: 'table and prose name Toxic as the settled sub-move (Light Screen was a trial); the encoded field is stale',
	},
	'2022/ukekent.htm#0': {
		dvsFromTable: ['venusaur'],
		reason: 'table publishes Atk DV 1 (female); encoded field has 3 (male); prose is silent — the published table is taken',
	},
	'2023/1geki.htm#0': {
		dvsFromTable: ['skarmory'],
		reason: 'table publishes FFFF, encoded 7FFF; entry is OHKO-rule and ineligible either way',
	},
};

const HP_TYPES = ['fighting', 'flying', 'poison', 'ground', 'rock', 'bug', 'ghost', 'steel',
	'fire', 'water', 'grass', 'electric', 'psychic', 'ice', 'dragon', 'dark'];
const HP_JP = { 蟲: 'bug', 虫: 'bug', 氷: 'ice', 飛: 'flying', 飛行: 'flying', 格: 'fighting', 格闘: 'fighting',
	岩: 'rock', 草: 'grass', 水: 'water', 炎: 'fire', 悪: 'dark', 霊: 'ghost', ゴースト: 'ghost', 地面: 'ground', 電気: 'electric' };
const hpType = (atk, def) => HP_TYPES[(4 * (atk % 4) + (def % 4)) % 16];

/** Visible move cell → PS id plus, for Hidden Power, the type the cell names. */
function moveFromText(text, learned) {
	const t = norm(text);
	const hp = /^(?:めざパ|めざめるパワー|目覚めるパワー)(.*)$/.exec(t);
	if (hp) return { id: 'hiddenpower', hpNamed: hp[1] ? HP_JP[hp[1]] || `?${hp[1]}` : null };
	if (MOVE_ALIASES[t]) return { id: MOVE_ALIASES[t], alias: t };
	if (learned.has(t)) return { id: learned.get(t) };
	if (movesByJp.has(t)) return { id: movesByJp.get(t) };
	return { id: null };
}

function itemFromText(text) {
	const t = norm(text);
	return ITEM_ALIASES[t] || itemsByJp.get(t) || null;
}

// ------------------------------------------------------------------ build
function buildMon(row, rec, learned, notes, resolution, conflicts) {
	// row: [level, species, m1..m4, item, dvText] (the image cell is empty)
	const [lvText, spText, ...rest] = row;
	const hasDv = /^[0-9A-F]{4}/i.test(rest[rest.length - 1]);
	const dvText = hasDv ? rest[rest.length - 1] : null;
	const item = rest[rest.length - (hasDv ? 2 : 1)];
	const moveCells = rest.slice(0, rest.length - (hasDv ? 2 : 1));
	const level = parseInt(lvText, 10);
	const speciesId = rec ? speciesByNum.get(rec.dex) : speciesByJp.get(norm(spText));
	const visibleSpecies = speciesByJp.get(norm(spText)) || null;
	if (rec && visibleSpecies && visibleSpecies !== speciesId) notes.push(`table species ${spText} != encoded dex ${rec.dex}`);
	if (!speciesId) throw new Error(`unmapped species ${spText}`);
	if (rec && rec.level !== level) notes.push(`${speciesId}: table level ${level} != encoded ${rec.level}`);

	const dvHex = /^[0-9A-F]{4}/i.exec(dvText || '');
	const tableDvs = dvHex ? (h => ({ atk: parseInt(h[0], 16), def: parseInt(h[1], 16), spe: parseInt(h[2], 16), spc: parseInt(h[3], 16) }))(dvHex[0].toUpperCase()) : null;
	let dvs;
	if (rec) dvs = rec.dvs;
	else if (tableDvs) dvs = tableDvs;
	else {
		dvs = { atk: 15, def: 15, spe: 15, spc: 15 };
		notes.push(`${speciesId}: no DVs given; assumed FFFF`);
	}
	const hex = d => [d.atk, d.def, d.spe, d.spc].map(x => x.toString(16).toUpperCase()).join('');
	if (rec && tableDvs && hex(tableDvs) !== hex(rec.dvs)) {
		if (resolution && (resolution.dvsFromTable || []).includes(speciesId)) {
			dvs = tableDvs;
			notes.push(`${speciesId}: table DVs ${hex(tableDvs)} taken over encoded ${hex(rec.dvs)} (${resolution.reason})`);
		} else conflicts.push(`${speciesId}: table DVs ${hex(tableDvs)} != encoded ${hex(rec.dvs)}`);
	}
	if (dvText && dvHex && dvText.length > 4) notes.push(`${speciesId}: DV cell annotation "${dvText.slice(4)}"`);

	const moves = [];
	const visible = moveCells.map(c => moveFromText(c, learned));
	if (rec) {
		rec.moveNums.forEach((num, i) => {
			let id = movesByNum.get(num);
			if (!id) throw new Error(`unknown move num ${num}`);
			const v = visible[i];
			const chosen = resolution && resolution.moves && resolution.moves[speciesId] && resolution.moves[speciesId][i];
			if (chosen) {
				notes.push(`${speciesId}: slot ${i + 1} ${chosen} (table) over encoded ${id} (${resolution.reason})`);
				id = chosen;
			} else if (v && v.id && v.id !== id && !(v.id === 'hiddenpower' && id === 'hiddenpower')) {
				conflicts.push(`${speciesId}: table move "${moveCells[i]}" != encoded ${id}`);
			}
			if (v && !v.id && moveCells[i]) learned.set(norm(moveCells[i]), id);
			if (v && v.alias) notes.push(`${speciesId}: "${moveCells[i]}" read as ${id} (encoded)`);
			moves.push(id);
		});
		const lowPp = rec.ppUps.map((u, i) => [u, i]).filter(([u, i]) => u < 3 && i < rec.moveNums.length);
		for (const [u, i] of lowPp) notes.push(`${speciesId}: ${moves[i]} has ${u} PP Ups (engine plays max PP)`);
	} else {
		visible.forEach((v, i) => {
			if (!v.id) throw new Error(`unmapped move "${moveCells[i]}"`);
			if (v.alias) notes.push(`${speciesId}: "${moveCells[i]}" read as ${v.id} (spelling)`);
			moves.push(v.id);
		});
	}
	const hpIdx = moves.indexOf('hiddenpower');
	if (hpIdx >= 0) {
		const t = hpType(dvs.atk, dvs.def);
		const named = visible[hpIdx] && visible[hpIdx].hpNamed;
		if (named && named !== t) notes.push(`${speciesId}: table names Hidden Power ${named}, DVs give ${t}`);
		moves[hpIdx] = 'hiddenpower' + t;
	}

	const itemId = itemFromText(item);
	if (!itemId) throw new Error(`unmapped item ${item}`);
	if (norm(item) === 'ヒントレンズ') notes.push(`${speciesId}: item "${item}" read as Scope Lens (spelling)`);

	const statExp = rec ? rec.statExp : { hp: 63, atk: 63, def: 63, spe: 63, spc: 63 };
	if (!rec) notes.push(`${speciesId}: stat exp not published; assumed maximum`);
	const ev = v => (v >= 63 ? 255 : v * 4);
	const iv = d => (d === 15 ? 31 : d * 2);
	const set = {
		name: '', species: dexJson.species[speciesId].name, item: dexJson.items[itemId].name,
		ability: 'No Ability', moves, nature: 'Serious', level,
		evs: { hp: ev(statExp.hp), atk: ev(statExp.atk), def: ev(statExp.def), spa: ev(statExp.spc), spd: ev(statExp.spc), spe: ev(statExp.spe) },
		ivs: { hp: 31, atk: iv(dvs.atk), def: iv(dvs.def), spa: iv(dvs.spc), spd: iv(dvs.spc), spe: iv(dvs.spe) },
	};
	// Happiness is never published; it only matters for Return/Frustration.
	if (moves.includes('frustration')) {
		set.happiness = 0;
		notes.push(`${speciesId}: happiness not published; Frustration assumed at 0`);
	} else if (moves.includes('return')) {
		set.happiness = 255;
		notes.push(`${speciesId}: happiness not published; Return assumed at 255`);
	}
	return { set, itemCode: rec ? rec.itemCode : null, maxHp: rec ? rec.maxHp : null };
}

function slugOf(entry, tableIdx, pageTables) {
	const base = 'mjj-' + entry.page.replace(/\.htm$/, '').replace(/\//g, '-').replace(/_/g, '');
	if (entry.anchor) return `${base}-${entry.anchor}`;
	return pageTables.length > 1 ? `${base}-${String.fromCharCode(97 + tableIdx)}` : base;
}

async function main() {
	if (process.argv.includes('--fetch')) await fetchAll();
	const indexBuf = fs.readFileSync(path.join(CACHE, 'index.htm'));
	const indexHtml = decode(indexBuf);
	const entries = parseIndex(indexHtml);

	const dex = new wasm.Dex();
	const validator = new wasm.Validator(dex);
	const learned = new Map(); // visible move spelling → id, learned from encoded pages
	const pages = new Map();
	for (const p of new Set(entries.map(e => e.page))) {
		const buf = fs.readFileSync(path.join(CACHE, p));
		const html = decode(buf);
		pages.set(p, { html, tables: parsePage(html), fingerprint: contentFingerprint(html) });
	}
	// Encoded tables first, so plain-text tables can reuse the spellings the
	// encoded ones pin down.
	const order = [];
	for (const [p, pg] of pages) pg.tables.forEach((tb, ti) => order.push([p, tb, ti]));
	order.sort((a, b) => Number(!!b[1].cField) - Number(!!a[1].cField));
	const built = new Map();
	for (const [p, tb, ti] of order) {
		{
			const notes = [];
			const conflicts = [];
			const recs = tb.cField ? decodeC(tb.cField) : null;
			const resolution = RESOLUTIONS[`${p}#${ti}`];
			const mons = tb.rows.map((row, i) => buildMon(row, recs && recs[i], learned, notes, resolution, conflicts));
			if (conflicts.length) throw new Error(`${p}#${ti} unresolved table/encoded conflicts: ${conflicts.join('; ')}`);
			const sets = mons.map(m => m.set);
			const canon = JSON.parse(validator.canonicalizeTeam(JSON.stringify(sets)));
			const verdict = canon.ok ? JSON.parse(validator.validateTeam(JSON.stringify(canon.team))) : { ok: false, errors: canon.errors };
			built.set(`${p}#${ti}`, {
				heading: tb.heading, anchor: tb.anchor, encoded: !!tb.cField,
				rows: tb.rows, cField: tb.cField,
				itemCodes: mons.map(m => m.itemCode), maxHp: mons.map(m => m.maxHp),
				sets: canon.ok ? canon.team : sets,
				canonicalizeApplied: canon.applied || [],
				valid: !!verdict.ok,
				errors: (Array.isArray(verdict.errors) ? verdict.errors : verdict.findings || []).filter(f => f.severity === 'error'),
				notes,
			});
		}
	}

	// item code ↔ item consistency across every encoded table
	const codeToItem = new Map();
	const itemConflicts = [];
	for (const b of built.values()) {
		b.itemCodes.forEach((code, i) => {
			if (code == null) return;
			const it = b.sets[i].item;
			if (codeToItem.has(code) && codeToItem.get(code) !== it) itemConflicts.push({ code, a: codeToItem.get(code), b: it });
			codeToItem.set(code, it);
		});
	}
	if (itemConflicts.length) throw new Error(`item code conflicts: ${JSON.stringify(itemConflicts)}`);

	const records = [];
	for (const e of entries) {
		const pg = pages.get(e.page);
		let tableIdxs;
		if (e.anchor) {
			tableIdxs = pg.tables.map((t, i) => [t, i]).filter(([t]) => t.anchor === e.anchor).map(([, i]) => [i]);
			tableIdxs = tableIdxs.length ? tableIdxs[0] : [];
		} else if (e.name === 'ガラプラス') {
			tableIdxs = pg.tables.map((_, i) => i); // one entry, two published teams
		} else {
			tableIdxs = [0];
		}
		if (!tableIdxs.length) throw new Error(`${e.href}: no table for anchor`);
		tableIdxs.forEach((ti, k) => {
			const b = built.get(`${e.page}#${ti}`);
			const id = e.name === 'ガラプラス' ? `${slugOf(e, 0, [0]).replace(/$/, '')}-${'ab'[k]}` : slugOf(e, ti, e.anchor ? pg.tables : [0]);
			records.push({ id, entry: e, tableIndex: ti, primary: k === 0, ...b });
		});
	}
	// Other team tables on intake pages that no serious entry designates.
	const used = new Set(records.map(r => `${r.entry.page}#${r.tableIndex}`));
	const unlisted = [];
	for (const [p, pg] of pages) {
		pg.tables.forEach((_, ti) => {
			if (used.has(`${p}#${ti}`)) return;
			const b = built.get(`${p}#${ti}`);
			unlisted.push({ id: `mjj-${p.replace(/\.htm$/, '').replace(/\//g, '-').replace(/_/g, '')}-t${ti}`, page: p, tableIndex: ti, ...b });
		});
	}

	const out = {
		source: 'majinjima-party-v1',
		index: BASE + 'index.htm',
		indexFingerprint: contentFingerprint(indexHtml),
		retrieved: '2026-09-29',
		selection: 'index rows with the author genre colour #ffaaaa (かなりガチなパーティ)',
		pageFingerprints: Object.fromEntries([...pages].map(([p, pg]) => [p, pg.fingerprint])),
		itemCodes: Object.fromEntries([...codeToItem].sort((a, b) => a[0] - b[0])),
		entries: records.map(r => ({
			id: r.id,
			url: BASE + r.entry.href,
			name: r.entry.name,
			heading: r.heading,
			indexMembers: r.entry.members,
			indexSummary: r.entry.summary,
			indexLogCount: r.entry.logCount,
			ohkoRule: r.entry.ohkoRule,
			primary: r.primary,
			encoded: r.encoded,
			table: r.rows,
			cField: r.cField,
			sets: r.sets,
			valid: r.valid,
			errors: r.errors,
			canonicalizeApplied: r.canonicalizeApplied,
			notes: r.notes,
		})),
		unlistedPageTeams: unlisted.map(u => ({
			id: u.id, url: BASE + u.page, heading: u.heading, encoded: u.encoded,
			table: u.rows, cField: u.cField, sets: u.sets, valid: u.valid, errors: u.errors, notes: u.notes,
		})),
	};
	fs.mkdirSync(path.dirname(OUT), { recursive: true });
	fs.writeFileSync(OUT, JSON.stringify(out, null, 1) + '\n');
	console.log(`${records.length} records from ${entries.length} serious entries; ${unlisted.length} unlisted page teams`);
	for (const r of records) {
		console.log(`${r.valid ? 'ok ' : 'BAD'} ${r.id.padEnd(28)} ${r.entry.ohkoRule ? '(OHKO) ' : ''}${r.sets.map(s => `${s.level}${s.species}`).join(' ')}`);
		for (const n of r.notes) console.log(`      - ${n}`);
		if (!r.valid) for (const e of r.errors) console.log(`      ! ${JSON.stringify(e)}`);
	}
	for (const u of unlisted) console.log(`unlisted ${u.id} valid=${u.valid} ${u.heading}`);
}

main().catch(e => {
	console.error(e);
	process.exit(1);
});
