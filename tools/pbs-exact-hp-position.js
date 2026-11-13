'use strict';

/**
 * Exact public opponent HP gate for battle PositionSpec inputs.
 * READY owns an already positioned searcher; the caller must free it.
 * Other statuses return no searcher and require an explicit caller decision.
 * Observations are explicit slot/species/maxHp facts, never default ratios.
 * Completion preserves fixed EVs and non-IV fields of normal candidates.
 * It retains every nearest DV tie, splitting and merging parent weights.
 * Rebuild from the original prior for each position; do not reuse a READY
 * certification or feed completed candidates back as the original prior.
 * Preview, zero normal candidates, fixed-template failures and limits are
 * explicit boundaries. This module performs no search or network access.
 */
function prepareExactHpPosition(options = {}) {
  const result = {status: 'INPUT_ERROR', searcher: null, beforeBelief: null, afterBelief: null,
    candidateChecks: [], excludedIds: [], remainingAliveIds: [], rootChecks: []};
  let searcher = null, errorStatus = 'INPUT_ERROR';
  const norm = value => typeof value === 'string'
    ? value.normalize('NFD').toLowerCase().replace(/[^a-z0-9]/g, '') : '';
  const requireValue = (ok, message) => { if (!ok) throw new Error(message); };
  const integer = value => Number.isSafeInteger(value);
  const stop = (status, error) => { result.status = status; if (error) result.error = error; return result; };
  function releaseCurrent() {
    const current = searcher;
    searcher = null;
    if (current) current.free();
  }
  function readBelief() {
    const belief = JSON.parse(searcher.beliefInfo());
    requireValue(integer(belief?.count) && belief.count >= 0 && typeof belief.fallback === 'boolean' &&
      Array.isArray(belief.candidates) && belief.candidates.length === belief.count &&
      belief.candidates.every(id => typeof id === 'string' && id.length > 0) &&
      new Set(belief.candidates).size === belief.count, 'Invalid engine belief response');
    return belief;
  }
  try {
    const {wasm, dex, side, poolJson, spec, observations, seed = 1, c = 0.4} = options;
    requireValue(wasm && typeof wasm.ProtocolSearcher === 'function' &&
      typeof wasm.Battle === 'function' && dex, 'Missing WASM runtime or Dex');
    requireValue(side === 0 || side === 1, 'Invalid side');
    requireValue(integer(seed) && seed >= 0 && seed <= 0xffffffff &&
      Number.isFinite(c) && c >= 0, 'Invalid searcher configuration');
    requireValue(typeof poolJson === 'string', 'poolJson must be a string');
    const pool = JSON.parse(poolJson);
    requireValue(pool && Array.isArray(pool.teams), 'Missing pool teams');
    const teams = new Map();
    for (const team of pool.teams) {
      requireValue(team && typeof team.id === 'string' && team.id.length > 0 &&
        !teams.has(team.id) && Array.isArray(team.sets), 'Invalid or duplicate pool team ID');
      teams.set(team.id, team);
    }
    const position = JSON.parse(JSON.stringify(spec));
    requireValue(position?.schema === 'nc2000-position-v1' && position.side === side &&
      position.team_preview === false && Array.isArray(position.sides) && position.sides.length === 2,
    'Expected a battle PositionSpec for this side');
    const foe = 1 - side, roster = position.sides[foe]?.mons;
    requireValue(Array.isArray(roster) && roster.length === 6 && roster.every(m => norm(m?.species)) &&
      new Set(roster.map(m => norm(m.species))).size === roster.length, 'Invalid or repeated opponent species');
    requireValue(Array.isArray(observations) && observations.length > 0, 'Explicit HP observations are required');
    const evidence = JSON.parse(JSON.stringify(observations)), slots = new Set(), species = new Set();
    for (const observation of evidence) {
      requireValue(observation && integer(observation.slot) && observation.slot >= 0 &&
        observation.slot < roster.length && !slots.has(observation.slot), 'Invalid or duplicate observation slot');
      const key = norm(observation.species), mon = roster[observation.slot];
      requireValue(key && !species.has(key) && key === norm(mon.species), 'Invalid or repeated observation species');
      requireValue(observation.side === undefined || observation.side === foe, 'Observation is for the wrong side');
      requireValue(integer(observation.maxHp) && observation.maxHp > 0 &&
        integer(mon.hp_den) && mon.hp_den === observation.maxHp && integer(mon.hp_num) &&
        mon.hp_num >= 0 && mon.hp_num <= observation.maxHp, 'Observation and input HP disagree');
      requireValue(observation.hp === undefined ||
        (integer(observation.hp) && observation.hp === mon.hp_num), 'Current observed HP differs from input');
      slots.add(observation.slot);
      species.add(key);
    }
    const specJson = JSON.stringify(position);
    errorStatus = 'ENGINE_ERROR';
    searcher = new wasm.ProtocolSearcher(dex, side, poolJson, seed, c);
    searcher.setPosition(specJson);
    result.beforeBelief = readBelief();
    if (!result.beforeBelief.count || result.beforeBelief.fallback) return stop('NO_PUBLIC_CANDIDATE');
    for (const candidateId of result.beforeBelief.candidates) {
      const team = teams.get(candidateId);
      requireValue(team, 'Engine returned an unknown candidate ID: ' + candidateId);
      let battle = null;
      try {
        const setsJson = JSON.stringify(team.sets);
        battle = new wasm.Battle(dex, setsJson, setsJson, '1,2,3,4');
        const state = JSON.parse(battle.stateView()), party = state?.sides?.[0]?.party;
        requireValue(Array.isArray(party), 'Candidate state party is missing');
        const checks = evidence.map(observation => {
          const matches = party.filter(mon => norm(mon.species) === norm(observation.species));
          requireValue(matches.length === 1 && integer(matches[0].maxhp) && matches[0].maxhp > 0,
            'Cannot measure candidate maximum HP: ' + candidateId + '/' + observation.species);
          return {slot: observation.slot, species: norm(observation.species), maxHp: matches[0].maxhp,
            observedMaxHp: observation.maxHp, match: matches[0].maxhp === observation.maxHp};
        });
        const match = checks.every(check => check.match);
        result.candidateChecks.push({id: candidateId, checks, match});
        (match ? result.remainingAliveIds : result.excludedIds).push(candidateId);
      } finally { if (battle) battle.free(); }
    }
    if (!result.remainingAliveIds.length) return stop('NO_EXACT_HP_CANDIDATE');
    let acceptedPoolJson = poolJson;
    if (result.excludedIds.length) {
      const excluded = new Set(result.excludedIds);
      acceptedPoolJson = JSON.stringify({...pool, teams: pool.teams.filter(team => !excluded.has(team.id))});
      releaseCurrent();
      searcher = new wasm.ProtocolSearcher(dex, side, acceptedPoolJson, seed, c);
      searcher.setPosition(specJson);
    }
    result.afterBelief = readBelief();
    requireValue(result.afterBelief.fallback === false && result.afterBelief.count === result.remainingAliveIds.length &&
      JSON.stringify([...result.afterBelief.candidates].sort()) === JSON.stringify([...result.remainingAliveIds].sort()),
    'Filtered pool produced an unexpected candidate set');
    const state = JSON.parse(searcher.stateView()), party = state?.sides?.[foe]?.party;
    requireValue(Array.isArray(party), 'Opponent root state party is missing');
    result.rootChecks = evidence.map(observation => {
      const input = roster[observation.slot];
      const matches = party.filter(mon => norm(mon.species) === norm(observation.species));
      const root = matches.length === 1 ? matches[0] : null;
      return {slot: observation.slot, species: norm(observation.species), inputHp: input.hp_num,
        inputMaxHp: input.hp_den, observedMaxHp: observation.maxHp, rootMatches: matches.length,
        rootHp: root?.hp ?? null, rootMaxHp: root?.maxhp ?? null,
        match: root !== null && integer(root.hp) && integer(root.maxhp) &&
          root.hp === input.hp_num && root.maxhp === input.hp_den && root.maxhp === observation.maxHp};
    });
    if (!result.rootChecks.every(check => check.match)) return stop('ROOT_HP_MISMATCH');
    result.status = 'READY';
    result.poolJson = acceptedPoolJson;
    result.searcher = searcher;
    searcher = null;
    return result;
  } catch (error) {
    return stop(errorStatus, String(error?.message || error));
  } finally {
    try { releaseCurrent(); }
    catch (error) { result.status = 'ENGINE_ERROR'; result.error = 'Searcher cleanup failed: ' + String(error?.message || error); }
  }
}

