// Validate the shipped catalog's complete grid against the real WASM identity.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
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
  if(artifact.status)throw Error('catalog preview tables are retired');
  const {compiledPreviewCompatibility,compiledPreviewPick}=await import('/src/compiled-preview.ts');
  const compatible = await compiledPreviewCompatibility(poolJson,embeddedDexJson());
  let invalidKeys = new Set(), validKeys = new Set();
  const keys=new Set();let matchups=0;
  try{
   for(const own of catalog.teams)for(const foe of catalog.teams){
    const b=new Battle(dex,JSON.stringify(foe.sets),JSON.stringify(own.sets),'1,2,3,4');
    const s=new BlindSearcher(b,1,poolJson,7,.4);
    try{
     s.observe(b);
     const key=s.compiledPreviewKey(b);keys.add(key);
     const legal=JSON.parse(b.legalChoices(1));
     const build=s.compiledPreviewBuild(), info=s.previewCacheInfo();
     for(const seed of Array.from({length:64},(_,i)=>i)){
      const choice=compiledPreviewPick(build,key,seed,.4,info,compatible);
      if(compatible.has(key)) {
        validKeys.add(key);
        if(!choice||!legal.some(c=>c.input===choice))throw Error(`invalid preview use ${own.id} vs ${foe.id}`);
      } else {invalidKeys.add(key); if(choice)throw Error('stale preview returned');}
     }
     matchups++;
    }finally{s.free();b.free();}
   }
  }finally{dex.free();}
  return {matchups,uniqueKeys:keys.size,validKeys:validKeys.size,invalidKeys:invalidKeys.size,slotsPerKey:64,slotChecks:matchups*64,allLegal:true};
 },wasmPath);
 assert.equal(result.matchups,1089);assert.equal(result.uniqueKeys,990);
 const out=process.env.NC2000_CHECK_OUT ?? '../data/preview-training-v3';
 fs.mkdirSync(out,{recursive:true});
 fs.writeFileSync(path.join(out,'compiled-coverage.json'),JSON.stringify(result,null,2)+'\n');console.log(result);
}finally{await browser?.close();await vite?.close();server.close();}
