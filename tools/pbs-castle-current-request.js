'use strict';
// Castle A (automatic attack) is accepted only when a verified ongoing Encore
// identifies exactly one PP-positive native move. Other A meanings fail closed.
function resolveEncoreAutoAction({protocol, mySide, ppRows}) {
  const assert = require('node:assert/strict');
  assert.ok(mySide === 0 || mySide === 1, 'AUTO_SIDE_INVALID');
  assert.ok(Array.isArray(protocol) && protocol.every(x => typeof x === 'string'),
    'AUTO_PROTOCOL_INVALID');
  assert.ok(Array.isArray(ppRows) && ppRows.length === 4, 'AUTO_PDAT_MOVES_REQUIRED');
  const key = value => String(value || '').toLowerCase().replace(/[^a-z0-9]/g, '');
  const moveKey = value => {
    const k = key(value);
    return k.startsWith('hiddenpower') ? 'hiddenpower' : k;
  };
  const prefix = 'p' + (mySide + 1) + 'a: ';
  let activeActor = null, lastMove = null, encoreMove = null, encore = false;
  let startIndex = -1, moveIndex = -1;
  for (const [index, line] of protocol.entries()) {
    const part = line.split('|');
    const kind = part[1];
    if ((kind === 'switch' || kind === 'drag') && part[2]?.startsWith(prefix)) {
      activeActor = part[2];
      lastMove = null;
      encoreMove = null;
      encore = false;
      moveIndex = -1;
      startIndex = -1;
    } else if (kind === 'move' && activeActor !== null && part[2] === activeActor) {
      const move = moveKey(part[3]);
      assert.ok(move, 'AUTO_OWN_MOVE_ID_UNKNOWN');
      if (encore) assert.equal(move, encoreMove, 'AUTO_ENCORE_MOVE_CHANGED');
      lastMove = move;
      moveIndex = index;
    } else if (kind === '-start' && part[2] === activeActor && key(part[3]) === 'encore') {
      assert.ok(lastMove && moveIndex >= 0, 'AUTO_ENCORE_PRIOR_MOVE_NOT_OBSERVED');
      encore = true;
      encoreMove = lastMove;
      startIndex = index;
    } else if (kind === '-end' && part[2] === activeActor && key(part[3]).endsWith('encore')) {
      encore = false;
      encoreMove = null;
      startIndex = -1;
    } else if ((kind === '-transform' || kind === 'replace') && part[2] === activeActor) {
      encore = false;
      encoreMove = null;
      startIndex = -1;
      lastMove = null;
    }
  }
  assert.ok(activeActor && encore && encoreMove && startIndex >= 0,
    'AUTO_A_REQUIRES_VERIFIED_ACTIVE_ENCORE');
  const slots = ppRows.flatMap((row, index) =>
    moveKey(row.move) === encoreMove ? [index + 1] : []);
  assert.equal(slots.length, 1, 'AUTO_ENCORE_MOVE_NOT_UNIQUE_IN_PDAT');
  const slotIndex = slots[0], pp = ppRows[slotIndex - 1].pp;
  assert.ok(Number.isSafeInteger(pp) && pp > 0,
    'AUTO_ENCORE_CURRENT_PP_NOT_POSITIVE');
  return {kind: 'auto', value: 'A', moveId: encoreMove, slotIndex, pp,
    source: 'PROTOCOL_ACTIVE_ENCORE_AND_OWN_PDAT'};
}

