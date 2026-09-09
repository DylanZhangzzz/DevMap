'use strict';
// Compatibility bridge: the reviewed transport/receipt logic remains in place.
// Requiring either module never starts a process or reads a corpus.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const crypto=require('node:crypto');
const {execFileSync}=require('node:child_process');
const legacy=require('./shared-summary-performance.cjs');
const api={...legacy,...legacy.runtime};
function validateDimensions(mode,dimensions) {
  assert(['smoke','acceptance'].includes(mode));
  if(mode==='acceptance')assert.deepEqual(dimensions,{worktrees:20,sessions:100,events:100000},'Acceptance requires the fixed 20/100/100000 corpus');
}
function createRun(config,sourceFiles,protocol) {
  assert.equal(process.platform,'win32','Windows named-pipe harness only');
  const receipt=api.validateReceipt(config);validateDimensions(config.mode,receipt.dimensions);
  const parent=api.checked(config.run_parent,'directory');
  assert(!api.within(parent.path,receipt.allocation_root)&&parent.path!==receipt.allocation_root);
  const input=api.checked(config.candidate,'file'), python=api.checked(config.python,'file').path;
  const space=fs.statfsSync(parent.path,{bigint:true});
  assert(space.bavail*space.bsize>=BigInt(256*1024*1024)+fs.statSync(input.path,{bigint:true}).size,'Insufficient owned run reserve');
  const run=fs.mkdtempSync(path.join(parent.path,'shared-freshness-')), runIdentity=api.checked(run,'directory');
  const exe=path.join(run,'devmap.exe'), build=api.hash(fs.readFileSync(input.path));
  fs.copyFileSync(input.path,exe,fs.constants.COPYFILE_EXCL);
  assert.equal(api.hash(fs.readFileSync(exe)),build);
  const ctx={config,receipt,exe,build,python,run,children:[],rows:[],bytes:0,asyncError:null};
  ctx.verifyRun=()=>assert.deepEqual(api.checked(run,'directory'),runIdentity,'Run replaced');
  ctx.verifyFixture=()=>assert.deepEqual(api.validateReceipt(config),receipt,'Receipt/fixture replaced');
  const logs=new Map();
  ctx.append=(name,bytes)=>{
    ctx.verifyRun();assert.equal(path.basename(name),name);ctx.bytes+=Buffer.byteLength(bytes);
    assert(ctx.bytes<=128*1024*1024,'Run evidence exceeds 128 MiB; retain failure');
    if(!logs.has(name))logs.set(name,fs.openSync(path.join(run,name),'wx'));
    const data=Buffer.isBuffer(bytes)?bytes:Buffer.from(bytes);let offset=0;
    while(offset<data.length){const n=fs.writeSync(logs.get(name),data,offset,data.length-offset);assert(n>0);offset+=n;}
  };
  ctx.closeLogs=()=>{for(const fd of logs.values())fs.closeSync(fd);logs.clear();};
  ctx.report={schema:'devmap/shared-summary-freshness/1',mode:config.mode,configuration:config,protocol,
    candidate_sha256:build,receipt_sha256:api.hash(fs.readFileSync(config.receipt)),source_hashes:{},
    owners:[],audits:[],warmups:[],trials:Array.from({length:protocol.measured},(_,index)=>({index,status:'not-executed'})),
    errors:[],cleanup_errors:[],completed:false,freshness_acceptance:false,performance_acceptance:false,
    scope:'Tracked-file dirty/clean visibility; natural TTL; uncontrolled warm OS cache; no HEAD/config/idle acceptance',
    started_at:new Date().toISOString()};
  ctx.save=()=>{ctx.verifyRun();const next=path.join(run,'report.next');fs.writeFileSync(next,JSON.stringify(ctx.report,null,2),{flag:'wx'});fs.renameSync(next,path.join(run,'report.json'));};
  fs.writeFileSync(path.join(run,'creation.json'),JSON.stringify({run:runIdentity,nonce:crypto.randomBytes(16).toString('hex'),candidate_sha256:build}),{flag:'wx'});
  for(const source of sourceFiles){const name=path.basename(source);ctx.report.source_hashes[name]=api.hash(fs.readFileSync(source));fs.copyFileSync(source,path.join(run,name),fs.constants.COPYFILE_EXCL);}
  ctx.save();console.log(JSON.stringify({run}));
  const db=path.join(receipt.common,'devmap/devmap.db');
  ctx.sidecars=()=>Object.fromEntries(['-wal','-shm','-journal'].map(s=>{const p=db+s;return[s,fs.existsSync(p)?{bytes:fs.statSync(p).size}:null];}));
  ctx.prepare=()=>{
    ctx.baseline=api.sqlState(python,db);ctx.immutable=api.inventory(receipt.immutable_roots);
    assert.deepEqual(ctx.baseline,receipt.baseline_sql);
    assert.deepEqual({entries:ctx.immutable.entries,sha256:ctx.immutable.sha256},receipt.baseline_immutable);
    for(const snapshot of ctx.baseline.activation_snapshot_paths){const backup=api.checked(snapshot,'directory').path;
      assert(receipt.immutable_roots.some(p=>{const root=api.checked(p,'directory').path;return root===backup||api.within(backup,root);}));}
    assert.equal(ctx.baseline.meta[1],receipt.repository_id);
    assert.equal(ctx.baseline.tables.journal_sessions.rows,receipt.dimensions.sessions);
    assert.equal(ctx.baseline.tables.journal_records.rows,receipt.dimensions.events);
    ctx.append('immutable-before.json',JSON.stringify(ctx.immutable));
    ctx.report.sqlite_sidecars_before=ctx.sidecars();
    ctx.runtimeIdentity=JSON.parse(execFileSync(exe,['runtime','--identity','--source',receipt.source],{encoding:'utf8',timeout:15000,windowsHide:true}));
    assert.match(ctx.runtimeIdentity.repository,/^[a-f0-9]{64}$/);
    assert.equal(api.checked(ctx.runtimeIdentity.source,'directory').path,api.checked(receipt.source,'directory').path);
    assert.equal(api.checked(ctx.runtimeIdentity.common,'directory').path,api.checked(receipt.common,'directory').path);
    assert(receipt.worktrees.some(w=>api.checked(w.git_dir,'directory').path===api.checked(ctx.runtimeIdentity.git_dir,'directory').path));
    ctx.report.runtime_identity=ctx.runtimeIdentity;ctx.report.dimensions=receipt.dimensions;ctx.save();
  };
  ctx.preserve=()=>{
    ctx.verifyFixture();
    if(ctx.baseline)assert.deepEqual(api.sqlState(python,db),ctx.baseline);
    if(ctx.immutable)assert.deepEqual(api.inventory(receipt.immutable_roots),ctx.immutable);
  };
  ctx.health=()=>{if(ctx.asyncError)throw ctx.asyncError;ctx.verifyRun();};
  return ctx;
}
module.exports={...api,createRun,validateDimensions};
