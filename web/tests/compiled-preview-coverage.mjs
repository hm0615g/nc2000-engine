// Validate the shipped catalog's complete grid against the real WASM identity.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {fileURLToPath} from 'node:url';
const wasmPath=fileURLToPath(new URL('../../crates/wasm/pkg-web/nc2000_wasm.js',import.meta.url));
import {createServer as http} from 'node:http';
import {createServer} from 'vite';
import {chromium} from '@playwright/test';
const server=http();let vite,browser;
try {
 await new Promise(r=>server.listen(0,'127.0.0.1',r));
 vite=await createServer({server:{middlewareMode:true,hmr:{server}}});server.on('request',vite.middlewares);
 browser=await chromium.launch();const page=await browser.newPage();await page.goto(`http://127.0.0.1:${server.address().port}/`,{waitUntil:'networkidle'});
 const result=await page.evaluate(async(wasmPath)=>{
  const {default:init,Dex,Battle,BlindSearcher,embeddedDexJson}=await import('/@fs'+wasmPath);
  await init();const dex=new Dex();
  const catalog=await(await fetch('/data/team-pool-v2/team-pool.json')).json();
  const poolJson=await(await fetch('/data/belief-pool-v3/belief-pool.json')).text();
  const artifact=await(await fetch('/data/compiled-preview-v1.json')).json();
  const retired=artifact.status==='retired-training-v3';
  const {compiledPreviewMatches,compiledPreviewPick}=await import('/src/compiled-preview.ts');
  if(await compiledPreviewMatches(poolJson,embeddedDexJson())===retired)throw Error('preview retirement/compatibility mismatch');
  const keys=new Set();let matchups=0;
  try{
   for(const own of catalog.teams)for(const foe of catalog.teams){
    const b=new Battle(dex,JSON.stringify(foe.sets),JSON.stringify(own.sets),'1,2,3,4');
    const s=new BlindSearcher(b,1,poolJson,7,.4);
    try{
     s.observe(b);
     const key=s.compiledPreviewKey(b);keys.add(key);
     const legal=JSON.parse(b.legalChoices(1));
     for(const seed of [0,1]){
      const choice=compiledPreviewPick(s.compiledPreviewBuild(),key,seed,.4,s.previewCacheInfo());
      if(retired ? choice!==undefined : !choice||!legal.some(c=>c.input===choice))throw Error(`invalid preview use ${own.id} vs ${foe.id}`);
     }
     matchups++;
    }finally{s.free();b.free();}
   }
  }finally{dex.free();}
  if(retired){
   const {BotWorker}=await import('/src/bot.ts');const worker=[];
   for(const [own,foe] of [[catalog.teams[0],catalog.teams[1]],[catalog.teams[3],catalog.teams[6]]]){
    const bot=new BotWorker();try{
     await bot.newBattle(JSON.stringify(foe.sets),JSON.stringify(own.sets),'1,2,3,4',{poolJson,side:1,seed:7});
     const r=await bot.search(1,32,99,true);
     if(r.src!=='search'||r.policy.iterations!==32||!r.best)throw Error('retired preview did not use live search');
     worker.push({own:own.id,foe:foe.id,source:r.src,iterations:r.policy.iterations});
    }finally{bot.terminate();}
   }
   return {matchups,uniqueKeys:keys.size,allLegal:true,retired,worker};
  }
  // The approved optimization must retain the worker's zero-search table
  // path, including the six sets whose public gender changed.
  const {BotWorker}=await import('/src/bot.ts');
  const inventory=await(await fetch('/data/team-inventory-v1/inventory.json')).json();
  const evolved=await(await fetch('/data/team-selection-v2/evolved-teams.json')).json();
  const partybox=await(await fetch('/data/partybox-v0/teams.json')).json();
  const original=new Map([...inventory.teams,...evolved.teams,...partybox.teams].map(t=>[t.id,t.sets]));
  const optimized=catalog.teams.find(t=>JSON.stringify(t.sets)!==JSON.stringify(original.get(t.id)));
  const unchanged=catalog.teams.find(t=>JSON.stringify(t.sets)===JSON.stringify(original.get(t.id)));
  const female=catalog.teams.find(t=>t.sets.some((p,i)=>p.gender!==original.get(t.id)[i].gender));
  if(!optimized||!unchanged||!female)throw Error('optimized catalog fixtures missing');
  const worker=[];
  for(const [own,foe] of [[optimized,unchanged],[unchanged,female]]){
   const bot=new BotWorker();
   try{
    await bot.newBattle(JSON.stringify(foe.sets),JSON.stringify(own.sets),'1,2,3,4',{poolJson,side:1,seed:7});
    const r=await bot.search(1,32,99,true);
    if(r.src!=='table'||r.policy.iterations!==0||!r.best)throw Error('optimized catalog fell back to search');
    worker.push({own:own.id,foe:foe.id,source:r.src,iterations:r.policy.iterations});
   }finally{bot.terminate();}
  }
  return {matchups,uniqueKeys:keys.size,allLegal:true,worker};
 },wasmPath);
 assert.equal(result.matchups,1089);assert.equal(result.uniqueKeys,990);
 fs.writeFileSync(`${process.env.NC2000_CHECK_OUT ?? '../findings/fixed-budget-release-v1'}/compiled-coverage.json`,JSON.stringify(result,null,2)+'\n');console.log(result);
}finally{await browser?.close();await vite?.close();server.close();}
