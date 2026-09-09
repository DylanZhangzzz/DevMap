'use strict';
// Owned schema-2 fixture only. No fixture creation, recursive deletion, or PID kill.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const net = require('node:net');
const crypto = require('node:crypto');
const {spawn, execFileSync} = require('node:child_process');
const {setTimeout: delay} = require('node:timers/promises');
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const LIMIT = 32768, RUN_LIMIT = 128 * 1024 * 1024;
const within = (child, parent) => {
  const relative = path.relative(parent, child);
  return relative !== '' && relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
};
const identity = stat => ({dev:String(stat.dev), ino:String(stat.ino)});
function checked(p, kind) {
  assert(path.isAbsolute(p), 'Absolute path required');
  if(process.platform==='win32') {
    if(p.startsWith('\\\\?\\')) {
      const local=p.slice(4);
      assert(/^[A-Za-z]:\\/.test(local),'Unsupported extended path namespace');
      // Strip only the local drive namespace, before checking every ancestor.
      // Reject spellings whose Win32 interpretation would change on stripping.
      const parts=local.slice(3).split('\\');
      assert(parts.every((part,i)=>(part.length>0||i===parts.length-1)&&
        part!=='.'&&part!=='..'&&!/[. ]$/.test(part)&&!/[\x00-\x1f<>:"/|?*]/.test(part)&&
        !/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(part)), 'Ambiguous extended path');
      p=local;
    }
    assert(/^[A-Za-z]:[\\/]/.test(p),'Unsupported non-local drive path');
  }
  for (let cursor = p;; cursor = path.dirname(cursor)) {
    const st = fs.lstatSync(cursor, {bigint:true});
    assert(!st.isSymbolicLink(), `Link/reparse traversal refused: ${cursor}`);
    if (path.dirname(cursor) === cursor) break;
  }
  const st = fs.statSync(p, {bigint:true});
  assert(kind === 'file' ? st.isFile() : st.isDirectory(), `Wrong path kind: ${p}`);
  return {path:fs.realpathSync.native(p), ...identity(st)};
}
function nearestRank(rows) {
  const values = rows.filter(r => !r.error).map(r => r.elapsed_ms).sort((a,b)=>a-b);
  return {total:rows.length, failures:rows.filter(r=>r.error).length, successful:values.length,
    p50_ms:values[Math.max(0, Math.ceil(values.length*.5)-1)] ?? null,
    p95_ms:values[Math.max(0, Math.ceil(values.length*.95)-1)] ?? null,
    max_ms:values.at(-1) ?? null};
}
function policy(mode) {
  assert(['smoke','acceptance'].includes(mode));
  return mode === 'acceptance'
    ? {cold:20, clients:4, warmups:10, initial_rounds:100, max_rounds:300, hot_per_client:100, hot_p95_ms:200, cold_p95_ms:3000}
    : {cold:2, clients:4, warmups:1, initial_rounds:3, max_rounds:10, hot_per_client:2, hot_p95_ms:200, cold_p95_ms:3000};
}
const SQL = String.raw`
import sqlite3,json,sys,pathlib,hashlib,base64
p=pathlib.Path(sys.argv[1]).resolve()
c=sqlite3.connect(p.as_uri()+"?mode=ro",uri=True,timeout=10)
c.execute("PRAGMA query_only=ON"); c.execute("BEGIN")
meta=c.execute("select schema_version,repository_id,generation,backend_state from store_meta").fetchone()
assert meta[0]==2 and meta[3]=='active', 'not active schema 2'
names=[r[0] for r in c.execute("select name from sqlite_schema where type='table' and name not like 'sqlite_%' order by name")]
assert len(names)==14, 'exactly 14 user tables required'
def q(s): return '"'+s.replace('"','""')+'"'
def val(v):
 if isinstance(v,bytes): return {'blob':base64.b64encode(v).decode('ascii')}
 if isinstance(v,int): return {'int':str(v)}
 if isinstance(v,float): return {'float':v.hex()}
 return v
tables={}
for name in names:
 cols=[r[1] for r in c.execute('pragma table_info('+q(name)+')')]
 h=hashlib.sha256(); count=0
 for row in c.execute('select * from '+q(name)+' order by '+','.join(q(k) for k in cols)):
  h.update(json.dumps([val(v) for v in row],ensure_ascii=True,separators=(',',':')).encode()+b'\n'); count+=1
 tables[name]={'rows':count,'sha256':h.hexdigest()}
schema=list(c.execute("select type,name,tbl_name,sql from sqlite_schema order by type,name"))
backups=[json.loads(r[0])['snapshot_path'] for r in c.execute("select record_json from migration_sources where source_path='@activation'")]
print(json.dumps({'meta':meta,'tables':tables,'activation_snapshot_paths':backups,'schema_sha256':hashlib.sha256(json.dumps(schema,separators=(',',':')).encode()).hexdigest()},sort_keys=True))
`;
function sqlState(python, db) {
  const canonical = checked(db, 'file').path;
  return JSON.parse(execFileSync(python, ['-c', SQL, canonical], {encoding:'utf8',timeout:60000,maxBuffer:1024*1024,windowsHide:true}));
}
function inventory(roots) {
  const output = []; let count = 0;
  for (const root of roots) {
    checked(root, 'directory');
    const visit = dir => {
      for (const name of fs.readdirSync(dir).sort()) {
        assert(++count <= 200000, 'Immutable inventory entry bound');
        const p = path.join(dir,name), st = fs.lstatSync(p, {bigint:true});
        assert(!st.isSymbolicLink(), 'Immutable inventory contains a link');
        if (st.isDirectory()) { output.push({root,relative:path.relative(root,p),kind:'directory'}); visit(p); }
        else {
          assert(st.isFile()); const digest=crypto.createHash('sha256');
          const fd=fs.openSync(p,'r'), buffer=Buffer.alloc(65536);
          try { let n; while((n=fs.readSync(fd,buffer,0,buffer.length,null))>0) digest.update(buffer.subarray(0,n)); }
          finally {fs.closeSync(fd);}
          output.push({root,relative:path.relative(root,p),kind:'file',size:String(st.size),sha256:digest.digest('hex')});
        }
      }
    }; visit(root);
  }
  return {entries:output.length, sha256:hash(JSON.stringify(output)), inventory:output};
}
function hello(id, build) {
  return new Promise((resolve,reject) => {
    const client_instance=crypto.randomBytes(16).toString('hex');
    const socket=net.createConnection(`\\\\.\\pipe\\devmap-${id.repository}`);
    let bytes=Buffer.alloc(0), settled=false;
    const timer=setTimeout(()=>finish(new Error('Hello deadline')),3000);
    function finish(error,value) { if(settled)return; settled=true;clearTimeout(timer);socket.destroy();error?reject(error):resolve(value); }
    socket.on('error',finish); socket.on('close',()=>{if(!settled)finish(new Error('Hello closed'));});
    socket.on('connect',()=>{
      const body=Buffer.from(JSON.stringify({protocol:1,repository:id.repository,build,source:id.source,git_dir:id.git_dir,client_instance}));
      const frame=Buffer.alloc(4);frame.writeUInt32BE(body.length);socket.write(Buffer.concat([frame,body]));
    });
    socket.on('data',chunk=>{
      bytes=Buffer.concat([bytes,chunk]);
      if(bytes.length>16388)return finish(new Error('Hello overflow'));
      if(bytes.length<4)return; const length=bytes.readUInt32BE();
      if(length>16384)return finish(new Error('Hello frame overflow'));
      if(bytes.length<length+4)return;
      try {const result=JSON.parse(bytes.subarray(4,4+length));assert.equal(result.status,'Accepted');
        assert.equal(result.welcome.repository,id.repository);assert.equal(result.welcome.build,build);
        assert.equal(result.welcome.client_instance,client_instance);finish(null,result.welcome);
      } catch(error){finish(error);}
    });
  });
}
async function stop(entry) {
  // Ownership is the retained ChildProcess, never welcome.owner_pid.
  entry.stopping=true;
  if(entry.child.exitCode===null && entry.child.signalCode===null && !entry.spawn_error)entry.child.kill();
  let timer;
  try {await Promise.race([entry.closed,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error(`Owned child cleanup timeout: ${entry.label}`)),5000);})]);}
  finally {clearTimeout(timer);}
}
function spawnOwned(exe,args,label,ctx,stdio) {
  ctx.verifyRun();
  ctx.verifyFixture();
  const spawn_started_ms=performance.now();
  const child=spawn(exe,args,{windowsHide:true,stdio});
  const entry={child,label,spawn_started_ms,spawn_error:null,exit:null};
  ctx.children.push(entry); // Retain before any asynchronous readiness operation.
  entry.closed=new Promise(resolve=>{
    child.once('error',error=>{entry.spawn_error=String(error);ctx.asyncError??=error;resolve();});
    child.once('close',(code,signal)=>{entry.exit={code,signal};if(!entry.stopping)ctx.asyncError??=new Error(`Unexpected owned child exit: ${label}/${code}/${signal}`);resolve();});
  });
  child.stderr.on('data',bytes=>{try{ctx.append(`${label}.stderr`,bytes);}catch(error){ctx.asyncError=error;}});
  child.stderr.on('error',error=>{ctx.asyncError??=error;});
  return entry;
}
function proxy(exe,source,label,ctx) {
  const entry=spawnOwned(exe,['mcp','--source',source],label,ctx,['pipe','pipe','pipe']);
  let next=0, buffer=Buffer.alloc(0); const pending=new Map();
  const fail=error=>{for(const item of pending.values()){clearTimeout(item.timer);item.reject(error);}pending.clear();};
  entry.child.on('error',fail);entry.child.on('close',()=>fail(new Error(`Proxy closed: ${label}`)));
  entry.child.stdin.on('error',fail);
  entry.child.stdout.on('error',error=>{fail(error);ctx.asyncError??=error;});
  entry.child.stdout.on('end',()=>fail(new Error(`Proxy stdout ended: ${label}`)));
  entry.child.stdout.on('data',chunk=>{
    try {
      buffer=Buffer.concat([buffer,chunk]);assert(buffer.length<=4*1024*1024,'MCP framing bound');
      let end;while((end=buffer.indexOf(10))>=0){
        const line=buffer.subarray(0,end);buffer=buffer.subarray(end+1);
        if(!line.length)continue;
        const response=JSON.parse(line), item=pending.get(response.id);
        assert(item, 'Unexpected MCP response ID');pending.delete(response.id);clearTimeout(item.timer);
        item.resolve({response,wire_bytes:line.length+1});
      }
    }catch(error){fail(error);ctx.asyncError=error;}
  });
  const request=(method,params,timeoutMs=45000)=>new Promise((resolve,reject)=>{
    if(entry.exit || entry.spawn_error)return reject(new Error(`Proxy unavailable: ${label}`));
    const id=++next,timer=setTimeout(()=>{pending.delete(id);reject(new Error(`MCP deadline: ${label}/${method}`));},timeoutMs);
    pending.set(id,{resolve,reject,timer});
    entry.child.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n',error=>{if(error)fail(error);});
  });
  return {entry,request,previousCycle:null};
}
async function initialize(client) {
  const {response}=await client.request('initialize',{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'owned-summary-benchmark',version:'1'}});
  assert(!response.error && response.result, 'MCP initialization failed');
}
function modelFrom(packet,row) {
  row.wire_bytes=packet.wire_bytes;
  row.result_bytes=packet.response.result===undefined?null:Buffer.byteLength(JSON.stringify(packet.response.result));
  row.result_sha256=packet.response.result===undefined?null:hash(JSON.stringify(packet.response.result));
  assert(row.result_bytes===null || row.result_bytes<=LIMIT, 'Complete MCP result exceeds 32768 bytes');
  assert(!packet.response.error && packet.response.result?.isError!==true, JSON.stringify(packet.response));
  const model=packet.response.result.structuredContent;
  assert.equal(model?.schema_version,'devmap-summary/1');return model;
}
async function sample(client,phase,index,ctx,cursor=null) {
  const row={phase,index,client:client.entry.label,started_at:new Date().toISOString()};
  const started=performance.now();
  try {
    const timeout=Math.min(45000,ctx.phaseDeadline===undefined?45000:Math.floor(ctx.phaseDeadline-performance.now()));
    assert(timeout>0,'Predeclared phase deadline exceeded');
    const packet=await client.request('tools/call',{name:'devmap_read_map',arguments:{view:'summary',...(cursor?{cursor}:{})}},timeout);
    row.received_monotonic_ms=performance.now();row.elapsed_ms=row.received_monotonic_ms-started;
    ctx.append('responses.ndjson',JSON.stringify({phase,index,client:row.client,...packet})+'\n');
    const model=modelFrom(packet,row);
    row.generated_at=model.generated_at;row.observations=model.observations;row.snapshot_id=model.snapshot_id;
    assert.equal(model.repository_id,ctx.receipt.repository_id);
    assert.equal(model.current_worktree_id,ctx.receipt.current_worktree_id);
    assert.equal(model.counts_scope,'captured_bounded_model');
    assert.equal(model.source_truncated,false);
    assert.deepEqual(model.counts,ctx.receipt.expected_summary.counts);
    if(!cursor){
      for(const [name,total] of Object.entries(ctx.receipt.expected_summary.totals))assert.equal(model.pages[name]?.total,total);
      const cycle=model.observations.git_cycle;assert(Number.isSafeInteger(cycle));
      assert(client.previousCycle===null || cycle>=client.previousCycle, 'git_cycle decreased');
      row.freshness_group=cycle===client.previousCycle?'no-full-collection':'refresh-affected-or-first';
      client.previousCycle=cycle;
    }
    return {row,model};
  } catch(error){row.elapsed_ms??=performance.now()-started;row.error=String(error.stack||error);return {row,error};}
  finally {ctx.rows.push(row);ctx.append('samples.ndjson',JSON.stringify(row)+'\n');}
}
async function required(client,phase,index,ctx,cursor) {
  const result=await sample(client,phase,index,ctx,cursor);if(result.error)throw result.error;return result;
}
function envelope(model) {const {pages,...rest}=model;return rest;}
function uniqueWorktrees(rows, expectedCount) {
  assert.equal(rows.length,expectedCount,'Actual worktree count differs from declared dimensions');
  for(const key of ['worktree_id','root_identity','admin_identity']) {
    assert(rows.every(row=>typeof row[key]==='string'&&row[key].length>0),`Missing ${key}`);
    assert.equal(new Set(rows.map(row=>row[key])).size,rows.length,`Duplicate ${key}`);
  }
}
function workspaceSet(items, worktrees) {
  const actual=items.map(item=>item.worktree_id);
  assert.equal(new Set(actual).size,actual.length,'Repeated workspace across cursor pages');
  assert.deepEqual(actual.sort(),worktrees.map(row=>row.worktree_id).sort(),'Actual public workspaces differ from fixture receipt');
}
function singleLink(stat) { assert.equal(BigInt(stat.nlink),1n,'Mutable probe must have exactly one hard link'); }
function openProbe(p, expected) {
  singleLink(fs.lstatSync(p,{bigint:true}));assert.deepEqual(checked(p,'file'),expected);
  const fd=fs.openSync(p,'r+');
  try {const st=fs.fstatSync(fd,{bigint:true});singleLink(st);assert.deepEqual(identity(st),{dev:expected.dev,ino:expected.ino});return fd;}
  catch(error){fs.closeSync(fd);throw error;}
}
async function audit(client,ctx,label,interleave) {
  const {model}=await required(client,label,0,ctx), started=performance.now();
  if(interleave)await interleave();
  const expected=envelope(model), all={}, tokens=new Set();
  const page=async cursor=>{
    assert(performance.now()-started<55000,'Cursor audit exceeded 55 seconds; not silently restarted');
    assert(!tokens.has(cursor),'Cursor cycle');tokens.add(cursor);assert(tokens.size<=4096);
    const result=await required(client,label,tokens.size,ctx,cursor);assert.deepEqual(envelope(result.model),expected);return result.model.pages;
  };
  for(const name of ['workspaces','tasks','warnings']) {
    let part=model.pages[name];const items=[];
    for(;;){assert.equal(part.total,ctx.receipt.expected_summary.totals[name]);assert.equal(part.included,part.items.length);
      items.push(...part.items);assert.equal(part.incomplete,Boolean(part.next_cursor));
      if(!part.next_cursor)break;part=(await page(part.next_cursor))[name];}
    assert.equal(items.length,part.total);all[name]=items;
  }
  workspaceSet(all.workspaces,ctx.receipt.worktrees);
  const taskKeys=all.tasks.map(item=>JSON.stringify([item.worktree_id,item.id]));
  assert.equal(new Set(taskKeys).size,taskKeys.length,'Repeated task across cursor pages');
  let unicode=false;
  for(const item of [...all.workspaces,...all.tasks]) {
    let cursor=item.detail_cursor, offset=0, digest=null, total=null, chunks=0;const content=[];
    while(cursor){const detail=(await page(cursor)).details;assert.equal(detail.encoding,'json-utf8');assert.equal(detail.offset_bytes,offset);
      digest??=detail.sha256;total??=detail.total_bytes;assert.equal(detail.sha256,digest);assert.equal(detail.total_bytes,total);
      const bytes=Buffer.from(detail.content);offset+=bytes.length;content.push(bytes);chunks++;cursor=detail.next_cursor;assert.equal(detail.incomplete,Boolean(cursor));}
    const bytes=Buffer.concat(content);assert.equal(bytes.length,total);assert.equal(hash(bytes),digest);JSON.parse(bytes.toString('utf8'));
    if(chunks>1 && /[^\x00-\x7f]/.test(bytes.toString('utf8')))unicode=true;
  }
  if(ctx.config.mode==='acceptance')assert(unicode,'Acceptance fixture must exercise a multi-chunk Unicode detail');
  ctx.report.audits.push({label,snapshot_id:model.snapshot_id,observations:model.observations,totals:Object.fromEntries(Object.entries(all).map(([k,v])=>[k,v.length])),multichunk_unicode:unicode});ctx.save();
  return {model,all};
}
async function startOwner(ctx,label) {
  try {await hello(ctx.runtimeIdentity,ctx.build);throw new Error('Unowned endpoint already occupied');}
  catch(error){if(error.code!=='ENOENT')throw error;}
  const nonce=crypto.randomBytes(16).toString('hex');
  const entry=spawnOwned(ctx.exe,['runtime','--owner','--source',ctx.receipt.source,'--instance',nonce,'--idle-seconds','60'],label,ctx,['ignore','ignore','pipe']);
  const started=entry.spawn_started_ms;
  const deadline=performance.now()+15000;
  for(;;){
    if(entry.exit || entry.spawn_error)throw new Error(`Owned runtime unavailable: ${label}`);
    try {const welcome=await hello(ctx.runtimeIdentity,ctx.build);assert.equal(welcome.owner_pid,entry.child.pid);assert.equal(welcome.owner_instance,nonce);
      ctx.report.owners.push({label,pid:entry.child.pid,instance:nonce,created_at:new Date().toISOString()});ctx.save();return {entry,nonce,started,ready_ms:performance.now()-started};}
    catch(error){if(error.code!=='ENOENT')throw error;assert(performance.now()<deadline,'Owner readiness deadline');await delay(50);}
  }
}
async function sameOwner(ctx,owner) {
  assert(!owner.entry.exit && !owner.entry.spawn_error,'Owned owner exited');
  const current=await hello(ctx.runtimeIdentity,ctx.build);assert.equal(current.owner_pid,owner.entry.child.pid);assert.equal(current.owner_instance,owner.nonce);
}
function validateReceipt(config) {
  const allocation=checked(config.fixture_root,'directory'), receiptPath=checked(config.receipt,'file');
  assert(within(receiptPath.path,allocation.path),'Receipt outside explicitly supplied fixture allocation');
  const receipt=JSON.parse(fs.readFileSync(receiptPath.path,'utf8'));
  assert.equal(receipt.schema,'devmap/benchmark-fixture/1');assert.match(receipt.nonce,/^[a-f0-9]{32}$/);
  assert.equal(receipt.exclusive_creation,true);assert.equal(receipt.allocation_root,allocation.path);
  assert.deepEqual(receipt.allocation_identity,{dev:allocation.dev,ino:allocation.ino});
  assert.equal(receipt.schema_version,2);assert(Array.isArray(receipt.worktrees)&&receipt.worktrees.length>0);
  uniqueWorktrees(receipt.worktrees.map(row=>{
    const root=checked(row.root,'directory'),admin=checked(row.git_dir,'directory');
    return {worktree_id:row.worktree_id,root_identity:`${root.dev}:${root.ino}`,admin_identity:`${admin.dev}:${admin.ino}`};
  }),receipt.dimensions.worktrees);
  assert.equal(receipt.expected_summary.totals.workspaces,receipt.dimensions.worktrees);
  for(const item of [...receipt.worktrees,{root:receipt.source,git_dir:receipt.common}]){
    for(const p of [item.root,item.git_dir])assert(within(checked(p,'directory').path,allocation.path),'Fixture path outside allocation');
  }
  for(const item of receipt.owned_directories){assert(within(item.path,allocation.path));const actual=checked(item.path,'directory');assert.deepEqual({dev:actual.dev,ino:actual.ino},item.identity);}
  for(const p of [receipt.source,receipt.common,...receipt.worktrees.flatMap(w=>[w.root,w.git_dir])])assert(receipt.owned_directories.some(d=>d.path===p),'Missing physical directory receipt');
  assert(Array.isArray(receipt.immutable_roots),'Frozen backup/provenance inventory required, even when empty for native corpus');
  for(const p of receipt.immutable_roots)assert(within(checked(p,'directory').path,allocation.path));
  assert.deepEqual(Object.keys(receipt.expected_summary.totals).sort(),['tasks','warnings','workspaces']);
  assert(receipt.baseline_sql && receipt.baseline_immutable,'Generator baseline digests required');
  return receipt;
}
async function main(configPath) {
  assert.equal(process.platform,'win32','This owned pipe harness currently supports Windows only');
  const config=JSON.parse(fs.readFileSync(configPath,'utf8')), protocol=policy(config.mode);
  const receipt=validateReceipt(config), parent=checked(config.run_parent,'directory');
  assert(!within(parent.path,receipt.allocation_root) && parent.path!==receipt.allocation_root,'Run artifacts must be outside source allocation');
  const input=checked(config.candidate,'file'), python=checked(config.python,'file').path;
  const space=fs.statfsSync(parent.path,{bigint:true});assert(space.bavail*space.bsize>=BigInt(256*1024*1024)+fs.statSync(input.path,{bigint:true}).size,'Need 256 MiB reserve plus candidate copy; no cleanup permitted');
  const run=fs.mkdtempSync(path.join(parent.path,'shared-summary-')), runIdentity=checked(run,'directory');
  const build=hash(fs.readFileSync(input.path)), exe=path.join(run,'devmap.exe');
  fs.copyFileSync(input.path,exe,fs.constants.COPYFILE_EXCL);
  const ctx={config,receipt,build,exe,children:[],rows:[],bytes:0,asyncError:null};
  ctx.verifyRun=()=>assert.deepEqual(checked(run,'directory'),runIdentity,'Owned output directory replaced');
  ctx.verifyFixture=()=>assert.deepEqual(validateReceipt(config),receipt,'Fixture receipt or physical identity changed');
  const logFiles=new Map();
  ctx.append=(name,bytes)=>{ctx.verifyRun();ctx.bytes+=Buffer.byteLength(bytes);assert(ctx.bytes<=RUN_LIMIT,'Run log byte bound');
    if(!logFiles.has(name))logFiles.set(name,fs.openSync(path.join(run,name),'wx'));
    fs.writeSync(logFiles.get(name),bytes);};
  ctx.report={schema:'devmap/shared-summary-benchmark/1',mode:config.mode,protocol,source:receipt.source,candidate_sha256:build,
    harness_sha256:hash(fs.readFileSync(__filename)),fixture_receipt_sha256:hash(fs.readFileSync(config.receipt)),configuration:config,
    disk_before:{available:String(space.bavail*space.bsize)},scope:'owner-process cold; filesystem cache uncontrolled; no model/host/UI acceptance',
    hot_evidence:'same-client unchanged git_cycle; no full collection, not no Git subprocesses',rows_file:'samples.ndjson',responses_file:'responses.ndjson',
    performance_acceptance_scope:'cold/hot summary latency only; overall acceptance remains pending',
    freshness_acceptance:false,freshness_acceptance_status:'pending separate 100-trial/four-client cohort; one change is correctness smoke only',
    owners:[],cold:[],rounds:[],audits:[],errors:[],completed:false,started_at:new Date().toISOString()};
  ctx.save=()=>{ctx.verifyRun();const temp=path.join(run,'report.next');fs.writeFileSync(temp,JSON.stringify(ctx.report,null,2),{flag:'wx'});fs.renameSync(temp,path.join(run,'report.json'));};
  fs.writeFileSync(path.join(run,'creation.json'),JSON.stringify({nonce:crypto.randomBytes(16).toString('hex'),run:runIdentity,candidate_sha256:build,created_at:new Date().toISOString()}),{flag:'wx'});
  fs.copyFileSync(__filename,path.join(run,'harness.cjs'),fs.constants.COPYFILE_EXCL);ctx.save();console.log(JSON.stringify({run}));
  let baseline, immutable;
  const db=path.join(receipt.common,'devmap/devmap.db');
  const sidecars=()=>Object.fromEntries(['-wal','-shm','-journal'].map(suffix=>{const p=db+suffix;return [suffix,fs.existsSync(p)?{bytes:fs.statSync(p).size}:null];}));
  try {
    baseline=sqlState(python,db);immutable=inventory(receipt.immutable_roots);
    ctx.report.sqlite_sidecars_before=sidecars();
    for(const snapshot of baseline.activation_snapshot_paths){const backup=checked(snapshot,'directory').path;
      assert(receipt.immutable_roots.some(p=>{const root=checked(p,'directory').path;return backup===root||within(backup,root);}),'Activation backup missing from immutable inventory');}
    assert.deepEqual(baseline,receipt.baseline_sql);assert.deepEqual({entries:immutable.entries,sha256:immutable.sha256},receipt.baseline_immutable);
    assert.equal(baseline.meta[1],receipt.repository_id);
    assert.equal(baseline.tables.journal_sessions.rows,receipt.dimensions.sessions);assert.equal(baseline.tables.journal_records.rows,receipt.dimensions.events);
    assert.equal(receipt.worktrees.length,receipt.dimensions.worktrees);
    if(config.mode==='acceptance')assert.deepEqual(receipt.dimensions,{worktrees:20,sessions:100,events:100000});
    ctx.append('immutable-before.json',JSON.stringify(immutable));
    ctx.runtimeIdentity=JSON.parse(execFileSync(exe,['runtime','--identity','--source',receipt.source],{encoding:'utf8',timeout:15000,windowsHide:true}));
    // Runtime endpoint identity hashes common-dir + user; model/store identity
    // hashes the repository path separately. Never conflate the two namespaces.
    assert.match(ctx.runtimeIdentity.repository,/^[a-f0-9]{64}$/);
    assert.equal(checked(ctx.runtimeIdentity.source,'directory').path,checked(receipt.source,'directory').path);
    assert.equal(checked(ctx.runtimeIdentity.common,'directory').path,checked(receipt.common,'directory').path);
    assert(receipt.worktrees.some(w=>checked(w.git_dir,'directory').path===checked(ctx.runtimeIdentity.git_dir,'directory').path));
    ctx.report.runtime_identity=ctx.runtimeIdentity;
    const overall=performance.now()+45*60*1000;
    const health=()=>{assert(performance.now()<overall,'Overall benchmark deadline');if(ctx.asyncError)throw ctx.asyncError;ctx.verifyRun();};
    for(let i=0;i<protocol.cold;i++){
      health();const owner=await startOwner(ctx,`cold-owner-${i}`), client=proxy(exe,receipt.source,`cold-proxy-${i}`,ctx);
      try{const init=performance.now();await initialize(client);const initialization_ms=performance.now()-init;
        const {row}=await required(client,'owner-cold',i,ctx);const elapsed_ms=row.received_monotonic_ms-owner.started;await sameOwner(ctx,owner);
        ctx.report.cold.push({sample:i,instance:owner.nonce,elapsed_ms,owner_ready_ms:owner.ready_ms,initialization_ms,request_ms:row.elapsed_ms,observations:row.observations});ctx.save();}
      finally{const results=await Promise.allSettled([stop(client.entry)]);await stop(owner.entry);for(const r of results)if(r.status==='rejected')throw r.reason;}
    }
    assert.deepEqual(sqlState(python,db),baseline);assert.deepEqual(inventory(receipt.immutable_roots),immutable);
    const owner=await startOwner(ctx,'warm-owner'), clients=[];
    for(let i=0;i<protocol.clients;i++){const client=proxy(exe,receipt.source,`warm-proxy-${i}`,ctx);clients.push(client);await initialize(client);}
    for(let round=0;round<protocol.warmups;round++){health();const results=await Promise.allSettled(clients.map(c=>required(c,'warmup',round,ctx)));for(const r of results)if(r.status==='rejected')throw r.reason;}
    await sameOwner(ctx,owner);
    for(let round=0;round<protocol.max_rounds;round++){
      health();const start=performance.now();const results=await Promise.allSettled(clients.map(c=>sample(c,round<protocol.initial_rounds?'warm-initial':'warm-extension',round,ctx)));
      ctx.report.rounds.push({round,elapsed_ms:performance.now()-start,completion_spread_ms:Math.max(...results.map(r=>r.value?.row.elapsed_ms||0))-Math.min(...results.map(r=>r.value?.row.elapsed_ms||0))});ctx.save();
      assert(results.every(r=>r.status==='fulfilled'&&!r.value.error),'Measured failure retained; no automatic replacement');
      if(round+1>=protocol.initial_rounds && clients.every(c=>ctx.rows.filter(r=>r.client===c.entry.label&&r.phase.startsWith('warm-')&&r.freshness_group==='no-full-collection'&&!r.error).length>=protocol.hot_per_client))break;
    }
    await sameOwner(ctx,owner);
    for(let i=0;i<clients.length;i++)await audit(clients[i],ctx,`cursor-audit-${clients[i].entry.label}`,
      ()=>required(clients[(i+1)%clients.length],'other-client-refresh-during-cursor',i,ctx));
    assert.deepEqual(sqlState(python,db),baseline);assert.deepEqual(inventory(receipt.immutable_roots),immutable);
    ctx.report.preservation_read_cohorts=true;
    await smallChange(clients[0],ctx,owner);
    await sameOwner(ctx,owner);
    const initial=ctx.rows.filter(r=>r.phase==='warm-initial'), all=ctx.rows.filter(r=>r.phase==='warm-initial'||r.phase==='warm-extension');
    const hot=all.filter(r=>r.freshness_group==='no-full-collection');
    ctx.report.statistics={cold:nearestRank(ctx.report.cold),initial:nearestRank(initial),all:nearestRank(all),hot:nearestRank(hot),
      per_client:clients.map(c=>({client:c.entry.label,all:nearestRank(all.filter(r=>r.client===c.entry.label)),hot:nearestRank(hot.filter(r=>r.client===c.entry.label))})),
      max_result_bytes:Math.max(...ctx.rows.map(r=>r.result_bytes||0))};
    assert.equal(initial.length,protocol.initial_rounds*4);
    assert(ctx.report.statistics.per_client.every(c=>c.hot.successful>=protocol.hot_per_client),'Predeclared hot extension cap exhausted');
    ctx.report.hot_gate_passed=ctx.report.statistics.per_client.every(c=>c.hot.p95_ms<=protocol.hot_p95_ms);
    ctx.report.cold_gate_passed=ctx.report.statistics.cold.successful===protocol.cold&&ctx.report.statistics.cold.p95_ms<=protocol.cold_p95_ms;
    if(config.mode==='acceptance')assert(ctx.report.hot_gate_passed&&ctx.report.cold_gate_passed,'Cold/hot p95 gate failed');
  } catch(error){ctx.report.errors.push(String(error.stack||error));}
  finally {
    const cleanup=await Promise.allSettled(ctx.children.map(stop));
    ctx.report.child_lifecycle=ctx.children.map(c=>({label:c.label,pid:c.child.pid,exit:c.exit,spawn_error:c.spawn_error}));
    ctx.report.cleanup_errors=cleanup.filter(r=>r.status==='rejected').map(r=>String(r.reason));
    const measured=ctx.rows.filter(r=>r.phase==='warm-initial'||r.phase==='warm-extension');
    ctx.report.population_outcome={initial:nearestRank(measured.filter(r=>r.phase==='warm-initial')),all:nearestRank(measured),
      hot:nearestRank(measured.filter(r=>r.freshness_group==='no-full-collection')),
      request_failures:ctx.rows.filter(r=>r.error).map(r=>({phase:r.phase,index:r.index,client:r.client,error:r.error}))};
    try{if(baseline){ctx.report.sql_after=sqlState(python,db);assert.deepEqual(ctx.report.sql_after,baseline);}if(immutable)assert.deepEqual(inventory(receipt.immutable_roots),immutable);}
    catch(error){ctx.report.errors.push(`Final preservation: ${error.stack||error}`);}
    ctx.report.sqlite_sidecars_after=sidecars();
    if(ctx.asyncError)ctx.report.errors.push(`Asynchronous child/log failure: ${ctx.asyncError.stack||ctx.asyncError}`);
    for(const fd of logFiles.values())fs.closeSync(fd);
    ctx.report.completed=ctx.report.errors.length===0&&ctx.report.cleanup_errors.length===0;
    ctx.report.summary_latency_acceptance=ctx.report.completed&&config.mode==='acceptance'&&ctx.report.hot_gate_passed&&ctx.report.cold_gate_passed;
    ctx.report.performance_acceptance=false; // 100-trial freshness/resource gates are separate and still pending.
    ctx.report.finished_at=new Date().toISOString();ctx.save();
  }
  assert(ctx.report.completed,`Benchmark failed; retained report: ${run}`);console.log(JSON.stringify({run,completed:true,summary_latency_acceptance:ctx.report.summary_latency_acceptance,performance_acceptance:false}));
}