function completeExactHpPosition(options = {}) {
  const initial = prepareExactHpPosition(options);
  const completion = {attempted: false, initialStatus: initial.status, parents: [], generated: [], totalWeight: 0};
  if (initial.status !== 'NO_EXACT_HP_CANDIDATE') return {...initial, completion};
  completion.attempted = true;
  let owned = null, answer = null;
  const fail = (status, error) => (answer = {...initial, status, searcher: null, error, completion});
  const check = (ok, message, code = 'UNSUPPORTED_TEMPLATE') => {
    if (!ok) { const error = new Error(message); error.completionStatus = code; throw error; }
  };
  const norm = value => typeof value === 'string'
    ? value.normalize('NFD').toLowerCase().replace(/[^a-z0-9]/g, '') : '';
  const copy = value => JSON.parse(JSON.stringify(value));
  const canonical = value => Array.isArray(value) ? '[' + value.map(canonical).join(',') + ']'
    : value !== null && typeof value === 'object'
      ? '{' + Object.keys(value).sort().map(key => JSON.stringify(key) + ':' + canonical(value[key])).join(',') + '}'
      : JSON.stringify(value);
  try {
    const {speciesData, maxVariantsPerSource = 128, maxTotalVariants = 256} = options;
    check(speciesData && typeof speciesData === 'object' && !Array.isArray(speciesData), 'Missing species dictionary', 'INPUT_ERROR');
    check(Number.isSafeInteger(maxVariantsPerSource) && maxVariantsPerSource > 0 &&
      Number.isSafeInteger(maxTotalVariants) && maxTotalVariants > 0, 'Invalid variant limits', 'INPUT_ERROR');
    const pool = JSON.parse(options.poolJson), position = copy(options.spec), foe = 1 - options.side;
    const roster = position.sides[foe].mons, observations = new Map(options.observations.map(o => [o.slot, o]));
    const parents = new Set(initial.beforeBelief.candidates), byId = new Map(pool.teams.map(team => [team.id, team]));
    const generated = new Map(), generatedIdKeys = new Map();
    const statKeys = ['hp', 'atk', 'def', 'spa', 'spd', 'spe'];
    let expectedWeight = 0;
    function solveMon(template, publicMon, slot, parentReport) {
      const species = norm(template.species), data = speciesData[species];
      check(!Object.hasOwn(template, 'dvs'), 'Unsupported legacy dvs metadata: ' + species);
      check(data && Number.isSafeInteger(data.baseStats?.hp) && data.baseStats.hp > 0,
        'Missing species HP data: ' + species);
      check(Number.isSafeInteger(template.level) && template.level >= 1 && template.level <= 100 &&
        template.level === publicMon.level, 'Invalid or mismatched level: ' + species);
      check(Array.isArray(template.moves) && template.moves.length > 0 && template.moves.length <= 4 &&
        template.moves.every(move => norm(move)), 'Unsupported moves: ' + species);
      for (const [field, limit] of [['ivs', 255], ['evs', 65535]]) {
        const values = template[field];
        check(values == null || (typeof values === 'object' && !Array.isArray(values) &&
          Object.entries(values).every(([key, value]) => statKeys.includes(key) &&
            Number.isSafeInteger(value) && value >= 0 && value <= limit)), 'Unsupported ' + field + ': ' + species);
      }
      const sourceDvs = statKeys.map(key => (Math.max(0, Math.min(31, template.ivs?.[key] ?? 31)) & 30) / 2);
      const [a0, d0, s0, c0] = [sourceDvs[1], sourceDvs[2], sourceDvs[5], sourceDvs[3]];
      const actualSourceType = 4 * (a0 % 4) + (d0 % 4);
      const hasHiddenPower = template.moves.some(move => norm(move).startsWith('hiddenpower'));
      const hpMask = publicMon.hp_type_mask ?? 0xffff;
      check(Number.isSafeInteger(hpMask) && hpMask >= 0 && hpMask <= 0xffff, 'Invalid HP type mask: ' + species);
      const publicGender = publicMon.gender === '' || publicMon.gender === 'N' ? 'N' : publicMon.gender;
      const rawGender = template.gender;
      const absentGender = rawGender == null || rawGender === '';
      const fixedGender = ['M', 'F', 'N'].includes(data.gender) ? data.gender : null;
      let femaleFraction = null;
      if (fixedGender) {
        check(publicGender === fixedGender && (absentGender || rawGender === fixedGender),
          'Fixed gender mismatch: ' + species);
      } else {
        check((rawGender === 'M' || rawGender === 'F') && rawGender === publicGender,
          'Ambiguous or mismatched mixed-species gender: ' + species);
        femaleFraction = data.genderRatio?.F ?? 0.5;
        check(Number.isFinite(femaleFraction) && femaleFraction >= 0 && femaleFraction <= 1,
          'Invalid female ratio: ' + species);
      }
      const targetHp = observations.get(slot)?.maxHp;
      const evTerm = Math.floor(Math.max(0, Math.min(255, template.evs?.hp ?? 0)) / 4);
      const possibleHp = Array.from({length: 16}, (_, hpDv) =>
        Math.floor((2 * data.baseStats.hp + 2 * hpDv + evTerm + 100) * template.level / 100 + 10));
      const report = {slot, species, targetMaxHp: targetHp ?? null, examined: 0, feasible: 0,
        minDistance: null, ties: 0, sourceActualHpType: hasHiddenPower ? actualSourceType : null};
      parentReport.monChecks.push(report);
      let minimum = Infinity, solutions = [];
      for (let a = 0; a < 16; a++) for (let d = 0; d < 16; d++)
        for (let s = 0; s < 16; s++) for (let special = 0; special < 16; special++) {
          report.examined++;
          if (!fixedGender && ((a < femaleFraction * 16) !== (publicGender === 'F'))) continue;
          const type = 4 * (a % 4) + (d % 4);
          if (!(hpMask & (1 << type)) || (hasHiddenPower && type !== actualSourceType)) continue;
          const h = ((a & 1) << 3) | ((d & 1) << 2) | ((s & 1) << 1) | (special & 1);
          if (targetHp !== undefined && possibleHp[h] !== targetHp) continue;
          report.feasible++;
          const distance = Math.abs(a - a0) + Math.abs(d - d0) + Math.abs(s - s0) + Math.abs(special - c0);
          if (distance < minimum) { minimum = distance; solutions = []; report.ties = 0; }
          if (distance === minimum) {
            report.ties++;
            if (solutions.length <= maxVariantsPerSource) solutions.push({hp: 2 * h, atk: 2 * a,
              def: 2 * d, spa: 2 * special, spd: 2 * special, spe: 2 * s});
          }
        }
      report.minDistance = Number.isFinite(minimum) ? minimum : null;
      return {template, report, solutions};
    }
    for (const parentId of initial.beforeBelief.candidates) {
      const parent = byId.get(parentId);
      check(parent && Array.isArray(parent.sets) && parent.sets.length === 6, 'Invalid parent team: ' + parentId);
      const parentWeight = parent.weight ?? 1;
      check(Number.isFinite(parentWeight) && parentWeight > 0, 'Invalid parent weight: ' + parentId);
      const report = {parentId, parentWeight, status: 'PENDING', K: 0, variants: [], monChecks: []};
      completion.parents.push(report);
      const sourceSpecies = parent.sets.map(mon => norm(mon.species));
      check(sourceSpecies.every(Boolean) && new Set(sourceSpecies).size === 6, 'Repeated parent species: ' + parentId);
      const solved = parent.sets.map(template => {
        const slot = roster.findIndex(mon => norm(mon.species) === norm(template.species));
        check(slot >= 0, 'Parent species is absent from the public roster: ' + parentId);
        return solveMon(template, roster[slot], slot, report);
      });
      if (solved.some(mon => mon.report.ties === 0)) {
        report.status = 'NO_IV_SOLUTION_WITH_FIXED_TEMPLATE';
        continue;
      }
      let K = 1;
      for (const mon of solved) {
        check(mon.report.ties <= maxVariantsPerSource && K <= Math.floor(maxVariantsPerSource / mon.report.ties),
          'Per-source variant limit exceeded: ' + parentId, 'VARIANT_LIMIT_EXCEEDED');
        K *= mon.report.ties;
      }
      report.K = K;
      const share = parentWeight / K;
      check(Number.isFinite(share) && share > 0, 'Unrepresentable variant weight: ' + parentId);
      let products = [[]];
      for (const mon of solved) products = products.flatMap(prefix =>
        mon.solutions.map(ivs => [...prefix, {...copy(mon.template), ivs}]));
      check(products.length === K, 'Incomplete variant enumeration', 'MODEL_ENGINE_MISMATCH');
      for (const [variantIndex, sets] of products.entries()) {
        const {id: unusedId, weight: unusedWeight, ...metadata} = parent;
        const identity = {...metadata, sets}, key = canonical(identity);
        let entry = generated.get(key);
        if (!entry) {
          check(generated.size < maxTotalVariants, 'Total variant limit exceeded', 'VARIANT_LIMIT_EXCEEDED');
          const candidateId = 'EXACT_HP_IV_' + require('node:crypto').createHash('sha256').update(key).digest('hex');
          check(!byId.has(candidateId) && (!generatedIdKeys.has(candidateId) || generatedIdKeys.get(candidateId) === key),
            'Generated candidate ID collision', 'MODEL_ENGINE_MISMATCH');
          generatedIdKeys.set(candidateId, key);
          entry = {team: {...identity, id: candidateId, weight: 0}, contributions: []};
          generated.set(key, entry);
        }
        entry.team.weight += share;
        check(Number.isFinite(entry.team.weight) && entry.team.weight > 0, 'Generated weight overflow');
        entry.contributions.push({parentId, variantIndex, weight: share});
        report.variants.push(entry.team.id);
      }
      expectedWeight += parentWeight;
      check(Number.isFinite(expectedWeight), 'Parent weight sum overflow');
      report.status = 'GENERATED';
    }
    if (!generated.size) return fail('NO_IV_SOLUTION_WITH_FIXED_TEMPLATE', 'No parent has a solution at its fixed EVs and public constraints');
    completion.generated = [...generated.values()].map(entry => ({id: entry.team.id,
      weight: entry.team.weight, contributions: entry.contributions}));
    completion.totalWeight = completion.generated.reduce((sum, row) => sum + row.weight, 0);
    completion.expectedWeight = expectedWeight;
    check(Number.isFinite(completion.totalWeight) && Math.abs(completion.totalWeight - expectedWeight) <=
      Number.EPSILON * Math.max(completion.totalWeight, expectedWeight) * 8 * Math.max(1, completion.generated.length),
    'Generated prior mass differs from parent mass', 'MODEL_ENGINE_MISMATCH');
    const completedPool = {...pool, teams: [...pool.teams.filter(team => !parents.has(team.id)),
      ...[...generated.values()].map(entry => entry.team)]};
    const final = prepareExactHpPosition({...options, poolJson: JSON.stringify(completedPool)});
    owned = final.searcher;
    completion.certification = {status: final.status, beforeBelief: final.beforeBelief,
      afterBelief: final.afterBelief, excludedIds: final.excludedIds, error: final.error ?? null};
    const expectedIds = completion.generated.map(row => row.id).sort();
    check(final.status === 'READY' && owned && final.excludedIds.length === 0 &&
      JSON.stringify([...final.afterBelief.candidates].sort()) === JSON.stringify(expectedIds),
    'Generated candidates failed exact WASM certification', 'MODEL_ENGINE_MISMATCH');
    answer = {...final, completion};
    owned = null;
    return answer;
  } catch (error) {
    return fail(error.completionStatus || 'ENGINE_ERROR', String(error?.message || error));
  } finally {
    if (owned) {
      try { owned.free(); }
      catch (error) { if (answer) answer.cleanupError = String(error?.message || error); }
    }
  }
}

module.exports = Object.freeze({prepareExactHpPosition, completeExactHpPosition});
