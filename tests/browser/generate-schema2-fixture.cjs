'use strict';
// Explicit orchestrator for a precompiled ignored Rust test. Never runs Cargo.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),net=require('node:net');
const {spawn,execFileSync}=require('node:child_process');
const {setTimeout:delay}=require('node:timers/promises');
const {checked,inventory,sqlState,within,envelope}=require('./shared-summary-performance.cjs');
const sha=b=>crypto.createHash('sha256').update(b).digest('hex');
const RESERVE=512n*1024n*1024n,LOG_RESERVE=128n*1024n*1024n;
const PHYSICAL=String.raw`
import ctypes,sys,json
from ctypes import wintypes as w
class INFO(ctypes.Structure):
 _fields_=[('attributes',w.DWORD),('created',w.FILETIME),('accessed',w.FILETIME),('written',w.FILETIME),('volume',w.DWORD),('size_high',w.DWORD),('size_low',w.DWORD),('links',w.DWORD),('index_high',w.DWORD),('index_low',w.DWORD)]
k=ctypes.WinDLL('kernel32',use_last_error=True)
k.CreateFileW.argtypes=[w.LPCWSTR,w.DWORD,w.DWORD,ctypes.c_void_p,w.DWORD,w.DWORD,w.HANDLE];k.CreateFileW.restype=w.HANDLE
k.GetFileInformationByHandle.argtypes=[w.HANDLE,ctypes.POINTER(INFO)];k.GetFileInformationByHandle.restype=w.BOOL
k.CloseHandle.argtypes=[w.HANDLE];k.CloseHandle.restype=w.BOOL
h=k.CreateFileW(sys.argv[1],0x80,7,None,3,0x02200000,None)
assert h!=ctypes.c_void_p(-1).value, ctypes.get_last_error()
try:
 i=INFO();assert k.GetFileInformationByHandle(h,ctypes.byref(i)),ctypes.get_last_error()
 assert i.attributes & 0x10 and not i.attributes & 0x400
 print('windows:'+str(i.volume)+':'+str((i.index_high<<32)|i.index_low))
finally:k.CloseHandle(h)
`;
function dimensions(config){
  if(config.mode==='scale')return {worktrees:20,sessions:100,events_per_session:1000};
  assert.equal(config.mode,'smoke');const d=config.dimensions||{worktrees:2,sessions:2,events_per_session:3};
  assert(Number.isInteger(d.worktrees)&&d.worktrees>=1&&d.worktrees<=4);
  assert(Number.isInteger(d.sessions)&&d.sessions>=d.worktrees&&d.sessions<=8&&d.sessions%d.worktrees===0);
  assert(Number.isInteger(d.events_per_session)&&d.events_per_session>=1&&d.events_per_session<=20);return d;
}
function available(p){const s=fs.statfsSync(p,{bigint:true});return s.bavail*s.bsize;}
function preflight(config){
  assert.equal(process.platform,'win32');assert.equal(config.approved_private_parent,true,'Root must pre-approve parent ACL; this script never changes system or parent ACLs');
  const parent=checked(config.parent,'directory'),candidate=checked(config.candidate,'file'),test=checked(config.test_executable,'file'),python=checked(config.python,'file'),trusted_temp=checked(config.trusted_temp,'directory');
  const job_wrapper=checked(path.join(__dirname,'windows-owned-generator-job.py'),'file'),job_wrapper_sha256=sha(fs.readFileSync(job_wrapper.path));
  assert.equal(sha(fs.readFileSync(candidate.path)),config.candidate_sha256);assert.equal(sha(fs.readFileSync(test.path)),config.test_executable_sha256);
  const dims=dimensions(config),payload=BigInt(dims.sessions*dims.events_per_session*1024+8192);
  const required=RESERVE+LOG_RESERVE+payload*8n+fs.statSync(candidate.path,{bigint:true}).size;
  const free=available(parent.path);assert(free>=required,'Preflight disk estimate cannot preserve 512 MiB + future log/candidate reserve');
  assert(config.evaluation_time&&Number.isFinite(Date.parse(config.evaluation_time)),'Fixed evaluation_time required');
  return {parent,candidate,test,python,trusted_temp,job_wrapper,job_wrapper_sha256,dims,available_bytes:String(free),required_bytes:String(required)};
}
function createNew(parent,prefix){const nonce=crypto.randomBytes(16).toString('hex'),native=path.join(parent,prefix+nonce);fs.mkdirSync(native);return {nonce,native,identity:checked(native,'directory')};}
function jsonNew(file,value){fs.writeFileSync(file,JSON.stringify(value,null,2),{flag:'wx'});}
function environment(allocation,trustedTemp){
  const env={...process.env};for(const key of Object.keys(env))if(key.toUpperCase().startsWith('GIT_'))delete env[key];
  // Existing user configuration/state is never a generation target. Direct
  // legacy generation does not require these directories to exist beforehand.
  return {...env,HOME:path.join(allocation,'profile'),USERPROFILE:path.join(allocation,'profile'),XDG_CONFIG_HOME:path.join(allocation,'profile/xdg'),
    LOCALAPPDATA:path.join(allocation,'state'),XDG_STATE_HOME:path.join(allocation,'state'),TEMP:trustedTemp,TMP:trustedTemp,GIT_TERMINAL_PROMPT:'0',GIT_NO_LAZY_FETCH:'1',GIT_NO_REPLACE_OBJECTS:'1'};
}
function owned(exe,args,label,ctx,stdio=['ignore','pipe','pipe']){
  ctx.verify();
  const expected=exe===ctx.checked.test.path?ctx.config.test_executable_sha256:ctx.config.candidate_sha256;
  assert.equal(sha(fs.readFileSync(exe)),expected,'Reviewed executable changed before launch');
  let jobReport=null;
  if(label==='rust-generator'){
    jobReport=path.join(ctx.run,'rust-job.json');assert(!fs.existsSync(jobReport));
    assert.equal(sha(fs.readFileSync(ctx.checked.job_wrapper.path)),ctx.checked.job_wrapper_sha256,'Job wrapper changed before launch');
    args=[ctx.checked.job_wrapper.path,'--report',jobReport,'--exe',exe,'--',...args];
    exe=ctx.checked.python.path;stdio=['pipe','pipe','pipe'];
  }
  const child=spawn(exe,args,{stdio,windowsHide:true,env:ctx.env});
  const entry={child,label,jobReport,exit:null,error:null,stopping:false};ctx.children.push(entry);
  if(jobReport)child.stdin.on('error',e=>{if(!entry.exit)ctx.error=e;});
  entry.closed=new Promise(resolve=>{child.once('error',e=>{entry.error=e;resolve();});child.once('close',(code,signal)=>{entry.exit={code,signal};resolve();});});
  child.stderr.on('data',b=>{try{ctx.append(`${label}.stderr`,b);}catch(e){ctx.error=e;}});child.stderr.on('error',e=>{ctx.error=e;});return entry;
}
async function stop(entry){entry.stopping=true;
  if(entry.child.exitCode===null&&entry.child.signalCode===null&&!entry.error){
    if(entry.jobReport)entry.child.stdin.end('abort\n');else entry.child.kill();
  }
  let timer;try{await Promise.race([entry.closed,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error(`Owned child not reaped: ${entry.label}`)),entry.jobReport?12000:5000);})]);}
  catch(error){if(entry.jobReport&&entry.child.exitCode===null){entry.child.kill();
    let finalTimer;try{await Promise.race([entry.closed,new Promise((_,reject)=>{finalTimer=setTimeout(()=>reject(new Error('Job wrapper forced close not reaped')),5000);})]);}finally{clearTimeout(finalTimer);}}
    throw error;}
  finally{clearTimeout(timer);}
  if(entry.jobReport&&!entry.error){entry.job=JSON.parse(fs.readFileSync(entry.jobReport,'utf8'));assert.equal(entry.job.empty_confirmed,true,'Job emptiness was not confirmed');}
}
function validateManifest(manifest,allocation,python){
  assert(Array.isArray(manifest.worktrees)&&Array.isArray(manifest.immutable_roots));
  const dirs=[manifest.source,manifest.common,...manifest.worktrees.flatMap(w=>[w.root,w.git_dir]),...manifest.immutable_roots];
  for(const p of dirs)assert(within(checked(p,'directory').path,allocation),'Manifest directory outside owned allocation');
  const db=checked(manifest.database,'file').path;assert(within(db,allocation));
  assert.equal(db,checked(path.join(manifest.common,'devmap/devmap.db'),'file').path);
  assert(within(checked(manifest.change_probe.path,'file').path,allocation));
  assert.equal(manifest.worktrees.length,manifest.dimensions.worktrees);
  for(const key of ['root','git_dir','worktree_id'])assert.equal(new Set(manifest.worktrees.map(w=>w[key])).size,manifest.worktrees.length);
  for(const row of manifest.worktrees){
    for(const [p,expected] of [[row.root,row.root_physical],[row.git_dir,row.admin_physical]]){
      const actual=execFileSync(python,['-c',PHYSICAL,p],{encoding:'utf8',timeout:5000,windowsHide:true}).trim();assert.equal(actual,expected,'Manifest physical directory replaced');
    }
  }
  assert(manifest.worktrees.some(w=>w.worktree_id===manifest.current_worktree_id&&checked(w.root,'directory').path===checked(manifest.source,'directory').path));
}
function hello(id,build){return new Promise((resolve,reject)=>{
  const client_instance=crypto.randomBytes(16).toString('hex'),socket=net.createConnection(`\\\\.\\pipe\\devmap-${id.repository}`);let bytes=Buffer.alloc(0),done=false;
  const timer=setTimeout(()=>finish(new Error('Hello timeout')),3000);
  function finish(error,result){if(done)return;done=true;clearTimeout(timer);socket.destroy();error?reject(error):resolve(result);}
  socket.on('error',finish);socket.on('close',()=>{if(!done)finish(new Error('Hello closed'));});
  socket.on('connect',()=>{const body=Buffer.from(JSON.stringify({protocol:1,repository:id.repository,build,source:id.source,git_dir:id.git_dir,client_instance})),size=Buffer.alloc(4);size.writeUInt32BE(body.length);socket.write(Buffer.concat([size,body]));});
  socket.on('data',chunk=>{bytes=Buffer.concat([bytes,chunk]);if(bytes.length>16388)return finish(new Error('Hello overflow'));if(bytes.length<4)return;const size=bytes.readUInt32BE();if(size>16384)return finish(new Error('Hello frame overflow'));if(bytes.length<size+4)return;
    try{const response=JSON.parse(bytes.subarray(4,size+4));assert.equal(response.status,'Accepted');const w=response.welcome;assert.equal(w.repository,id.repository);assert.equal(w.build,build);assert.equal(w.client_instance,client_instance);finish(null,w);}catch(e){finish(e);}});
});}
async function publicSummary(manifest,ctx){
  const exe=ctx.checked.candidate.path,build=ctx.config.candidate_sha256;
  assert.equal(sha(fs.readFileSync(exe)),build,'Candidate changed before runtime identity invocation');
  const id=JSON.parse(execFileSync(exe,['runtime','--identity','--source',manifest.source],{encoding:'utf8',timeout:15000,windowsHide:true,env:ctx.env}));
  assert.equal(checked(id.common,'directory').path,checked(manifest.common,'directory').path);
  assert.equal(checked(id.source,'directory').path,checked(manifest.source,'directory').path);
  try{await hello(id,build);throw new Error('Existing unowned endpoint refused');}catch(e){if(e.code!=='ENOENT')throw e;}
  const nonce=crypto.randomBytes(16).toString('hex'),owner=owned(exe,['runtime','--owner','--source',manifest.source,'--instance',nonce,'--idle-seconds','60'],'summary-owner',ctx,['ignore','ignore','pipe']);
  const deadline=performance.now()+15000;
  for(;;){assert(!owner.error&&!owner.exit,'Owned owner failed');try{const w=await hello(id,build);assert.equal(w.owner_pid,owner.child.pid);assert.equal(w.owner_instance,nonce);break;}catch(e){if(e.code!=='ENOENT')throw e;assert(performance.now()<deadline);await delay(50);}}
  const proxy=owned(exe,['mcp','--source',manifest.source],'summary-proxy',ctx,['pipe','pipe','pipe']);
  let next=0,buffer=Buffer.alloc(0);const pending=new Map();
  const fail=e=>{for(const p of pending.values()){clearTimeout(p.timer);p.reject(e);}pending.clear();};
  proxy.child.on('error',fail);proxy.child.on('close',()=>fail(new Error('Proxy exited')));proxy.child.stdin.on('error',fail);proxy.child.stdout.on('error',fail);proxy.child.stdout.on('end',()=>fail(new Error('Proxy EOF')));
  proxy.child.stdout.on('data',chunk=>{try{buffer=Buffer.concat([buffer,chunk]);assert(buffer.length<=4*1024*1024);let i;while((i=buffer.indexOf(10))>=0){const line=buffer.subarray(0,i);buffer=buffer.subarray(i+1);if(!line.length)continue;const result=JSON.parse(line),p=pending.get(result.id);assert(p);pending.delete(result.id);clearTimeout(p.timer);p.resolve(result);}}catch(e){fail(e);ctx.error=e;}});
  const request=(method,params)=>new Promise((resolve,reject)=>{const id=++next,timer=setTimeout(()=>{pending.delete(id);reject(new Error('MCP deadline'));},45000);pending.set(id,{resolve,reject,timer});proxy.child.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n',e=>{if(e)fail(e);});});
  const init=await request('initialize',{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'schema2-fixture-finalizer',version:'1'}});assert(!init.error&&init.result);
  const started=performance.now(),tokens=new Set();
  const fetch=async cursor=>{assert(performance.now()-started<55000,'Finalization cursor audit timeout; no restart');
    if(cursor){assert(!tokens.has(cursor));tokens.add(cursor);assert(tokens.size<=4096);}
    const response=await request('tools/call',{name:'devmap_read_map',arguments:{view:'summary',...(cursor?{cursor}:{})}});
    ctx.append('summary-responses.ndjson',JSON.stringify(response)+'\n');assert(!response.error&&response.result?.isError===false);assert(Buffer.byteLength(JSON.stringify(response.result))<=32768);
    const m=response.result.structuredContent;assert.equal(m.schema_version,'devmap-summary/1');assert.equal(m.repository_id,manifest.repository_id);assert.equal(m.current_worktree_id,manifest.current_worktree_id);assert.equal(m.counts_scope,'captured_bounded_model');assert.equal(m.source_truncated,false);return m;};
  const initial=await fetch(),same=envelope(initial),collections={};
  for(const name of ['workspaces','tasks','warnings']){let page=initial.pages[name],items=[];const total=page.total;
    for(;;){assert.equal(page.total,total);assert.equal(page.included,page.items.length);assert.equal(page.incomplete,Boolean(page.next_cursor));items.push(...page.items);if(!page.next_cursor)break;const m=await fetch(page.next_cursor);assert.deepEqual(envelope(m),same);page=m.pages[name];}
    assert.equal(items.length,total);collections[name]=items;}
  const actual=collections.workspaces.map(w=>w.worktree_id);assert.equal(new Set(actual).size,actual.length);assert.deepEqual(actual.sort(),manifest.worktrees.map(w=>w.worktree_id).sort());
  let unicode=false;
  for(const item of [...collections.workspaces,...collections.tasks]){let cursor=item.detail_cursor,offset=0,total=null,digest=null,chunks=0;const bytes=[];
    while(cursor){const m=await fetch(cursor);assert.deepEqual(envelope(m),same);const d=m.pages.details;assert.equal(d.encoding,'json-utf8');assert.equal(d.offset_bytes,offset);total??=d.total_bytes;digest??=d.sha256;assert.equal(d.total_bytes,total);assert.equal(d.sha256,digest);const b=Buffer.from(d.content);bytes.push(b);offset+=b.length;chunks++;cursor=d.next_cursor;assert.equal(d.incomplete,Boolean(cursor));}
    const payload=Buffer.concat(bytes);assert.equal(payload.length,total);assert.equal(sha(payload),digest);JSON.parse(payload);unicode ||=chunks>1&&/[^\x00-\x7f]/.test(payload.toString());}
  assert(unicode,'Public summary did not expose required multi-chunk Unicode fixture detail');
  const welcome=await hello(id,build);assert.equal(welcome.owner_pid,owner.child.pid);assert.equal(welcome.owner_instance,nonce);
  await stop(proxy);await stop(owner);
  return {counts:initial.counts,totals:Object.fromEntries(Object.entries(collections).map(([k,v])=>[k,v.length])),observations:initial.observations,unicode_verified:unicode};
}
async function generate(config){
  const checkedInputs=preflight(config),allocation=createNew(checkedInputs.parent.path,'schema2-'),run=createNew(checkedInputs.parent.path,'schema2-generation-');
  jsonNew(path.join(run.native,'creation.json'),{nonce:run.nonce,identity:run.identity,allocation_nonce:allocation.nonce,allocation:allocation.identity});
  jsonNew(path.join(run.native,'preflight.json'),checkedInputs);
  console.log(JSON.stringify({allocation:allocation.native,run:run.native,stage:'exclusive directories created; retain even if later setup fails'}));
  const physical=execFileSync(checkedInputs.python.path,['-c',PHYSICAL,allocation.native],{encoding:'utf8',timeout:5000,windowsHide:true}).trim();
  const allocationReceipt={schema:'devmap/schema2-allocation/1',nonce:allocation.nonce,exclusive_creation:true,native_root:allocation.native,allocation_root:allocation.identity.path,
    allocation_identity:{dev:allocation.identity.dev,ino:allocation.identity.ino},physical_identity:physical,mode:config.mode,dimensions:checkedInputs.dims,evaluation_time:config.evaluation_time,
    candidate_sha256:config.candidate_sha256,test_executable_sha256:config.test_executable_sha256,created_at:new Date().toISOString()};
  const receiptPath=path.join(allocation.native,'allocation.json');jsonNew(receiptPath,allocationReceipt);const seal=sha(fs.readFileSync(receiptPath));
  const ctx={config,checked:checkedInputs,run:run.native,children:[],error:null,bytes:0,env:environment(allocation.native,checkedInputs.trusted_temp.path)},fds=new Map();
  ctx.verify=()=>{assert.deepEqual(checked(allocation.native,'directory'),allocation.identity);assert.deepEqual(checked(run.native,'directory'),run.identity);assert.deepEqual(checked(config.trusted_temp,'directory'),checkedInputs.trusted_temp);assert.equal(sha(fs.readFileSync(receiptPath)),seal);assert(available(allocation.native)>=RESERVE+LOG_RESERVE);};
  ctx.append=(name,b)=>{ctx.verify();ctx.bytes+=Buffer.byteLength(b);assert(ctx.bytes<=64*1024*1024,'Generator log budget');if(!fds.has(name))fds.set(name,fs.openSync(path.join(run.native,name),'wx'));fs.writeSync(fds.get(name),b);};
  const report={schema:'devmap/schema2-generation-run/1',allocation:allocation.identity,run:run.identity,config,preflight:checkedInputs,stages:[],completed:false};
  const save=()=>{const p=path.join(run.native,'report.next');fs.writeFileSync(p,JSON.stringify(report,null,2),{flag:'wx'});fs.renameSync(p,path.join(run.native,'report.json'));};
  const stage=name=>{report.stages.push({name,at:new Date().toISOString(),available_bytes:String(available(allocation.native))});save();};
  stage('allocation_created');
  console.log(JSON.stringify({allocation:allocation.native,run:run.native,stage:'allocated; no automatic cleanup'}));
  try {
    ctx.env.DEVMAP_SCHEMA2_ALLOCATION_RECEIPT=receiptPath;ctx.env.DEVMAP_SCHEMA2_ALLOCATION_SHA256=seal;
    const entry=owned(checkedInputs.test.path,['--ignored','--exact','generate_owned_schema2_scale_corpus','--nocapture','--test-threads=1'],'rust-generator',ctx);
    entry.child.stdout.on('data',b=>{try{ctx.append('rust-generator.stdout',b);}catch(e){ctx.error=e;}});entry.child.stdout.on('error',e=>{ctx.error=e;});
    const deadline=performance.now()+30*60*1000;
    while(!entry.exit&&!entry.error){if(ctx.error)throw ctx.error;assert(performance.now()<deadline,'Generator deadline; retain partial allocation');ctx.verify();await delay(1000);}
    await entry.closed;await stop(entry);assert(!entry.error&&entry.exit.code===0,`Rust generator failed: ${entry.error||JSON.stringify(entry.exit)}`);stage('rust_active_verified');
    const manifest=JSON.parse(fs.readFileSync(path.join(allocation.native,'manifest.json'),'utf8'));assert.equal(manifest.schema,'devmap-synthetic-scale/2');assert.equal(manifest.state,'active_verified');assert.equal(manifest.allocation_nonce,allocation.nonce);
    validateManifest(manifest,allocation.identity.path,checkedInputs.python.path);
    const immutableRoots=manifest.immutable_roots.map(p=>checked(p,'directory').path);
    const baseline=sqlState(checkedInputs.python.path,manifest.database),immutable=inventory(immutableRoots);
    for(const p of baseline.activation_snapshot_paths){const actual=checked(p,'directory').path;assert(within(actual,allocation.identity.path));assert(immutableRoots.some(root=>actual===root||within(actual,root)),'Activation backup not covered by immutable roots');}
    assert.equal(baseline.meta[0],2);assert.equal(baseline.tables.journal_records.rows,manifest.dimensions.events);assert.equal(baseline.tables.journal_sessions.rows,manifest.dimensions.sessions);
    for(const relative of ['profile','profile/xdg','state'])fs.mkdirSync(path.join(allocation.native,relative),{recursive:true});
    const summary=await publicSummary(manifest,ctx);stage('public_summary_verified');
    assert.deepEqual(sqlState(checkedInputs.python.path,manifest.database),baseline);assert.deepEqual(inventory(immutableRoots),immutable);
    const roots=[manifest.source,manifest.common,...manifest.worktrees.flatMap(w=>[w.root,w.git_dir])];const own=Array.from(new Set(roots.map(p=>checked(p,'directory').path))).map(p=>{const d=checked(p,'directory');assert(within(d.path,allocation.identity.path));return {path:d.path,identity:{dev:d.dev,ino:d.ino}};});
    const worktrees=manifest.worktrees.map(w=>({root:checked(w.root,'directory').path,git_dir:checked(w.git_dir,'directory').path,worktree_id:w.worktree_id}));
    for(const key of ['root','git_dir','worktree_id'])assert.equal(new Set(worktrees.map(w=>w[key])).size,worktrees.length);
    const receipt={schema:'devmap/benchmark-fixture/1',nonce:allocation.nonce,exclusive_creation:true,allocation_root:allocation.identity.path,allocation_identity:{dev:allocation.identity.dev,ino:allocation.identity.ino},schema_version:2,
      source:checked(manifest.source,'directory').path,common:checked(manifest.common,'directory').path,repository_id:manifest.repository_id,current_worktree_id:manifest.current_worktree_id,worktrees,owned_directories:own,
      dimensions:manifest.dimensions,immutable_roots:immutableRoots,baseline_sql:baseline,baseline_immutable:{entries:immutable.entries,sha256:immutable.sha256},
      expected_summary:{counts:summary.counts,totals:summary.totals},change_probe:{...manifest.change_probe,path:checked(manifest.change_probe.path,'file').path},generator:{candidate_sha256:config.candidate_sha256,test_executable_sha256:config.test_executable_sha256,orchestrator_sha256:sha(fs.readFileSync(__filename)),manifest_sha256:sha(fs.readFileSync(path.join(allocation.native,'manifest.json'))),evaluation_time:config.evaluation_time,summary_observations:summary.observations}};
    ctx.verify();assert.equal(receipt.dimensions.worktrees,worktrees.length);assert.equal(receipt.expected_summary.totals.workspaces,worktrees.length);
    report.pending_receipt=receipt;
  }catch(error){report.error=String(error.stack||error);stage('failed');}
  finally {
    const cleanup=await Promise.allSettled(ctx.children.map(stop));report.cleanup_errors=cleanup.filter(r=>r.status==='rejected').map(r=>String(r.reason));report.children=ctx.children.map(e=>({label:e.label,pid:e.child.pid,exit:e.exit,job_report:e.jobReport,job:e.job,error:String(e.error||'')}));
    if(ctx.error)report.async_error=String(ctx.error);
    for(const fd of fds.values())fs.closeSync(fd);
    if(!report.error&&!report.async_error&&!report.cleanup_errors.length&&report.pending_receipt){ctx.verify();jsonNew(path.join(allocation.native,'benchmark-receipt.json'),report.pending_receipt);delete report.pending_receipt;report.completed=true;stage('receipt_ready');}
    else save();
  }
  assert(report.completed,`Fixture failed and retained: ${run.native}`);console.log(JSON.stringify({receipt:path.join(allocation.native,'benchmark-receipt.json'),performance_acceptance:false}));
}
if(require.main===module){
  const [mode,file]=process.argv.slice(2);
  if(mode==='--preflight'&&file){try{console.log(JSON.stringify(preflight(JSON.parse(fs.readFileSync(file))),null,2));}catch(e){console.error(e);process.exitCode=1;}}
  else if(mode==='--generate'&&file)generate(JSON.parse(fs.readFileSync(file))).catch(e=>{console.error(e);process.exitCode=1;});
  else console.log('Explicit only: --preflight config.json (read-only) | --generate config.json (new owned fixture; requires root authorization)');
}
module.exports={dimensions};