async function smallChange(client,ctx,owner) {
  const probe=ctx.receipt.change_probe;assert(probe,'Owned tracked-file change probe receipt required');
  const checkedFile=checked(probe.path,'file');assert(within(checkedFile.path,ctx.receipt.allocation_root));
  singleLink(fs.lstatSync(probe.path,{bigint:true}));
  const original=fs.readFileSync(probe.path);assert(original.length<=65536);assert.equal(hash(original),probe.sha256);
  const worktree=ctx.receipt.worktrees.find(w=>w.worktree_id===probe.worktree_id);assert(worktree);
  assert(within(checkedFile.path,checked(worktree.root,'directory').path));
  const relative=path.relative(worktree.root,probe.path).split(path.sep).join('/');
  const tracked=execFileSync('git',['-C',worktree.root,'ls-files','--error-unmatch','-z','--',relative],{timeout:5000,maxBuffer:65536,windowsHide:true,env:{...process.env,GIT_TERMINAL_PROMPT:'0',GIT_NO_LAZY_FETCH:'1',GIT_NO_REPLACE_OBJECTS:'1'}});
  assert.equal(tracked.toString('utf8'),relative+'\0','Probe must be exactly one tracked path');
  const initial=await audit(client,ctx,'change-baseline'), lane=initial.all.workspaces.find(w=>w.worktree_id===probe.worktree_id);
  assert(lane?.git_status.status_observed && !lane.git_status.dirty && lane.git_status.changed_file_count===0,'Probe worktree must start clean');
  const rewrite=bytes=>{const fd=openProbe(probe.path,checkedFile);try{
    fs.writeSync(fd,bytes,0,bytes.length,0);fs.ftruncateSync(fd,bytes.length);fs.fsyncSync(fd);
  }finally{fs.closeSync(fd);}};
  async function observe(dirty,priorCycle) {
    const start=performance.now();let attempt=0;
    ctx.phaseDeadline=start+15000;
    try {
    while(performance.now()-start<15000){
      const data=await audit(client,ctx,dirty?'change-dirty':'change-restored');
      const row=data.all.workspaces.find(w=>w.worktree_id===probe.worktree_id);
      if(row?.git_status.status_observed && row.git_status.dirty===dirty && (dirty?row.git_status.changed_file_count>=1:row.git_status.changed_file_count===0) && data.model.observations.git_cycle>priorCycle)
        return {elapsed_ms:performance.now()-start,observations:data.model.observations,head:row.head};
      attempt++;assert(attempt<=30);await delay(250);
    }throw new Error('Small Git change detection exceeded 15 seconds');
    } finally {delete ctx.phaseDeadline;}
  }
  try {
    rewrite(Buffer.concat([original,Buffer.from('\n# devmap-owned-change-probe\n')]));
    const retained=await required(client,'retained-page-after-external-Git-change',0,ctx,lane.detail_cursor);
    assert.deepEqual(envelope(retained.model),envelope(initial.model),'External Git change must not rewrite retained observation timestamps');
    const detected=await observe(true,initial.model.observations.git_cycle);assert.equal(detected.head,lane.head);
    ctx.report.change_detection=detected;await sameOwner(ctx,owner);
  } finally {rewrite(original);assert.equal(hash(fs.readFileSync(probe.path)),probe.sha256);}
  ctx.report.change_restoration=await observe(false,ctx.report.change_detection.observations.git_cycle);ctx.save();
}
function selfTest() {
  assert(within(path.resolve('a/b'),path.resolve('a')));assert(!within(path.resolve('ab'),path.resolve('a')));
  assert.equal(policy('acceptance').initial_rounds*policy('acceptance').clients,400);assert.equal(policy('acceptance').max_rounds,300);
  assert.equal(nearestRank([{elapsed_ms:1},{elapsed_ms:4},{error:'x'}]).p95_ms,4);
  const row={};assert.throws(()=>modelFrom({wire_bytes:1,response:{result:{isError:true}}},row));assert(row.result_bytes>0);
  assert.throws(()=>modelFrom({wire_bytes:1,response:{result:{text:'x'.repeat(LIMIT)}}},{}));
  assert.deepEqual(envelope({snapshot_id:'s',pages:{a:1}}),{snapshot_id:'s'});
  const one={worktree_id:'a',root_identity:'r',admin_identity:'g'};
  assert.throws(()=>uniqueWorktrees(Array(20).fill(one),20));
  assert.throws(()=>uniqueWorktrees([one],20));
  assert.throws(()=>workspaceSet([{worktree_id:'a'},{worktree_id:'a'}],[{worktree_id:'a'},{worktree_id:'b'}]));
  assert.throws(()=>workspaceSet([{worktree_id:'other'}],[{worktree_id:'a'}]));
  assert.throws(()=>singleLink({nlink:2n}));
  console.log('Pure harness self-tests passed; no files, Git, owner, or corpus opened.');
}
function pathSelfTest() {
  assert.equal(process.platform,'win32');
  const holder=fs.mkdtempSync(path.join(require('node:os').tmpdir(),'devmap-checked-path-'));
  const directory=path.join(holder,'real'),file=path.join(directory,'file.txt'),link=path.join(holder,'junction');
  fs.mkdirSync(directory);fs.writeFileSync(file,'owned path fixture',{flag:'wx'});
  // Retain this tiny allocation as evidence; no corpus or user path is modified.
  console.log(JSON.stringify({owned_path_fixture:holder}));
  fs.symlinkSync(directory,link,'junction');
  assert.throws(()=>checked(link,'directory'),/Link\/reparse/);
  assert.throws(()=>checked(path.join(link,'file.txt'),'file'),/Link\/reparse/);
  assert.throws(()=>checked(path.toNamespacedPath(link),'directory'),/Link\/reparse/);
  assert.throws(()=>checked(path.toNamespacedPath(path.join(link,'file.txt')),'file'),/Link\/reparse/);
  assert.deepEqual(checked(path.toNamespacedPath(directory),'directory'),checked(directory,'directory'));
  assert.deepEqual(checked(path.toNamespacedPath(file),'file'),checked(file,'file'));
  const drive=path.parse(holder).root;
  assert.deepEqual(checked(path.toNamespacedPath(drive),'directory'),checked(drive,'directory'));
  for(const unsafe of ['\\\\server\\share\\file','\\\\?\\UNC\\server\\share\\file','\\\\.\\C:\\file','\\\\?\\GLOBALROOT\\Device\\HarddiskVolume1\\file',`\\\\?\\${directory}\\..\\real\\file.txt`]) {
    assert.throws(()=>checked(unsafe,'file'),/Unsupported|Ambiguous/);
  }
  console.log('Owned extended directory/file/root identities and ancestor junction rejection passed.');
}
function fixtureSelfTest() {
  const holder=fs.mkdtempSync(path.join(require('node:os').tmpdir(),'devmap-probe-link-'));
  const owned=path.join(holder,'owned'),template=path.join(holder,'template'),probe=path.join(owned,'probe');
  try {fs.mkdirSync(owned);fs.writeFileSync(template,'unchanged',{flag:'wx'});fs.linkSync(template,probe);
    assert.throws(()=>openProbe(probe,checked(probe,'file')),/exactly one hard link/);
    assert.equal(fs.readFileSync(template,'utf8'),'unchanged');
    console.log('Owned hard-link negative passed; no owner/Git/corpus used.');
  } finally {for(const p of [probe,template])if(fs.existsSync(p))fs.unlinkSync(p);if(fs.existsSync(owned))fs.rmdirSync(owned);fs.rmdirSync(holder);}
}
if(require.main===module){
  if(process.argv[2]==='--self-test')selfTest();
  else if(process.argv[2]==='--path-self-test')pathSelfTest();
  else if(process.argv[2]==='--fixture-self-test')fixtureSelfTest();
  else if(process.argv[2]==='--config'&&process.argv[3])main(path.resolve(process.argv[3])).catch(error=>{console.error(error);process.exitCode=1;});
  else console.log('Usage: node shared-summary-performance.cjs --self-test | --config <explicit-owned-fixture-config.json>');
}
module.exports={policy,nearestRank,within,modelFrom,envelope,sqlState,inventory,checked,
  runtime:{validateReceipt,workspaceSet,openProbe,startOwner,sameOwner,proxy,initialize,required,audit,stop,hash}};
