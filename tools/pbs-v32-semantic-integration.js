'use strict';

const assert = require('node:assert/strict');
const {createBridge} = require('./pbs-converter-bridge');
const {exportCandidate} = require('./pbs-v32-position-full-audit');

const id = x => String(x || '')
  .toLowerCase().replace(/[^a-z0-9]/g, '');
const same = (a,b) => JSON.stringify(a) === JSON.stringify(b);
const copy = x => JSON.parse(JSON.stringify(x));

const SEMANTIC_MON = [
  'boosts', 'volatiles', 'rest', 'slept', 'tox_counter',
  'comp_brn', 'comp_par', 'locked', 'charging', 'must_recharge',
  'transformed_into', 'mimic_overlay', 'stall_streak',
  'last_protect_turn', 'protected_this_turn'
];

let bridge;

function integrateV32Position({spec,rawLlogHtml,pdat,pickedRosterSlots}) {
  assert.ok(spec?.sides?.length === 2, 'base PositionSpec required');

  const report = {
    applied: false, changes: [], conflicts: [], status: 'SKIPPED'
  };

  if (spec.team_preview || !rawLlogHtml) return {spec,report};

  if (!bridge) bridge = createBridge();

  const oracle = bridge.parseHtml(rawLlogHtml);
  report.eventCounts = oracle.eventCounts;

  if (oracle.errors.length || oracle.unknown.length) {
    report.status = 'CONVERTER_UNCLEAN';
    report.conflicts.push(
      'converter_errors=' + oracle.errors.length +
      ',unknown=' + oracle.unknown.length
    );
    return {spec,report};
  }

  const converted = exportCandidate({
    mySide:spec.side,
    spec,pdat,pickedRosterSlots,forceSwitch:spec.force_switch
  },oracle.protocol).spec;

  if (
    converted.side !== spec.side ||
    converted.sides.length !== spec.sides.length ||
    converted.sides.some((s,i) =>
      s.active !== spec.sides[i].active ||
      s.mons.length !== spec.sides[i].mons.length ||
      s.mons.some((m,j) =>
        id(m.species) !== id(spec.sides[i].mons[j].species) ||
        m.level !== spec.sides[i].mons[j].level
      )
    )
  ) {
    report.status = 'IDENTITY_MISMATCH';
    report.conflicts.push('active/roster/level mismatch');
    return {spec,report};
  }

  const merged = copy(spec);

  if (Object.prototype.hasOwnProperty.call(spec, '_sanitizeDiffs')) {
    Object.defineProperty(merged, '_sanitizeDiffs', {
      value: spec._sanitizeDiffs,
      writable: true,
      configurable: true,
      enumerable: false
    });
  }

  const set = (target,field,value,where) => {
    if (!same(target[field],value)) {
      report.changes.push({
        path:where+'.'+field,
        old:target[field],
        v32:value
      });
      target[field] = copy(value);
    }
  };

  for (let s=0;s<2;s++) {
    const dest = merged.sides[s];
    const src = converted.sides[s];

    for (let i=0;i<dest.mons.length;i++) {
      const mon = dest.mons[i];
      const tracked = src.mons[i];
      const where = `sides[${s}].mons[${i}]`;

      if (!tracked.appeared) continue;

      if (Boolean(tracked.fainted) !== Boolean(mon.fainted)) {
        report.conflicts.push(where+':fainted');
        continue;
      }

      for (const field of SEMANTIC_MON) {
        if (Object.hasOwn(tracked,field)) {
          set(mon,field,tracked[field],where);
        }
      }

      const uses = new Map();

      for (const item of [
        ...(mon.uses || []),
        ...(tracked.uses || [])
      ]) {
        if (
          !item?.move ||
          !Number.isInteger(item.n) ||
          item.n < 0
        ) continue;

        const key = id(item.move)
          .replace(/^hiddenpower.+$/, 'hiddenpower');

        const old = uses.get(key);

        uses.set(key, {
          move:key,
          n:Math.max(old?.n || 0,item.n)
        });
      }

      set(mon,'uses',[...uses.values()],where);

      const revealed = [...new Set([
        ...(mon.revealed_moves || []),
        ...(tracked.revealed_moves || [])
      ].map(x =>
        id(x).replace(/^hiddenpower.+$/, 'hiddenpower')
      ).filter(Boolean))];

      if (s !== spec.side) {
        set(mon,'revealed_moves',revealed,where);
      }

      if (
        tracked.last_move &&
        mon.last_move &&
        id(tracked.last_move) !== id(mon.last_move)
      ) {
        report.conflicts.push(
          where+':last_move:'+
          mon.last_move+'|'+tracked.last_move
        );
      } else if (tracked.last_move && !mon.last_move) {
        set(mon,'last_move',tracked.last_move,where);
      }

      if (
        s !== spec.side &&
        !mon.active &&
        !mon.status &&
        tracked.status
      ) {
        set(mon,'status',tracked.status,where);
      }

      for (const field of ['item_original','item_current']) {
        const v = tracked[field];
        if (v?.known && !mon[field]?.known) {
          set(mon,field,v,where);
        }
      }

      if (tracked.item_gained && !mon.item_gained) {
        set(mon,'item_gained',true,where);
      }
    }

    if (!same(dest.conditions,src.conditions)) {
      report.conflicts.push(`sides[${s}]:conditions`);
    }

    if (
      dest.last_move &&
      src.last_move &&
      id(dest.last_move) !== id(src.last_move)
    ) {
      report.conflicts.push(
        `sides[${s}]:last_move:${dest.last_move}|${src.last_move}`
      );
    } else if (!dest.last_move && src.last_move) {
      set(dest,'last_move',src.last_move,`sides[${s}]`);
    }
  }

  report.applied = report.changes.length > 0;
  report.status = report.conflicts.length
    ? 'OK_WITH_CONFLICTS'
    : 'OK';

  return {spec:merged,report};
}

module.exports = {
  integrateV32Position,
  SEMANTIC_MON
};
