'use strict';
const fs = require('fs');
const path = require('path');
const wasm = require('../crates/wasm/pkg-node/nc2000_wasm.js');
const { normalizeTraining } = require('./normalize-training.js');

const REPO = path.join(__dirname, '..');
const CACHE = path.join(REPO, 'tmp/majinjima-cache');
const ASSASSINS_PATH = path.join(REPO, 'data/kodoku-r3/majinjima-assassins.json');
const BELIEF_POOL_PATH = path.join(REPO, 'data/belief-pool-v3/belief-pool.json');

const i18n = JSON.parse(fs.readFileSync(path.join(REPO, 'data/i18n-ja.json'), 'utf8'));
const dexJson = JSON.parse(fs.readFileSync(path.join(REPO, 'data/gen2stadium2.json'), 'utf8'));

const decode = buf => new TextDecoder('euc-jp').decode(buf);
const stripTags = s => s.replace(/<[^>]+>/g, '').replace(/&nbsp;/g, ' ').trim();
function norm(s) {
	return s
		.replace(/[～〜]+/g, 'ー')
		.replace(/[−―—–‐‑－ｰ-]/g, 'ー')
		.replace(/[\s　]+/g, '')
		.normalize('NFKC');
}
const toId = s => String(s ?? '').toLowerCase().replace(/[^a-z0-9]/g, '');

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

const PDINPUT_ITEM_BY_CODE = {
	0: '', 4: 'brightpowder', 31: 'luckypunch', 36: 'metalpowder', 74: 'quickclaw',
	75: 'psncureberry', 77: 'softsand', 78: 'sharpbeak', 79: 'przcureberry', 80: 'burntberry',
	81: 'iceberry', 82: 'poisonbarb', 83: 'kingsrock', 84: 'bitterberry', 85: 'mintberry',
	89: 'silverpowder', 96: 'mysticwater', 97: 'twistedspoon', 99: 'blackbelt', 103: 'blackglasses',
	105: 'pinkbow', 106: 'stick', 108: 'nevermeltice', 109: 'magnet', 110: 'miracleberry',
	114: 'spelltag', 118: 'miracleseed', 119: 'thickclub', 120: 'focusband', 126: 'hardstone',
	139: 'charcoal', 140: 'berryjuice', 141: 'scopelens', 144: 'metalcoat', 145: 'dragonfang',
	147: 'leftovers', 151: 'mysteryberry', 152: 'dragonscale', 153: 'berserkgene', 164: 'lightball',
	171: 'polkadotbow', 173: 'upgrade', 174: 'berry', 175: 'goldberry',
};

