'use strict';
const fs = require('fs');
const path = require('path');
const wasm = require('../crates/wasm/pkg-node/nc2000_wasm.js');
const {normalizeTraining} = require('./normalize-training.js');

const REPO = path.join(__dirname, '..');
const CACHE = path.join(REPO, 'tmp/majinjima-cache');
const ASSASSINS_PATH = path.join(REPO, 'data/kodoku-r3/majinjima-assassins.json');

const i18n = JSON.parse(fs.readFileSync(path.join(REPO, 'data/i18n-ja.json'), 'utf8'));
const dexJson = JSON.parse(fs.readFileSync(path.join(REPO, 'data/gen2stadium2.json'), 'utf8'));

const decode = buf => new TextDecoder('euc-jp').decode(buf);
const stripTags = s => s.replace(/<[^>]+>/g, '').replace(/&nbsp;/g, ' ').trim();
function norm(s) {
	return s.replace(/[−―—–‐‑－ｰ-]/g, 'ー').replace(/[\s　]+/g, '').normalize('NFKC');
}

const speciesByNum = new Map();
for (const [id, e] of Object.entries(dexJson.species)) if (e.num > 0 && !speciesByNum.has(e.num)) speciesByNum.set(e.num, id);
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

const MOVE_ALIASES = {
	'10万ボトル': 'thunderbolt', '冷凍ビール': 'icebeam', 'アルコール': 'encore', '大文学': 'fireblast',
	'バントタッチ': 'batonpass', '呪い': 'curse', '堪える': 'endure', '滅びの唄': 'perishsong', '炎のパンチ': 'firepunch',
	'ギガドレイン': 'gigadrain', 'バトンタッチ': 'batonpass', 'アンコール': 'encore', '冷凍ビーム': 'icebeam',
	'10万ボルト': 'thunderbolt', '大文字': 'fireblast', 'サイコキネシス': 'psychic', '地震': 'earthquake',
	'ドわすれ': 'amnesia', 'ド忘れ': 'amnesia', '電磁波': 'thunderwave', '威張る': 'swagger',
	'雷のパンチ': 'thunderpunch', '冷凍パンチ': 'icepunch', '噛み付く': 'bite', '噛みくだく': 'crunch',
	'カウンター': 'counter', '眠る': 'rest', '寝言': 'sleeptalk', '嫌な音': 'screech', '撒き菱': 'spikes',
	'高速スピン': 'rapidspin', '空を飛ぶ': 'fly', '波乗り': 'surf', 'ハイドロポンプ': 'hydropump',
};
const ITEM_ALIASES = {
	'食べ残し': 'leftovers', '奇跡の実': 'miracleberry', '薄荷の実': 'mintberry',
	'麻痺治しの実': 'przcureberry', '黄金の実': 'goldberry', '光の粉': 'brightpowder',
	'太いホネ': 'thickclub', '破壊の遺伝子': 'berserkgene', 'ピントレンズ': 'scopelens',
	'先制のツメ': 'quickclaw', '水玉リボン': 'polkadotbow', 'ピンクのリボン': 'pinkbow',
	'木の実ジュース': 'berryjuice', '黒帯': 'blackbelt', '銀の粉': 'silverpowder',
	'曲がったスプーン': 'twistedspoon', '神秘の雫': 'mysticwater', '磁石': 'magnet',
	'木炭': 'charcoal', 'ヒントレンズ': 'scopelens', '王者の印': 'kingsrock', '竜の牙': 'dragonfang',
	'毒針': 'poisonbarb', '気合のハチマキ': 'focusband', '鋭い嘴': 'sharpbeak', '硬い石': 'hardstone',
	'奇跡のタネ': 'miracleseed', '黒いメガネ': 'blackglasses', '解毒の実': 'psncureberry',
	'苦い実': 'bitterberry', '木の実': 'berry', '焦点レンズ': 'scopelens', 'ピント': 'scopelens',
	'気合の鉢巻': 'focusband', '王者': 'kingsrock', '爪': 'quickclaw', '残飯': 'leftovers',
	'粉': 'brightpowder', '奇跡': 'miracleberry', '薄荷': 'mintberry', '黄金': 'goldberry',
	'リボン': 'polkadotbow', '呪いの御札': 'spelltag', '呪いの札': 'spelltag', 'のろいのおふだ': 'spelltag',
	'フォーカスレンズ': 'scopelens', 'ピントレンズ ': 'scopelens',
};

