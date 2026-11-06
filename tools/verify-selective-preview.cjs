// Compare selective preview output with the actual previous artifact using WASM keys.
const fs=require('node:fs'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const wasm=require('../crates/wasm/pkg-node/nc2000_wasm.js');
const read=p=>JSON.parse(fs.readFileSync(p,'utf8'));
if (!process.argv[2]) throw new Error('Usage: node tools/verify-selective-preview.cjs BASELINE_ARTIFACT [REPORT]');
const old=read(process.argv[2]),current=read('data/compiled-preview-v1.json');
const catalog=read('data/team-pool-v2/team-pool.json').teams;
const dex=new wasm.Dex(),ownKeys=new Set(),publicKeys=new Set();
const pool=fs.readFileSync('data/belief-pool-v3/belief-pool.json','utf8');
for(const team of catalog.filter(t=>t.nash)) {
 const json=JSON.stringify(team.sets),b=new wasm.Battle(dex,json,json,'1,2,3,4');
 const s=new wasm.BlindSearcher(b,0,pool,7,.4);
 const [own,foe]=JSON.parse(s.compiledPreviewKey(b));ownKeys.add(JSON.stringify(own));publicKeys.add(JSON.stringify(foe));
 s.free();b.free();
}
let affected=0,unchangedUnrelated=0,changed=0;
for(const [k,v] of Object.entries(current.policies)) {
 const [own,foe]=JSON.parse(k);
 if(ownKeys.has(JSON.stringify(own))||publicKeys.has(JSON.stringify(foe)))affected++;
 else{assert.deepEqual(v,old.policies[k]);unchangedUnrelated++;}
 changed+=JSON.stringify(v)!==JSON.stringify(old.policies[k]);
}
assert.equal(affected,149);assert.equal(unchangedUnrelated,841);
const report={affectedPolicyKeys:affected,unchangedUnrelatedPolicyKeys:unchangedUnrelated,changedPolicyChoices:changed,
 artifactSha256:crypto.createHash('sha256').update(fs.readFileSync('data/compiled-preview-v1.json')).digest('hex'),partialRebake:current.partialRebake};
fs.writeFileSync(process.argv[3] || 'target/selective-preview-verification.json',JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({affected,unchangedUnrelated,changed}));dex.free();