const MOVE_ALIASES = {
	'10万ボトル': 'thunderbolt', '冷凍ビール': 'icebeam', 'アルコール': 'encore', '大文学': 'fireblast',
	'バントタッチ': 'batonpass', '呪い': 'curse', '堪える': 'endure', '滅びの唄': 'perishsong', '滅びの歌': 'perishsong',
	'炎のパンチ': 'firepunch', '炎パンチ': 'firepunch', '雷パンチ': 'thunderpunch',
	'ギガドレイン': 'gigadrain', 'バトンタッチ': 'batonpass', 'アンコール': 'encore', '冷凍ビーム': 'icebeam',
	'10万ボルト': 'thunderbolt', '大文字': 'fireblast', 'サイコキネシス': 'psychic', '地震': 'earthquake',
	'ドわすれ': 'amnesia', 'ド忘れ': 'amnesia', '電磁波': 'thunderwave', '威張る': 'swagger',
	'雷のパンチ': 'thunderpunch', '冷凍パンチ': 'icepunch', '噛み付く': 'bite', '噛みくだく': 'crunch', '噛み砕く': 'crunch',
	'カウンター': 'counter', '眠る': 'rest', '寝言': 'sleeptalk', '嫌な音': 'screech', '撒き菱': 'spikes',
	'高速スピン': 'rapidspin', '空を飛ぶ': 'fly', '波乗り': 'surf', 'ハイドロポンプ': 'hydropump',
	'宿木のタネ': 'leechseed', 'やどりぎのタネ': 'leechseed', '岩雪崩': 'rockslide', '自己再生': 'recover',
	'身代わり': 'substitute', 'タマゴ産み': 'softboiled', '卵産み': 'softboiled', '影分身': 'doubleteam',
	'捨て身タックル': 'doubleedge', 'のしかかり': 'bodyslam', '恩返し': 'return', 'やつあたり': 'frustration', '八つ当たり': 'frustration',
	'大爆発': 'explosion', '自爆': 'selfdestruct', '鈍い': 'curse', '剣の舞': 'swordsdance', '腹太鼓': 'bellydrum',
	'光の壁': 'lightscreen', 'リフレクター': 'reflect', '眠り粉': 'sleeppowder', '痺れ粉': 'stunspore', '毒の粉': 'poisonpowder',
	'どくどく': 'toxic', '毒々': 'toxic', '吹き飛ばし': 'whirlwind', '吠える': 'roar', '地球投げ': 'seismictoss',
	'クロスチョップ': 'crosschop', '爆裂パンチ': 'dynamicpunch', '飛び膝蹴り': 'highjumpkick', '地獄車': 'submission',
	'起死回生': 'reversal', 'じたばた': 'flail', '高速移動': 'agility', '怪しい光': 'confuseray', '黒い眼差し': 'meanlook',
	'道連れ': 'destinybond', '痛み分け': 'painsplit', '朝の日差し': 'morningsun', '光合成': 'synthesis', '月の光': 'moonlight',
	'ミルク飲み': 'milkdrink', '日本晴れ': 'sunnyday', '雨乞い': 'raindance', '砂嵐': 'sandstorm',
	'ソーラービーム': 'solarbeam', '雷': 'thunder', '吹雪': 'blizzard', '火炎放射': 'flamethrower',
	'ヘドロ爆弾': 'sludgebomb', 'シャドーボール': 'shadowball', 'アイアンテール': 'irontail',
	'ドリル嘴': 'drillpeck', 'ドリルくちばし': 'drillpeck', '破壊光線': 'hyperbeam', 'はかいこうせん': 'hyperbeam',
	'催眠術': 'hypnosis', '悪魔のキッス': 'lovelykiss', 'キノコの胞子': 'spore', '歌う': 'sing',
	'電撃砲': 'zapcannon', '電磁砲': 'zapcannon', 'オクタン砲': 'octazooka', '泥棒': 'thief',
	'連続斬り': 'furycutter', '切り裂く': 'slash', '気合溜め': 'focusenergy', '怖い顔': 'scaryface',
	'綿胞子': 'cottonspore', '渦潮': 'whirlpool', 'ミサイル針': 'pinmissile', '指を振る': 'metronome',
	'舌で舐める': 'lick', 'スプーン曲げ': 'kinesis', 'テクスチャー2': 'conversion2', 'メガトンキック': 'megakick',
	'ホネブーメラン': 'bonemerang', '転がる': 'rollout', '丸くなる': 'defensecurl', 'メロメロ': 'attract',
	'甘える': 'charm', 'ロックオン': 'lockon', '心の眼': 'mindreader', '翼で打つ': 'wingattack',
	'電光石火': 'quickattack', '神速': 'extremespeed', '追い打ち': 'pursuit', 'ミラーコート': 'mirrorcoat',
	'金縛り': 'disable', 'こごえる風': 'icywind', '凍える風': 'icywind', '原始の力': 'ancientpower',
	'ヨガのポーズ': 'meditate', 'バリアー': 'barrier', '溶ける': 'acidarmor', '成長': 'growth',
	'癒しの鈴': 'healbell', 'プレゼント': 'present', '我慢': 'bide', '踏みつけ': 'stomp', '踏み付け': 'stomp',
	'未来予知': 'futuresight', '必殺前歯': 'superfang', '頭突き': 'headbutt', '岩砕き': 'rocksmash',
	'二度蹴り': 'doublekick', '回し蹴り': 'rollingkick', 'ゴッドバード': 'skyattack', '超音波': 'supersonic',
	'竜の怒り': 'dragonrage', 'スピードスター': 'swift', '葉っぱカッター': 'razorleaf', '火炎車': 'flamewheel',
	'悪夢': 'nightmare', '守る': 'protect', '見切り': 'detect',
};

