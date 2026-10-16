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
  const {compiledPreviewMatches,compiledPreviewPick}=await import('/src/compiled-preview.ts');
  if(!await compiledPreviewMatches(poolJson,embeddedDexJson()))throw Error('shipped prior rejected');
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
      if(!choice||!legal.some(c=>c.input===choice))throw Error(`missing/illegal ${own.id} vs ${foe.id}`);
     }
     matchups++;
    }finally{s.free();b.free();}
   }
  }finally{dex.free();}
  return {matchups,uniqueKeys:keys.size,allLegal:true};
 },wasmPath);
 assert.equal(result.matchups,576);assert.equal(result.uniqueKeys,504);
 fs.writeFileSync('../findings/fixed-budget-release-v1/compiled-coverage.json',JSON.stringify(result,null,2)+'\n');console.log(result);
}finally{await browser?.close();await vite?.close();server.close();}
