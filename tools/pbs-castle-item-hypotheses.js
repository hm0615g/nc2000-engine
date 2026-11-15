"use strict";
// Only the adapted opponent hypothesis is repaired; public observations,
// native search logic and normal belief-pool teams are never rewritten.
const ITEM_BITS = new Map([
  ['leftovers',1],['mintberry',2],['miracleberry',4],['przcureberry',8],
  ['psncureberry',16],['iceberry',32],['burntberry',64],['bitterberry',128],
  ['berry',256],['goldberry',512],['berryjuice',1024],['berserkgene',2048]
]);
const SPECIES_ONLY = new Map([
  ['thickclub','marowak'],['lightball','pikachu'],
  ['metalpowder','ditto'],['stick','farfetchd']
]);
const id = v => String(v || '').normalize('NFD').toLowerCase().replace(/[^a-z0-9]/g,'');
const copy = v => JSON.parse(JSON.stringify(v));
function needsRepair(sets, mons) {
  return sets.some((set, slot) => {
    const mon=mons[slot], item=id(set.item);
    if (!mon || Boolean(item)!==(mon.item_flag===true)) return true;
    if (mon.item_original?.known===true && item!==id(mon.item_original.item)) return true;
    if (item && (mon.excluded_items || 0) & (ITEM_BITS.get(item) || 0)) return true;
    return item && mons.some((other,j) => j!==slot &&
      other.item_original?.known===true && id(other.item_original.item)===item);
  });
}
function choices(source, mon, baseTeams) {
  if (mon.item_original?.known===true) {
    const item=mon.item_original.item || '';
    return Boolean(id(item))===(mon.item_flag===true)
      ? [{name:item,key:id(item),origin:'PUBLIC_ORIGINAL',score:1e12}] : [];
  }
  if (mon.item_flag!==true) return [{name:'',key:'',origin:'PUBLIC_NO_ITEM',score:1e12}];
  const species=id(source.species), level=Number(source.level || 50);
  const selected=new Map();
  const add=(item,origin,score) => {
    const key=id(item), bit=ITEM_BITS.get(key)||0;
    if (!key || selected.has(key) || (bit && ((mon.excluded_items||0)&bit)) ||
        (SPECIES_ONLY.has(key) && SPECIES_ONLY.get(key)!==species)) return;
    selected.set(key,{name:item,key,origin,score});
  };
  add(source.item,'ORIGINAL_HYPOTHESIS',1e12);
  const exact=[], other=[], generic=[];
  for (const team of baseTeams) {
    const weight=Number(team.weight)>0?Number(team.weight):1;
    for (const set of team.sets||[]) {
      if (!set.item) continue;
      const same=id(set.species)===species;
      const entry={item:set.item,score:weight};
      (same && Number(set.level||50)===level ? exact : same ? other : generic).push(entry);
    }
  }
  const byWeight=(a,b)=>b.score-a.score || id(a.item).localeCompare(id(b.item));
  for (const v of exact.sort(byWeight)) add(v.item,'SAME_SPECIES_LEVEL_POOL',v.score);
  for (const v of other.sort(byWeight)) add(v.item,'SAME_SPECIES_POOL',v.score);
  for (const v of generic.sort(byWeight)) add(v.item,'GENERIC_EXISTING_POOL',v.score);
  return [...selected.values()].slice(0,36);
}
function repairAdaptedTeamItems({teams, baseTeams, positionSpec, maxVariants=3}) {
  if (!positionSpec || !Array.isArray(teams) || !Array.isArray(baseTeams) ||
      !Array.isArray(positionSpec.sides) || ![0,1].includes(positionSpec.side))
    throw Error('INVALID_PUBLIC_ITEM_REPAIR_INPUT');
  const publicMons=positionSpec.sides[1-positionSpec.side]?.mons;
  if (!Array.isArray(publicMons) || publicMons.length!==6) throw Error('PUBLIC_ROSTER_REQUIRED');
  const output=[];
  const repairs=[];
  for (const team of teams) {
    if (!String(team.id||'').startsWith('OPPONENT_ADAPTED_') ||
        !Array.isArray(team.sets) || team.sets.length!==6 ||
        !needsRepair(team.sets,publicMons)) {
      output.push(team);
      continue;
    }
    if (!team.sets.every((s,i)=>id(s.species)===id(publicMons[i].species))) {
      throw Error('ADAPTED_ROSTER_SPECIES_ORDER_MISMATCH');
    }
    const options=team.sets.map((set,i)=>choices(set,publicMons[i],baseTeams));
    const order=[0,1,2,3,4,5].sort((a,b)=>
      Number(publicMons[b].item_original?.known===true)-Number(publicMons[a].item_original?.known===true) ||
      options[a].length-options[b].length || a-b);
    const assigned=Array(6).fill(null), used=new Set(), solutions=[];
    let explored=0, exhausted=false;
    const maxNodes=100000;
    function search(depth, changes, allowance) {
      if (explored++ >= maxNodes) {exhausted=true;return;}
      if (solutions.length>=maxVariants) return;
      if (depth===6) {
        if (changes===allowance) solutions.push(assigned.map(x=>({...x})));
        return;
      }
      const slot=order[depth];
      for (const option of options[slot]) {
        if (option.key && used.has(option.key)) continue;
        const delta=option.key===id(team.sets[slot].item)?0:1;
        if (changes+delta>allowance) continue;
        assigned[slot]=option;
        if (option.key) used.add(option.key);
        search(depth+1,changes+delta,allowance);
        if (option.key) used.delete(option.key);
        assigned[slot]=null;
        if (solutions.length>=maxVariants || exhausted) break;
      }
    }
    for (let budget=0;budget<=4 && !solutions.length && !exhausted;budget++) search(0,0,budget);
    if (!solutions.length) {
      // Fail closed. The unchanged hypothesis will be rejected by WASM if
      // inconsistent; never suppress the real public-HP safety gate.
      output.push(team);
      repairs.push({id:team.id,status:'NO_CERTIFIED_ITEM_ASSIGNMENT',exhausted});
      continue;
    }
    const weight=Number(team.weight)>0?Number(team.weight):1;
    for (let v=0;v<solutions.length;v++) {
      const sets=team.sets.map((set,i)=>({...set,item:solutions[v][i].name}));
      const changed=sets.map((set,i)=>id(set.item)!==id(team.sets[i].item)?i:-1).filter(i=>i>=0);
      if (needsRepair(sets,publicMons)) throw Error('PUBLIC_ITEM_REPAIR_STILL_INCONSISTENT');
      output.push({...team,id:team.id+'__PUBLIC_ITEMS_'+String(v+1).padStart(2,'0'),
        sets,weight:weight/solutions.length});
      repairs.push({id:team.id,variant:v+1,changedSlots:changed,status:'HYPOTHESIS_NOT_FACT'});
    }
  }
  return {teams:output,repairs};
}
module.exports=Object.freeze({repairAdaptedTeamItems});
