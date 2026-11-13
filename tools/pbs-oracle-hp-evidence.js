'use strict';

const toId = x => String(x || '')
  .normalize('NFD')
  .toLowerCase()
  .replace(/[^a-z0-9]/g, '');

function planOracleBenchHp({
  oracle, foeSide, previewMons, seenFoeMons, top
}) {
  const result = {
    changes: [],
    skipped: [],
    conflicts: [],
    topChecks: 0
  };

  if (
    !oracle ||
    !Array.isArray(oracle.errors) ||
    !Array.isArray(oracle.unknown) ||
    oracle.errors.length ||
    oracle.unknown.length
  ) {
    result.conflicts.push('oracle_not_clean');
    return result;
  }

  if (
    ![0, 1].includes(foeSide) ||
    !Array.isArray(previewMons) ||
    !Array.isArray(seenFoeMons) ||
    previewMons.length !== seenFoeMons.length
  ) {
    result.conflicts.push('roster_missing_or_mismatched');
    return result;
  }

  const sideName = foeSide === 0 ? 'p1' : 'p2';
  const health = oracle.health?.[sideName];

  if (!health || typeof health !== 'object') {
    result.conflicts.push('oracle_health_missing');
    return result;
  }

  const rosterIds = previewMons.map(
    x => toId(x.speciesId || x.species)
  );

  const activeFromTop = toId(top?.speciesId);
  const activeFromOracle = toId(oracle.active?.[sideName]);

  for (const [english, observed] of Object.entries(health)) {
    const speciesId = toId(english);

    const positions = rosterIds.flatMap(
      (id, i) => id === speciesId ? [i] : []
    );

    if (positions.length !== 1) {
      result.skipped.push({
        species: english,
        reason: 'roster_not_unique',
        matches: positions.length
      });
      continue;
    }

    const slot = positions[0];
    const remembered = seenFoeMons[slot];
    const hp = observed?.hpAbs;

    if (!Number.isSafeInteger(hp) || hp < 0) {
      result.skipped.push({
        species: english,
        reason: 'no_observed_absolute_hp'
      });
      continue;
    }

    if (activeFromTop && speciesId === activeFromTop) {
      if (Number.isSafeInteger(top?.hp)) {
        result.topChecks++;

        if (top.hp !== hp) {
          result.conflicts.push(
            'top_hp_disagrees:' + english +
            ':oracle=' + hp +
            ':top=' + top.hp
          );
        }
      }

      result.skipped.push({
        species: english,
        reason: 'top_screen_is_authority'
      });
      continue;
    }

    if (activeFromOracle && speciesId === activeFromOracle) {
      result.skipped.push({
        species: english,
        reason: 'oracle_active_without_top_match'
      });
      continue;
    }

    // Converter maxHp can be only a lower bound.
    // Use maxHp previously observed independently from PBS.
    if (
      !remembered ||
      !Number.isSafeInteger(remembered.maxHp) ||
      remembered.maxHp <= 0
    ) {
      result.skipped.push({
        species: english,
        reason: 'no_independently_observed_max_hp'
      });
      continue;
    }

    if (hp > remembered.maxHp) {
      result.conflicts.push(
        'hp_out_of_range:' + english +
        ':hp=' + hp +
        ':known_max=' + remembered.maxHp
      );
      continue;
    }

    if (remembered.fainted && hp > 0) {
      result.conflicts.push(
        'fainted_but_positive_hp:' + english
      );
      continue;
    }

    if (
      remembered.hp === hp &&
      Boolean(remembered.fainted) === (hp === 0)
    ) {
      result.skipped.push({
        species: english,
        reason: 'already_agrees'
      });
      continue;
    }

    result.changes.push({
      species: english,
      slot,
      from: remembered.hp ?? null,
      to: hp,
      maxHp: remembered.maxHp,
      fainted: hp === 0
    });
  }

  return result;
}

function applyPlannedBenchHp(seenFoeMons, plan) {
  if (
    !plan ||
    !Array.isArray(plan.changes) ||
    !Array.isArray(plan.conflicts) ||
    plan.conflicts.length
  ) return 0;

  let applied = 0;

  for (const change of plan.changes) {
    const mon = seenFoeMons[change.slot];

    if (
      !mon ||
      mon.maxHp !== change.maxHp ||
      mon.hp !== change.from ||
      !Number.isSafeInteger(change.to) ||
      change.to < 0 ||
      change.to > mon.maxHp
    ) continue;

    mon.hp = change.to;

    if (change.fainted) {
      mon.fainted = true;
      mon.status = '';
    }

    applied++;
  }

  return applied;
}

module.exports = {
  planOracleBenchHp,
  applyPlannedBenchHp
};