const HP_TYPES = ['fighting', 'flying', 'poison', 'ground', 'rock', 'bug', 'ghost', 'steel',
	'fire', 'water', 'grass', 'electric', 'psychic', 'ice', 'dragon', 'dark'];
const HP_JP = { 蟲: 'bug', 虫: 'bug', 氷: 'ice', 飛: 'flying', 飛行: 'flying', 格: 'fighting', 格闘: 'fighting',
	岩: 'rock', 草: 'grass', 水: 'water', 炎: 'fire', 悪: 'dark', 霊: 'ghost', ゴースト: 'ghost', 地面: 'ground', 電気: 'electric' };
const hpType = (atk, def) => HP_TYPES[(4 * (atk % 4) + (def % 4)) % 16];

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

function moveFromText(text) {
	const t = norm(text);
	const hp = /^(?:めざパ|めざめるパワー|目覚めるパワー)(.*)$/.exec(t);
	if (hp) return { id: 'hiddenpower', hpNamed: hp[1] ? HP_JP[hp[1]] || `?${hp[1]}` : null };
	if (MOVE_ALIASES[t]) return { id: MOVE_ALIASES[t], alias: t };
	if (movesByJp.has(t)) return { id: movesByJp.get(t) };
	return { id: null };
}

function itemFromText(text) {
	const t = norm(text);
	return ITEM_ALIASES[t] || itemsByJp.get(t) || null;
}

function buildMon(row, rec) {
	const [lvText, spText, ...rest] = row;
	const hasDv = /^[0-9A-F]{4}/i.test(rest[rest.length - 1]);
	const dvText = hasDv ? rest[rest.length - 1] : null;
	const item = rest[rest.length - (hasDv ? 2 : 1)];
	const moveCells = rest.slice(0, rest.length - (hasDv ? 2 : 1));
	const level = parseInt(lvText, 10);
	const speciesId = rec ? speciesByNum.get(rec.dex) : speciesByJp.get(norm(spText));
	if (!speciesId) throw new Error(`unmapped species ${spText}`);

	const dvHex = /^[0-9A-F]{4}/i.exec(dvText || '');
	const tableDvs = dvHex ? (h => ({ atk: parseInt(h[0], 16), def: parseInt(h[1], 16), spe: parseInt(h[2], 16), spc: parseInt(h[3], 16) }))(dvHex[0].toUpperCase()) : null;
	let dvs;
	if (rec) dvs = rec.dvs;
	else if (tableDvs) dvs = tableDvs;
	else dvs = { atk: 15, def: 15, spe: 15, spc: 15 };

	const moves = [];
	const visible = moveCells.map(c => moveFromText(c));
	if (rec) {
		rec.moveNums.forEach((num, i) => {
			let id = movesByNum.get(num);
			moves.push(id);
		});
	} else {
		visible.forEach((v, i) => {
			if (!v.id) throw new Error(`unmapped move "${moveCells[i]}"`);
			moves.push(v.id);
		});
	}
	const hpIdx = moves.indexOf('hiddenpower');
	if (hpIdx >= 0) {
		const t = hpType(dvs.atk, dvs.def);
		moves[hpIdx] = 'hiddenpower' + t;
	}

	const itemId = itemFromText(item);
	if (!itemId) throw new Error(`unmapped item "${item}"`);

	const statExp = rec ? rec.statExp : { hp: 63, atk: 63, def: 63, spe: 63, spc: 63 };
	const ev = v => (v >= 63 ? 255 : v * 4);
	const iv = d => (d === 15 ? 31 : d * 2);
	const set = {
		name: '', species: dexJson.species[speciesId].name, item: dexJson.items[itemId].name,
		ability: 'No Ability', moves, nature: 'Serious', level,
		evs: { hp: ev(statExp.hp), atk: ev(statExp.atk), def: ev(statExp.def), spa: ev(statExp.spc), spd: ev(statExp.spc), spe: ev(statExp.spe) },
		ivs: { hp: 31, atk: iv(dvs.atk), def: iv(dvs.def), spa: iv(dvs.spc), spd: iv(dvs.spc), spe: iv(dvs.spe) },
	};
	if (moves.includes('frustration')) set.happiness = 0;
	else if (moves.includes('return')) set.happiness = 255;
	return set;
}