// Castle-only strict current request and action mapping. Maximum PP is supplied
// separately by the observation session from the initial unspent PDAT.
// Copied moves and unrepresented forms are explicit rejection boundaries.
function buildCurrentRequest({client, dexData, mySide, ownSets, previewSides,
  pickedRosterSlots, rawTurnHtml, rawPdatHtml, protocol, decisionTurn}) {
  const assert = require('node:assert/strict');
  const id = value => String(value).normalize('NFD').toLowerCase().replace(/[^a-z0-9]/g, '');
  const hpTypes = new Set(['bug', 'dark', 'dragon', 'electric', 'fighting', 'fire',
    'flying', 'ghost', 'grass', 'ground', 'ice', 'poison', 'psychic', 'rock', 'steel', 'water']);
  const statuses = new Set(['', 'brn', 'par', 'slp', 'frz', 'psn', 'tox', 'fnt']);
  const notes = ['CURRENT_PP_FROM_CURRENT_CASTLE_PDAT',
    'MAXIMUM_PP_NOT_SUPPLIED_WHEN_UNKNOWN',
    'CURRENT_TRAPPED_FROM_RAW_SWITCH_OPTIONS',
    'TRAPPED_UNOBSERVABLE_WITH_NO_LIVE_BENCH_FALSE_ONLY_FOR_CURRENT_LEGALITY',
    'OWN_GENDER_FROM_PUBLIC_PREVIEW_AND_CURRENT_PDAT'];
  const integer = (value, low, high, label) => {
    assert.ok(Number.isSafeInteger(value) && value >= low && value <= high, label);
    return value;
  };
  function rejectUnmapped(value, label) {
    if (value === null || typeof value !== 'object') return;
    for (const key of Object.getOwnPropertyNames(value)) {
      assert.ok(!key.startsWith('_unmapped'), label + ': ' + key);
      if (key !== 'length') rejectUnmapped(value[key], label + '.' + key);
    }
  }
  function speciesKey(mon, label) {
    assert.ok(mon && typeof mon === 'object', label);
    const value = mon.speciesId ?? mon.species;
    assert.ok(typeof value === 'string' && value.length, label + ': species');
    const key = id(value);
    assert.ok(key && Object.hasOwn(dexData.species, key), label + ': unknown species');
    return key;
  }
  function checkGender(value, label) {
    assert.ok(value === '' || value === 'M' || value === 'F', label + ': gender');
    return value;
  }
  function canonicalMove(value, label) {
    assert.ok(typeof value === 'string' && value.length, label + ': move');
    const key = id(value);
    if (key.startsWith('hiddenpower')) {
      const suffix = key.slice('hiddenpower'.length);
      assert.ok(suffix === '' || hpTypes.has(suffix), label + ': Hidden Power type');
      assert.ok(Object.hasOwn(dexData.moves, 'hiddenpower'), label + ': Hidden Power dex entry');
      return 'hiddenpower';
    }
    assert.ok(key && Object.hasOwn(dexData.moves, key), label + ': unknown move');
    return key;
  }
  function hpAndStatus(mon, label) {
    integer(mon.maxHp, 1, 10000, label + ': maxHP');
    integer(mon.hp, 0, mon.maxHp, label + ': HP');
    assert.ok(typeof mon.status === 'string' && statuses.has(mon.status), label + ': status');
    assert.equal(mon.status === 'fnt', mon.hp === 0, label + ': faint/HP consistency');
  }
  function details(value, label) {
    assert.ok(typeof value === 'string', label + ': details');
    const match = value.match(/^([^,]+), L(\d+)(?:, ([MF]))?$/);
    assert.ok(match, label + ': explicit species, level and optional gender required');
    const species = id(match[1]);
    assert.ok(Object.hasOwn(dexData.species, species), label + ': unknown details species');
    return {species, level: integer(Number(match[2]), 1, 100, label + ': details level'),
      gender: match[3] ?? ''};
  }

  assert.ok(mySide === 0 || mySide === 1, 'mySide must be explicit');
  integer(decisionTurn, 1, 65535, 'decisionTurn');
  assert.ok(dexData && dexData.species && dexData.moves && dexData.items, 'dexData tables');
  assert.ok(typeof rawTurnHtml === 'string' && rawTurnHtml.length, 'rawTurnHtml');
  assert.ok(typeof rawPdatHtml === 'string' && rawPdatHtml.length, 'rawPdatHtml');
  assert.ok(Array.isArray(protocol) && protocol.length && protocol.every(line => typeof line === 'string'),
    'protocol lines');
  assert.ok(Array.isArray(previewSides) && previewSides.length === 2, 'previewSides');
  assert.ok(Array.isArray(ownSets) && ownSets.length === 6, 'ownSets');
  assert.ok(Array.isArray(pickedRosterSlots) && pickedRosterSlots.length === 3, 'pickedRosterSlots');
  assert.equal(new Set(pickedRosterSlots).size, 3, 'Duplicate picked roster slots');
  for (const slot of pickedRosterSlots) integer(slot, 0, 5, 'Picked roster slot');
  rejectUnmapped(previewSides, 'preview');
  const preview = previewSides.map((mons, side) => {
    assert.ok(Array.isArray(mons) && mons.length === 6, 'Preview roster length: ' + side);
    const result = mons.map((mon, slot) => ({species: speciesKey(mon, 'preview ' + side + ':' + slot),
      level: integer(mon.level, 1, 100, 'Preview level'), gender: checkGender(mon.gender, 'Preview')}));
    assert.equal(new Set(result.map(mon => mon.species)).size, 6, 'Preview species uniqueness');
    return result;
  });
  const ownMoveIds = ownSets.map((set, slot) => {
    assert.equal(speciesKey(set, 'own set ' + slot), preview[mySide][slot].species, 'Own preview species order');
    assert.equal(integer(set.level, 1, 100, 'Own set level'), preview[mySide][slot].level, 'Own preview level');
    if (set.gender !== undefined && set.gender !== null) {
      const gender = set.gender === 'N' ? '' : checkGender(set.gender, 'Own set');
      assert.equal(gender, preview[mySide][slot].gender, 'Own set/public gender mismatch');
    }
    assert.ok(Array.isArray(set.moves) && set.moves.length === 4, 'Own set must have four moves');
    const moves = set.moves.map((move, index) => canonicalMove(move, 'Own set move ' + slot + ':' + index));
    assert.equal(new Set(moves).size, 4, 'Duplicate canonical own moves');
    return moves;
  });

  const activeSlots = [null, null], protocolPreview = [[], []];
  for (const [index, line] of protocol.entries()) {
    const fields = line.split('|');
    if (fields[1] === 'poke') {
      assert.ok(fields[2] === 'p1' || fields[2] === 'p2', 'Protocol preview side');
      protocolPreview[Number(fields[2][1]) - 1].push(details(fields[3], 'Protocol preview ' + index));
    }
    if (fields[1] !== 'switch' && fields[1] !== 'drag') continue;
    const subject = (fields[2] ?? '').match(/^p([12])a: (.+)$/);
    assert.ok(subject, 'Protocol switch subject at ' + index);
    const side = Number(subject[1]) - 1, mon = details(fields[3], 'Protocol switch ' + index);
    const matches = preview[side].flatMap((known, slot) =>
      known.species === mon.species && known.level === mon.level ? [slot] : []);
    assert.equal(matches.length, 1, 'Protocol switch must match one preview slot');
    const slot = matches[0];
    assert.equal(mon.gender, preview[side][slot].gender, 'Protocol switch/preview gender');
    activeSlots[side] = slot;
  }
  assert.deepEqual(protocolPreview, preview, 'Protocol/public preview identity and order');
  for (const slot of activeSlots) integer(slot, 0, 5, 'Protocol active slot');
  assert.ok(pickedRosterSlots.includes(activeSlots[mySide]), 'Own active was not picked');

  const page = client.classifyPage({html: rawTurnHtml});
  const ui = client.interpretBattleRequest({kind: page.kind, html: rawTurnHtml});
  const pdat = client.parsePdatHtml(rawPdatHtml);
  rejectUnmapped(ui, 'UI');
  rejectUnmapped(pdat, 'PDAT');
  assert.ok(ui && ui.battleTurn && Array.isArray(ui.battleTurn.sides) && ui.battleTurn.sides.length === 2,
    'Parsed top status table');
  assert.ok(['INPUT', 'BATONN', 'CHANGE'].includes(ui.actionType), 'Unsupported decision action');
  const forced = ui.actionType !== 'INPUT';
  assert.equal(ui.isForceSwitch, forced, 'Parser force-switch classification');
  assert.ok(Array.isArray(pdat.mons) && pdat.mons.length === 3, 'PDAT picked roster length');
  const ppBySlot = Array(6).fill(null), pdatBySlot = Array(6).fill(null);
  for (const [pos, mon] of pdat.mons.entries()) {
    const slot = pickedRosterSlots[pos], known = preview[mySide][slot];
    assert.equal(speciesKey(mon, 'PDAT ' + pos), known.species, 'PDAT/picked species order');
    assert.equal(integer(mon.level, 1, 100, 'PDAT level'), known.level, 'PDAT/preview level');
    assert.equal(checkGender(mon.gender, 'PDAT'), known.gender, 'PDAT/preview gender');
    hpAndStatus(mon, 'PDAT ' + pos);
    assert.ok(typeof mon.item === 'string' && (mon.item === '' ||
      (id(mon.item) === mon.item && Object.hasOwn(dexData.items, mon.item))), 'PDAT current item');
    assert.ok(Array.isArray(mon.moves) && mon.moves.length === 4, 'PDAT must expose four mapped move slots');
    const moves = mon.moves.map((move, index) => {
      const key = canonicalMove(move.moveId, 'PDAT move ' + pos + ':' + index);
      return {move: key, pp: integer(move.pp, 0, 2147483647, 'PDAT current PP')};
    });
    assert.deepEqual(moves.map(move => move.move), ownMoveIds[slot], 'PDAT/own move slot order');
    ppBySlot[slot] = moves;
    pdatBySlot[slot] = mon;
  }
  const ownActive = pdatBySlot[activeSlots[mySide]];
  if (ui.actionType === 'CHANGE') assert.equal(ownActive.hp, 0, 'CHANGE requires fainted own active in this supported form');
  else assert.ok(ownActive.hp > 0, 'INPUT/BATONN requires living own active');
  for (const [side, top] of ui.battleTurn.sides.entries()) {
    const absent = top.speciesId === null && top.hp === null && top.maxHp === null;
    if (absent) {
      assert.ok(forced, 'Normal input requires a complete top status table');
      continue;
    }
    const known = preview[side][activeSlots[side]];
    assert.equal(speciesKey(top, 'Top ' + side), known.species, 'Top/protocol active species');
    assert.equal(integer(top.level, 1, 100, 'Top level'), known.level, 'Top/protocol active level');
    assert.equal(checkGender(top.gender, 'Top'), known.gender, 'Top/preview gender');
    hpAndStatus(top, 'Top ' + side);
    if (side === mySide) {
      assert.equal(top.hp, ownActive.hp, 'Own top/PDAT HP');
      assert.equal(top.maxHp, ownActive.maxHp, 'Own top/PDAT maxHP');
      assert.equal(top.status, ownActive.status, 'Own top/PDAT status');
    }
  }

  const options = ui.battleTurn.options;
  assert.ok(Array.isArray(options) && options.length > 0, 'Raw MOVE options');
  assert.equal(new Set(options.map(option => option.value)).size, options.length, 'Duplicate raw options');
  const automatic = options.filter(option => option.value === 'A');
  assert.ok(automatic.length <= 1, 'Duplicate Castle automatic option');
  if (automatic.length) {
    assert.ok(!forced, 'Castle A not legal on forced switch');
    assert.ok(automatic[0].kind === 'other', 'Castle A parser classification changed');
    assert.ok(!options.some(option => option.kind === 'move'), 'Castle A mixed with explicit W');
    const resolved = resolveEncoreAutoAction({protocol, mySide,
      ppRows: ppBySlot[activeSlots[mySide]]});
    Object.assign(automatic[0], resolved);
    notes.push('CASTLE_A_VERIFIED_ENCORE_ONLY');
  }
  const moveOptions = [], switchOptions = [];
  for (const option of options) {
    assert.ok(option.kind === 'move' || option.kind === 'switch' || option.kind === 'auto',
      'Unsupported raw option kind');
    if (option.kind === 'move' || option.kind === 'auto') {
      assert.ok(!forced, 'Forced switch must not expose move options');
      integer(option.slotIndex, 1, 4, 'Raw move slot');
      const row = ppBySlot[activeSlots[mySide]][option.slotIndex - 1];
      if (option.kind === 'move') {
        assert.equal(option.value, 'W' + option.slotIndex, 'Raw W value');
        assert.equal(canonicalMove(option.moveId, 'Raw W move'), row.move, 'Raw W/PDAT move');
        assert.equal(integer(option.pp, 1, 2147483647, 'Raw W current PP'), row.pp, 'Raw W/PDAT PP');
      } else {
        assert.equal(option.value, 'A', 'Raw A value');
        assert.equal(canonicalMove(option.moveId, 'Raw A move'), row.move, 'Raw A/PDAT move');
        assert.equal(integer(option.pp, 1, 2147483647, 'Raw A current PP'), row.pp, 'Raw A/PDAT PP');
      }
      moveOptions.push(option);
    } else {
      integer(option.slotIndex, 1, 3, 'Raw C slot');
      assert.equal(option.value, 'C' + option.slotIndex, 'Raw C value');
      const slot = pickedRosterSlots[option.slotIndex - 1], mon = pdatBySlot[slot];
      assert.notEqual(slot, activeSlots[mySide], 'Raw C cannot select the current active');
      assert.ok(mon.hp > 0, 'Raw C cannot select a fainted mon');
      assert.equal(speciesKey(option, 'Raw C'), preview[mySide][slot].species, 'Raw C/PDAT species');
      assert.equal(option.hp, mon.hp, 'Raw C/PDAT HP');
      assert.equal(option.maxHp, mon.maxHp, 'Raw C/PDAT maxHP');
      switchOptions.push(option);
    }
  }
  const liveBenchPositions = pdat.mons.flatMap((mon, pos) =>
    pickedRosterSlots[pos] !== activeSlots[mySide] && mon.hp > 0 ? [pos + 1] : []);
  const trapped = !forced && liveBenchPositions.length > 0 && switchOptions.length === 0;
  if (forced || switchOptions.length > 0) {
    assert.deepEqual(switchOptions.map(option => option.slotIndex).sort((a, b) => a - b),
      liveBenchPositions.slice().sort((a, b) => a - b), 'Raw C options must cover exactly the live bench');
  }
  const pokemon = pdat.mons.map((mon, pos) => {
    const slot = pickedRosterSlots[pos], known = preview[mySide][slot];
    const name = dexData.species[known.species].name;
    assert.ok(typeof name === 'string' && name.length && !name.includes('|'), 'Dex species display name');
    return {ident: 'p' + (mySide + 1) + ': ' + name,
      details: name + ', L' + mon.level + (mon.gender === '' ? '' : ', ' + mon.gender),
      condition: mon.hp === 0 ? '0 fnt' : mon.hp + '/' + mon.maxHp + (mon.status === '' ? '' : ' ' + mon.status),
      active: slot === activeSlots[mySide], item: mon.item};
  });
  const request = {side: {id: 'p' + (mySide + 1), name: 'p' + (mySide + 1), pokemon}, rqid: decisionTurn};
  if (forced) request.forceSwitch = [true];
  else {
    assert.ok(moveOptions.length > 0, 'Normal input requires at least one explicit W option');
    const enabledSlots = new Set(moveOptions.map(option => option.slotIndex));
    const moves = ppBySlot[activeSlots[mySide]].map((row, index) => ({id: row.move,
      pp: row.pp, disabled: !enabledSlots.has(index + 1)}));
    request.active = [{moves, trapped}];
  }
  const rawLegal = options.map(option => option.kind === 'move' || option.kind === 'auto'
    ? 'move ' + canonicalMove(option.moveId, 'Legal raw move') : 'switch ' + option.slotIndex);
  const requestLegal = [];
  if (!forced) for (const move of request.active[0].moves) {
    if (!move.disabled && move.pp !== 0) requestLegal.push('move ' + move.id);
  }
  if (forced || (!trapped && request.active[0].moves.length > 0)) {
    for (const [pos, mon] of pdat.mons.entries()) {
      if (pickedRosterSlots[pos] !== activeSlots[mySide] && mon.hp > 0) requestLegal.push('switch ' + (pos + 1));
    }
  }
  if (forced && requestLegal.length === 0) requestLegal.push('pass');
  const legalityInputs = {raw: rawLegal.slice().sort(), request: requestLegal.slice().sort()};
  assert.deepEqual(legalityInputs.request, legalityInputs.raw, 'Raw/request legal inputs');
  // Keep Castle's fixed C1..C3 numbering separate from engine positions.
  const castleParty = pickedRosterSlots.slice();
  const activeRosterSlot = activeSlots[mySide];
  const engineParty = [activeRosterSlot, ...castleParty.filter(slot => slot !== activeRosterSlot)];
  const engineToCastle = engineParty.map(slot => castleParty.indexOf(slot) + 1);
  const castleToEngine = castleParty.map(slot => engineParty.indexOf(slot) + 1);
  const originalPokemon = request.side.pokemon;
  request.side.pokemon = engineToCastle.map(position => originalPokemon[position - 1]);
  assert.equal(request.side.pokemon[0].active, true, 'Engine party must be active first');
  const targets = new Map(engineParty.map(slot => [slot, {
    hp: pdatBySlot[slot].hp, status: pdatBySlot[slot].status,
    species: preview[mySide][slot].species
  }]));
  const rawActionMappings = options.map(option => {
    if (option.kind === 'move' || option.kind === 'auto') {
      const key = canonicalMove(option.moveId, 'Raw action map move');
      return {kind: option.kind, value: option.value, nativeInput: 'move ' + key,
        rawInput: 'move ' + key, moveId: key, moveSlot: option.slotIndex,
        actorRosterSlot: activeRosterSlot};
    }
    const castlePosition = option.slotIndex;
    const targetRosterSlot = castleParty[castlePosition - 1];
    const enginePosition = castleToEngine[castlePosition - 1];
    const target = targets.get(targetRosterSlot);
    assert.notEqual(targetRosterSlot, activeRosterSlot, 'Mapped switch targets active');
    assert.ok(target.hp > 0 && target.status !== 'fnt', 'Mapped switch targets fainted mon');
    assert.equal(speciesKey(option, 'Mapped switch'), target.species, 'Mapped switch species');
    return {kind: 'switch', value: option.value, nativeInput: 'switch ' + enginePosition,
      rawInput: 'switch ' + castlePosition, enginePosition, castlePosition,
      targetRosterSlot, species: target.species};
  });
  const expectedNativeInputs = rawActionMappings.map(row => row.nativeInput);
  assert.equal(new Set(expectedNativeInputs).size, expectedNativeInputs.length,
    'Duplicate normalized native action');
  const nativeRequestInputs = [];
  if (!forced) for (const row of request.active[0].moves) {
    if (!row.disabled && row.pp > 0) nativeRequestInputs.push('move ' + row.id);
  }
  if (forced || !trapped) for (const [index, slot] of engineParty.entries()) {
    if (slot !== activeRosterSlot && targets.get(slot).hp > 0) nativeRequestInputs.push('switch ' + (index + 1));
  }
  if (forced && nativeRequestInputs.length === 0) nativeRequestInputs.push('pass');
  legalityInputs.native = nativeRequestInputs.slice().sort();
  assert.deepEqual(legalityInputs.native, expectedNativeInputs.slice().sort(),
    'Normalized request/raw native actions');
  notes.push('NATIVE_REQUEST_PARTY_ACTIVE_FIRST_CASTLE_C_ORDER_PRESERVED',
    'LEGALITY_INPUTS_REQUEST_RETAINS_PRE_NORMALIZATION_CASTLE_ORDER');

  function mapNativeInput(input) {
    assert.equal(typeof input, 'string', 'Native action must be a string');
    const text = input.trim();
    const moveMatch = text.match(/^move\s+(\S+)$/);
    if (moveMatch) {
      const key = canonicalMove(moveMatch[1], 'Native action');
      const matches = rawActionMappings.filter(row =>
        (row.kind === 'move' || row.kind === 'auto') && row.moveId === key);
      assert.equal(matches.length, 1, 'Native move has no unique current raw option: ' + input);
      return {...matches[0], nativeInput: input};
    }
    const switchMatch = text.match(/^switch\s+([1-9]\d*)$/);
    assert.ok(switchMatch, 'Unsupported native action: ' + input);
    const enginePosition = integer(Number(switchMatch[1]), 1, engineParty.length, 'Native switch position');
    const targetRosterSlot = engineParty[enginePosition - 1];
    assert.notEqual(targetRosterSlot, activeRosterSlot, 'Native switch targets current active');
    const target = targets.get(targetRosterSlot);
    assert.ok(target.hp > 0 && target.status !== 'fnt', 'Native switch targets fainted mon');
    const matches = rawActionMappings.filter(row => row.kind === 'switch' && row.targetRosterSlot === targetRosterSlot);
    assert.equal(matches.length, 1, 'Native switch has no unique current raw option: ' + input);
    assert.equal(matches[0].enginePosition, enginePosition, 'Native switch order changed');
    return {...matches[0], nativeInput: input};
  }

  return {request, ui, pdat, activeSlots, ppBySlot, legalityInputs, notes,
    engineParty: engineParty.slice(), castleParty: castleParty.slice(),
    engineToCastle: engineToCastle.slice(), castleToEngine: castleToEngine.slice(),
    activeRosterSlot, expectedNativeInputs: expectedNativeInputs.slice(),
    rawActionMappings: rawActionMappings.map(row => ({...row})), mapNativeInput};
}

module.exports = {buildCurrentRequest, resolveEncoreAutoAction};