const ITEM_ALIASES = {
	'食べ残し': 'leftovers', '奇跡の実': 'miracleberry', '薄荷の実': 'mintberry',
	'麻痺治しの実': 'przcureberry', '黄金の実': 'goldberry', '光の粉': 'brightpowder',
	'太いホネ': 'thickclub', '太い骨': 'thickclub', '破壊の遺伝子': 'berserkgene', 'ピントレンズ': 'scopelens',
	'先制のツメ': 'quickclaw', '先制の爪': 'quickclaw', '水玉リボン': 'polkadotbow', 'ピンクのリボン': 'pinkbow',
	'木の実ジュース': 'berryjuice', '黒帯': 'blackbelt', '銀の粉': 'silverpowder',
	'曲がったスプーン': 'twistedspoon', '神秘の雫': 'mysticwater', '磁石': 'magnet',
	'木炭': 'charcoal', 'ヒントレンズ': 'scopelens', '王者の印': 'kingsrock', '竜の牙': 'dragonfang',
	'毒針': 'poisonbarb', '気合のハチマキ': 'focusband', '気合の鉢巻': 'focusband', '鋭い嘴': 'sharpbeak', '硬い石': 'hardstone',
	'奇跡のタネ': 'miracleseed', '黒いメガネ': 'blackglasses', '解毒の実': 'psncureberry', '毒消しの実': 'psncureberry',
	'焼けた木の実': 'burntberry', '凍った木の実': 'iceberry', '苦い木の実': 'bitterberry', '不思議な木の実': 'mysteryberry',
	'苦い実': 'bitterberry', '木の実': 'berry', '焦点レンズ': 'scopelens', 'ピント': 'scopelens',
	'王者': 'kingsrock', '爪': 'quickclaw', '残飯': 'leftovers',
	'粉': 'brightpowder', '奇跡': 'miracleberry', '薄荷': 'mintberry', '黄金': 'goldberry',
	'リボン': 'polkadotbow', '呪いの御札': 'spelltag', '呪いの札': 'spelltag', 'のろいのおふだ': 'spelltag',
	'フォーカスレンズ': 'scopelens', '電気玉': 'lightball', '電気球': 'lightball', '長ネギ': 'stick',
	'解けない氷': 'nevermeltice', '柔らかい砂': 'softsand', 'メタルコート': 'metalcoat', 'メタルパウダー': 'metalpowder',
	'ラッキーパンチ': 'luckypunch', '竜の鱗': 'dragonscale', 'アップグレード': 'upgrade', 'なし': '', '無し': '',
};

const HP_TYPES = ['fighting', 'flying', 'poison', 'ground', 'rock', 'bug', 'ghost', 'steel',
	'fire', 'water', 'grass', 'electric', 'psychic', 'ice', 'dragon', 'dark'];
const HP_JP = { 蟲: 'bug', 虫: 'bug', 氷: 'ice', 飛: 'flying', 飛行: 'flying', 格: 'fighting', 格闘: 'fighting',
	岩: 'rock', 草: 'grass', 水: 'water', 炎: 'fire', 悪: 'dark', 霊: 'ghost', ゴースト: 'ghost',
	地面: 'ground', 地: 'ground', 電気: 'electric', 電: 'electric', 毒: 'poison', 鋼: 'steel', 超: 'psychic', エスパー: 'psychic', 竜: 'dragon', 龍: 'dragon' };
const hpType = (atk, def) => HP_TYPES[(4 * (atk % 4) + (def % 4)) % 16];