function main() {
	const dexObj = new wasm.Dex();
	const validator = new wasm.Validator(dexObj);

	const assData = JSON.parse(fs.readFileSync(ASSASSINS_PATH, 'utf8'));
	const existingIds = new Set(assData.teams.map(t => t.id));
	const existingPages = new Set(assData.teams.map(t => t.page));

	const indexBuf = fs.readFileSync(path.join(CACHE, 'index.htm'));
	const html = decode(indexBuf);
	const rows = html.split(/<tr align="center">/i).slice(1);

	let addedCount = 0;
	for (const row of rows) {
		const colorMatch = /<td bgcolor="(#[0-9a-f]{6})"/i.exec(row);
		const linkMatch = /<a Href="([^"]+)">([\s\S]*?)<\/a>/i.exec(row);
		if (!linkMatch) continue;
		const href = linkMatch[1];
		const color = colorMatch ? colorMatch[1].toLowerCase() : null;
		const name = linkMatch[2].replace(/<[^>]+>/g, '').trim();
		const tds = [...row.matchAll(/<td[^>]*>([\s\S]*?)<\/td>/gi)].map(m => m[1].replace(/<[^>]+>/g, '').trim());
		const summary = tds[3] || '';

		if (existingPages.has(href)) continue;

		const [page, anchor] = href.split('#');
		const filePath = path.join(CACHE, page);
		if (!fs.existsSync(filePath)) continue;

		const pageHtml = decode(fs.readFileSync(filePath));
		const tables = parsePage(pageHtml);
		if (!tables.length) continue;

		let table = tables[0];
		if (anchor) {
			const found = tables.find(t => t.anchor === anchor);
			if (found) table = found;
		}

		try {
			const recs = table.cField ? decodeC(table.cField) : null;
			const sets = table.rows.map((r, i) => buildMon(r, recs && recs[i]));
			const canon = JSON.parse(validator.canonicalizeTeam(JSON.stringify(sets)));
			if (!canon.ok) continue;
			const validRes = JSON.parse(validator.validateTeam(JSON.stringify(canon.team)));
			if (!validRes.ok) continue;

			const baseSlug = 'mjj-' + page.replace(/\.htm$/, '').replace(/\//g, '-').replace(/_/g, '');
			let id = anchor ? `${baseSlug}-${anchor}` : baseSlug;
			if (existingIds.has(id)) {
				id = `${id}-new`;
			}

			const normSets = normalizeTraining(canon.team, dexJson);
			assData.teams.push({
				id,
				color,
				page: href,
				name,
				summary,
				heading: table.heading || name,
				encoded: !!table.cField,
				sets: normSets,
			});
			existingIds.add(id);
			existingPages.add(href);
			addedCount++;
		} catch (err) {}
	}

	// Also ensure kabegon is in assassins if not present
	if (!existingIds.has('mjj-2026-kabegon') && fs.existsSync(path.join(CACHE, '2026/kabegon.htm'))) {
		const pageHtml = decode(fs.readFileSync(path.join(CACHE, '2026/kabegon.htm')));
		const tables = parsePage(pageHtml);
		if (tables.length) {
			const table = tables[0];
			const recs = decodeC(table.cField);
			const sets = table.rows.map((r, i) => buildMon(r, recs[i]));
			const canon = JSON.parse(validator.canonicalizeTeam(JSON.stringify(sets)));
			if (canon.ok) {
				const normSets = normalizeTraining(canon.team, dexJson);
				assData.teams.push({
					id: 'mjj-2026-kabegon',
					color: '#ffaaaa',
					page: '2026/kabegon.htm',
					name: 'カベゴン',
					summary: '寝言カビSAガチ',
					heading: 'カベゴン',
					encoded: true,
					sets: normSets,
				});
				existingIds.add('mjj-2026-kabegon');
				addedCount++;
			}
		}
	}

	fs.writeFileSync(ASSASSINS_PATH, JSON.stringify(assData, null, 2) + '\n');
	console.log(`Updated ${ASSASSINS_PATH}: added ${addedCount} teams. Total teams: ${assData.teams.length}`);
}

main();