function unorderedMaterialKey(sets) {
	const stats = ['hp', 'atk', 'def', 'spa', 'spd', 'spe'];
	return JSON.stringify(sets.map(p => {
		const sp = toId(p.species);
		const gender = p.gender ?? dexJson.species[sp].gender;
		const item = toId(p.item);
		const moves = p.moves.map(m => {
			const k = toId(m);
			return k.startsWith('hiddenpower') ? 'hiddenpower' : k;
		}).sort();
		return [
			sp, p.level, gender, p.happiness ?? 255,
			stats.map(s => Math.min(31, Math.max(0, p.ivs?.[s] ?? 31)) & 30),
			stats.map(s => Math.min(255, Math.max(0, p.evs?.[s] ?? 0))),
			item, moves,
		];
	}).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0], 'en')));
}

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

function parseAllTables(html) {
	const tables = [];
	const re = /<table[^>]*>([\s\S]*?)<\/table>/gi;
	let m;
	while ((m = re.exec(html))) {
		const rows = [];
		for (const tr of m[1].split(/<tr[^>]*>/i).slice(1)) {
			const cells = [...tr.matchAll(/<td[^>]*>([\s\S]*?)(?=<td|<\/tr|$)/gi)]
				.map(c => stripTags(c[1].replace(/<\/td>/gi, '')));
			const nonEmpty = cells.filter(c => c !== '');
			if (nonEmpty.length >= 6 && /^\d{2}$/.test(nonEmpty[0])) rows.push(nonEmpty);
		}
		if (!rows.length) continue;
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

function moveFromText(text, learnedMoves) {
	let t = norm(text).replace(/\(※\)|（※）/g, '');
	if (t.includes('or')) t = t.split('or')[0];
	const hp = /^(?:めざパ|めざめるパワー|目覚めるパワー)(.*)$/.exec(t);
	if (hp) return { id: 'hiddenpower', hpNamed: hp[1] ? HP_JP[hp[1]] || null : null };
	if (MOVE_ALIASES[t]) return { id: MOVE_ALIASES[t] };
	if (learnedMoves.has(t)) return { id: learnedMoves.get(t) };
	if (movesByJp.has(t)) return { id: movesByJp.get(t) };
	return { id: null };
}

function itemOptionsFromText(text, learnedItems) {
	const cleaned = norm(text).replace(/\(※\)|（※）/g, '');
	const parts = cleaned.split('or');
	const out = [];
	for (const p of parts) {
		if (ITEM_ALIASES[p] !== undefined) out.push(ITEM_ALIASES[p]);
		else if (learnedItems.has(p)) out.push(learnedItems.get(p));
		else if (itemsByJp.has(p)) out.push(itemsByJp.get(p));
		else return null;
	}
	return out;
}

function buildMonFromRec(rec) {
	const speciesId = speciesByNum.get(rec.dex);
	if (!speciesId) throw new Error(`unknown dex ${rec.dex}`);
	const moves = rec.moveNums.map(num => {
		const id = movesByNum.get(num);
		if (!id) throw new Error(`unknown move num ${num}`);
		return id === 'hiddenpower' ? 'hiddenpower' + hpType(rec.dvs.atk, rec.dvs.def) : id;
	});
	const itemId = PDINPUT_ITEM_BY_CODE[rec.itemCode];
	if (itemId === undefined) throw new Error(`unknown itemCode ${rec.itemCode}`);
	const ev = v => (v >= 63 ? 255 : v * 4);
	const iv = d => (d === 15 ? 31 : d * 2);
	const set = {
		name: '', species: dexJson.species[speciesId].name, item: itemId ? dexJson.items[itemId].name : '',
		ability: 'No Ability', moves, nature: 'Serious', level: rec.level,
		evs: { hp: ev(rec.statExp.hp), atk: ev(rec.statExp.atk), def: ev(rec.statExp.def), spa: ev(rec.statExp.spc), spd: ev(rec.statExp.spc), spe: ev(rec.statExp.spe) },
		ivs: { hp: 31, atk: iv(rec.dvs.atk), def: iv(rec.dvs.def), spa: iv(rec.dvs.spc), spd: iv(rec.dvs.spc), spe: iv(rec.dvs.spe) },
	};
	if (moves.includes('frustration')) set.happiness = 0;
	else if (moves.includes('return')) set.happiness = 255;
	return set;
}

function buildMonFromRow(rawRow, rec, learnedMoves, learnedItems, usedItems = null) {
	if (rec) return buildMonFromRec(rec);
	const row = [...rawRow];
	while (row.length > 6 && /^ログ/.test(norm(row[row.length - 1]))) row.pop();
	const [lvText, spText, ...rest] = row;
	const hasDv = /^[0-9A-F]{4}/i.test(rest[rest.length - 1]);
	const dvText = hasDv ? rest[rest.length - 1] : null;
	const itemCell = rest[rest.length - (hasDv ? 2 : 1)];
	const moveCells = rest.slice(0, rest.length - (hasDv ? 2 : 1)).filter(c => norm(c) !== 'ー' && norm(c) !== '');
	const level = parseInt(lvText, 10);
	const speciesId = speciesByJp.get(norm(spText));
	if (!speciesId) throw new Error(`unmapped species ${spText}`);

	const dvHex = /^[0-9A-F]{4}/i.exec(dvText || '');
	let dvs = dvHex
		? (h => ({ atk: parseInt(h[0], 16), def: parseInt(h[1], 16), spe: parseInt(h[2], 16), spc: parseInt(h[3], 16) }))(dvHex[0].toUpperCase())
		: null;
	const moves = moveCells.map(c => {
		const v = moveFromText(c, learnedMoves);
		if (!v.id) throw new Error(`unmapped move "${c}"`);
		return v;
	});
	if (!dvs) {
		const hpNamed = moves.find(m => m.id === 'hiddenpower' && m.hpNamed)?.hpNamed;
		if (hpNamed) {
			const idx = HP_TYPES.indexOf(hpNamed);
			dvs = { atk: 12 + Math.floor(idx / 4), def: 12 + (idx % 4), spe: 15, spc: 15 };
		} else {
			dvs = { atk: 15, def: 15, spe: 15, spc: 15 };
		}
	}
	const finalMoves = moves.map(m => m.id === 'hiddenpower' ? 'hiddenpower' + hpType(dvs.atk, dvs.def) : m.id);

	const itemOpts = itemOptionsFromText(itemCell, learnedItems);
	if (!itemOpts || !itemOpts.length) throw new Error(`unmapped item "${itemCell}"`);
	let itemId = itemOpts[0];
	if (usedItems) {
		const nonColliding = itemOpts.find(opt => !opt || !usedItems.has(opt));
		if (nonColliding !== undefined) itemId = nonColliding;
		if (itemId) usedItems.add(itemId);
	}

	const statExp = { hp: 63, atk: 63, def: 63, spe: 63, spc: 63 };
	const speMatch = /素早さ努力レベル(\d+)/.exec(dvText || '');
	if (speMatch) statExp.spe = parseInt(speMatch[1], 10);
	const atkMatch = /攻撃努力レベル(\d+)/.exec(dvText || '');
	if (atkMatch) statExp.atk = parseInt(atkMatch[1], 10);

	const ev = v => (v >= 63 ? 255 : v * 4);
	const iv = d => (d === 15 ? 31 : d * 2);
	const set = {
		name: '', species: dexJson.species[speciesId].name, item: itemId ? dexJson.items[itemId].name : '',
		ability: 'No Ability', moves: finalMoves, nature: 'Serious', level,
		evs: { hp: ev(statExp.hp), atk: ev(statExp.atk), def: ev(statExp.def), spa: ev(statExp.spc), spd: ev(statExp.spc), spe: ev(statExp.spe) },
		ivs: { hp: 31, atk: iv(dvs.atk), def: iv(dvs.def), spa: iv(dvs.spc), spd: iv(dvs.spc), spe: iv(dvs.spe) },
	};
	if (finalMoves.includes('frustration')) set.happiness = 0;
	else if (finalMoves.includes('return')) set.happiness = 255;
	return set;
}

function main() {
	const dexObj = new wasm.Dex();
	const validator = new wasm.Validator(dexObj);

	const assData = JSON.parse(fs.readFileSync(ASSASSINS_PATH, 'utf8'));
	const beliefPool = JSON.parse(fs.readFileSync(BELIEF_POOL_PATH, 'utf8'));
	const existingIds = new Set(assData.teams.map(t => t.id));
	const existingPages = new Set(assData.teams.map(t => t.page));
	const seenUnorderedMat = new Set([
		...assData.teams.map(t => unorderedMaterialKey(t.sets)),
		...beliefPool.teams.map(t => unorderedMaterialKey(t.sets)),
	]);

	const learnedMoves = new Map();
	const learnedItems = new Map();
	for (const t of assData.teams) {
		if (!t.encoded) continue;
		const [p] = t.page.split('#');
		const fp = path.join(CACHE, p);
		if (!fs.existsSync(fp)) continue;
		const html = decode(fs.readFileSync(fp));
		for (const tb of parseAllTables(html)) {
			if (!tb.cField || tb.rows.length !== 6) continue;
			const recs = decodeC(tb.cField);
			tb.rows.forEach((rawRow, ri) => {
				const rec = recs[ri];
				const row = [...rawRow];
				while (row.length > 6 && /^ログ/.test(norm(row[row.length - 1]))) row.pop();
				const rest = row.slice(2);
				const hasDv = /^[0-9A-F]{4}/i.test(rest[rest.length - 1]);
				const itemTxt = norm(rest[rest.length - (hasDv ? 2 : 1)]).replace(/\(※\)|（※）/g, '');
				const itemId = PDINPUT_ITEM_BY_CODE[rec.itemCode];
				if (itemTxt && itemId !== undefined && !itemTxt.includes('or')) learnedItems.set(itemTxt, itemId);
				const moveCells = rest.slice(0, rest.length - (hasDv ? 2 : 1));
				rec.moveNums.forEach((num, mi) => {
					const mid = movesByNum.get(num);
					const mt = norm(moveCells[mi] || '').replace(/\(※\)|（※）/g, '');
					if (mt && mid && mid !== 'hiddenpower') learnedMoves.set(mt, mid);
				});
			});
		}
	}

	const validateAndNorm = sets => {
		if (sets.length !== 6 || sets.some(s => s.moves.length < 4)) return null;
		const canon = JSON.parse(validator.canonicalizeTeam(JSON.stringify(sets)));
		if (!canon.ok) return null;
		if (!JSON.parse(validator.validateTeam(JSON.stringify(canon.team))).ok) return null;
		const normSets = normalizeTraining(canon.team, dexJson);
		if (!JSON.parse(validator.validateTeam(JSON.stringify(normSets))).ok) return null;
		return normSets;
	};

	const admitTeam = entry => {
		const umat = unorderedMaterialKey(entry.sets);
		if (seenUnorderedMat.has(umat)) return false;
		let id = entry.id;
		if (existingIds.has(id)) id = `${id}-new`;
		assData.teams.push({ ...entry, id });
		existingIds.add(id);
		seenUnorderedMat.add(umat);
		return true;
	};

	try {
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

			if (existingPages.has(href) || summary.includes('(一撃)')) continue;

			const [rawPage, anchor] = href.split('#');
			const relPage = rawPage === '2022/hapimu.htm' ? '2022/hapimu_R.htm' : rawPage;
			const filePath = path.join(CACHE, relPage);
			if (!fs.existsSync(filePath)) continue;

			const pageHtml = decode(fs.readFileSync(filePath));
			const tables = parseAllTables(pageHtml);
			const allC = [...pageHtml.matchAll(/name="C" type="HIDDEN" value="([^"]+)"/gi)].map(m => m[1]);
			const baseSlug = 'mjj-' + rawPage.replace(/\.htm$/, '').replace(/\//g, '-').replace(/_/g, '');

			if (relPage === '2022/hapimu_R.htm' && tables.length > 1) {
				const core = tables[0].rows.map(r => buildMonFromRow(r, null, learnedMoves, learnedItems));
				const variants = [];
				for (let ti = 1; ti < tables.length; ti++) {
					if (tables[ti].rows.length >= 1 && tables[ti].rows.length <= 2) {
						try {
							const subs = tables[ti].rows.map(r => buildMonFromRow(r, null, learnedMoves, learnedItems));
							const normSets = validateAndNorm([...core.slice(0, 6 - subs.length), ...subs]);
							if (normSets) variants.push({ heading: tables[ti].heading || name, encoded: ti === 1, sets: normSets });
						} catch {}
					}
				}
				if (variants.length) {
					const weight = Number((1 / variants.length).toFixed(6));
					variants.forEach((v, idx) => {
						if (admitTeam({ id: `${baseSlug}-v${idx + 1}`, weight, color, page: href, name, summary, heading: v.heading, encoded: v.encoded, sets: v.sets })) {
							addedCount++;
						}
					});
					existingPages.add(href);
				}
				continue;
			}

			if (relPage === '2012/support.htm' && tables.length > 1 && allC.length === 1) {
				const cRecs = decodeC(allC[0]);
				const variants = [];
				for (let ti = 1; ti < tables.length; ti++) {
					if (tables[ti].rows.length === 1) {
						try {
							const ace = ti === 1 ? buildMonFromRec(cRecs[0]) : buildMonFromRow(tables[ti].rows[0], null, learnedMoves, learnedItems);
							const usedItems = new Set(ace.item ? [toId(ace.item)] : []);
							const core5 = ti === 1
								? cRecs.slice(1).map(r => buildMonFromRec(r))
								: tables[0].rows.map(r => buildMonFromRow(r, null, learnedMoves, learnedItems, usedItems));
							const normSets = validateAndNorm([ace, ...core5]);
							if (normSets) variants.push({ heading: tables[ti].heading || name, encoded: ti === 1, sets: normSets });
						} catch {}
					}
				}
				if (variants.length) {
					const weight = Number((1 / variants.length).toFixed(6));
					variants.forEach((v, idx) => {
						if (admitTeam({ id: `${baseSlug}-v${idx + 1}`, weight, color, page: href, name, summary, heading: v.heading, encoded: v.encoded, sets: v.sets })) {
							addedCount++;
						}
					});
					existingPages.add(href);
				}
				continue;
			}

			const sixTables = tables.filter(t => t.rows.length === 6);
			let target = anchor ? sixTables.find(t => t.anchor === anchor) : null;
			if (!target && sixTables.length) target = sixTables[0];

			if (target) {
				try {
					const recs = target.cField ? decodeC(target.cField) : null;
					const sets = target.rows.map((r, i) => buildMonFromRow(r, recs && recs[i], learnedMoves, learnedItems));
					const normSets = validateAndNorm(sets);
					if (normSets) {
						const id = anchor ? `${baseSlug}-${anchor}` : baseSlug;
						if (admitTeam({ id, color, page: href, name, summary, heading: target.heading || name, encoded: !!target.cField, sets: normSets })) {
							addedCount++;
							existingPages.add(href);
						}
					}
				} catch {}
			} else if (allC.length === 1) {
				try {
					const recs = decodeC(allC[0]);
					const normSets = validateAndNorm(recs.map(r => buildMonFromRec(r)));
					if (normSets) {
						const id = anchor ? `${baseSlug}-${anchor}` : baseSlug;
						if (admitTeam({ id, color, page: href, name, summary, heading: name, encoded: true, sets: normSets })) {
							addedCount++;
							existingPages.add(href);
						}
					}
				} catch {}
			}
		}

		fs.writeFileSync(ASSASSINS_PATH, JSON.stringify(assData, null, 2) + '\n');
		console.log(JSON.stringify({ path: ASSASSINS_PATH, added: addedCount, total: assData.teams.length }));
	} finally {
		validator.free();
		dexObj.free();
	}
}

main();
