// Ten-minute, no-request idle observation of an explicitly owned Windows core.
// Uses read-only requests against the scale corpus; selected SQL counters are
// compared, not a complete proof that every source/backup byte is unchanged.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const net = require('node:net');
const crypto = require('node:crypto');
const {spawn, execFileSync} = require('node:child_process');
const {createInterface} = require('node:readline');
const {setTimeout: delay} = require('node:timers/promises');
const root = path.resolve(__dirname, '../..');
const verification = path.join(root, 'target/verification');
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const python = 'C:/Users/user/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/python.exe';

function within(child, parent) {
  const relative = path.relative(parent, child);
  return relative !== '' && !relative.startsWith('..') && !path.isAbsolute(relative);
}
async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const done = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Owned child did not exit')), 5000);
    child.once('exit', () => { clearTimeout(timer); resolve(); });
    child.once('error', error => { clearTimeout(timer); reject(error); });
  });
  child.kill(); // Retained ChildProcess only; never a PID learned from Hello.
  await done;
}
function sqlState(source) {
  const script = 'import sqlite3,json,sys,pathlib; p=pathlib.Path(sys.argv[1]).resolve(); c=sqlite3.connect(p.as_uri()+"?mode=ro",uri=True); c.execute("BEGIN"); print(json.dumps({"meta":c.execute("select generation,backend_state from store_meta").fetchone(),"repository_id":c.execute("select repository_id from store_meta").fetchone()[0],"sessions":c.execute("select count(*) from journal_sessions").fetchone()[0],"events":c.execute("select count(*) from journal_records").fetchone()[0]}))';
  return JSON.parse(execFileSync(python, ['-c', script, path.join(source, '.git/devmap/devmap.db')], {encoding:'utf8',timeout:15000,windowsHide:true}));
}
function hello(identity, build) {
  const input = {protocol:1,repository:identity.repository,build,source:identity.source,
    git_dir:identity.git_dir,client_instance:crypto.randomBytes(16).toString('hex')};
  return new Promise((resolve,reject) => {
    const socket = net.createConnection(`\\\\.\\pipe\\devmap-${identity.repository}`);
    const timer = setTimeout(() => socket.destroy(new Error('Hello deadline')),3000);
    let bytes = Buffer.alloc(0), settled = false;
    const finish = (error,value) => {
      if (settled) return; settled=true; clearTimeout(timer); socket.destroy();
      error ? reject(error) : resolve(value);
    };
    socket.on('error', error => finish(error));
    socket.on('close', () => { if (!settled) finish(new Error('Hello closed before response')); });
    socket.on('connect', () => {
      const body=Buffer.from(JSON.stringify(input)), size=Buffer.alloc(4);
      size.writeUInt32BE(body.length); socket.write(Buffer.concat([size,body]));
    });
    socket.on('data', chunk => {
      bytes=Buffer.concat([bytes,chunk]);
      if (bytes.length>16388) return finish(new Error('Oversized Hello'));
      if (bytes.length<4) return;
      const size=bytes.readUInt32BE();
      if (size>16384) return finish(new Error('Oversized Hello frame'));
      if (bytes.length<size+4) return;
      try {
        const result=JSON.parse(bytes.subarray(4,4+size));
        assert.equal(result.status,'Accepted',JSON.stringify(result));
        assert.equal(result.welcome.repository,input.repository);
        assert.equal(result.welcome.build,build);
        assert.equal(result.welcome.client_instance,input.client_instance);
        finish(null,result.welcome);
      } catch(error) { finish(error); }
    });
  });
}
function proxy(exe, source, fixture, index) {
  const child=spawn(exe,['mcp','--source',source],{stdio:['pipe','pipe','pipe'],windowsHide:true});
  const pending=new Map(); let next=0, stderr='';
  child.stderr.on('data', bytes => {
    stderr=(stderr+bytes).slice(-8192);
    fs.appendFileSync(path.join(fixture,`proxy-${index}.stderr.log`),bytes);
  });
  const fail=error => { for (const item of pending.values()) {clearTimeout(item.timer);item.reject(error);} pending.clear(); };
  child.on('error',fail);
  child.on('exit',code=>fail(new Error(`Proxy exited ${code}: ${stderr}`)));
  const lines=createInterface({input:child.stdout});
  lines.on('line',line=>{
    try {
      if (Buffer.byteLength(line)>4*1024*1024) throw new Error('Proxy response too large');
      const result=JSON.parse(line), item=pending.get(result.id);
      if(item){pending.delete(result.id);clearTimeout(item.timer);item.resolve(result);}
    } catch(error){fail(error);}
  });
  const request=(method,params)=>new Promise((resolve,reject)=>{
    const id=++next;
    const timer=setTimeout(()=>{pending.delete(id);reject(new Error(`Proxy ${index} ${method} deadline`));},45000);
    pending.set(id,{resolve,reject,timer});
    child.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n',error=>{if(error)fail(error);});
  });
  return {child,lines,request};
}

async function main() {
  assert.equal(process.platform,'win32');
  const input=fs.realpathSync(process.env.DEVMAP_CANDIDATE_EXE);
  const source=fs.realpathSync(process.env.DEVMAP_RESOURCE_SOURCE);
  assert(within(source,fs.realpathSync(verification)),'Only retained checkout-owned verification corpus allowed');
  const manifestPath=path.join(path.dirname(source),'manifest.json');
  const manifest=JSON.parse(fs.readFileSync(manifestPath,'utf8'));
  const fixture=fs.mkdtempSync(path.join(verification,'shared-resources-'));
  fs.copyFileSync(__filename,path.join(fixture,'harness.cjs'));
  const exe=path.join(fixture,'devmap.exe'); fs.copyFileSync(input,exe);
  const build=hash(fs.readFileSync(exe));
  const report={kind:'owned-core-default-idle',candidate_sha256:build,source,manifest,
    harness_sha256:hash(fs.readFileSync(__filename)),
    observation_seconds:600,proxy_count:4,host_task_inventory_supplied:false,
    scope:'Core-only resources; successful idle exit is not continuously resident acceptance',
    preservation_check:'Read-only requests; selected SQL counters compared. No complete logical SQL, Git or frozen-backup digest.',
    started_at:new Date().toISOString()};
  const save=()=>fs.writeFileSync(path.join(fixture,'report.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify({fixture,candidate_sha256:build})); save();
  let owner,observer; const clients=[];
  try {
    report.sql_before=sqlState(source);
    assert.deepEqual(report.sql_before.meta,[0,'active']);
    assert.equal(report.sql_before.sessions,100);assert.equal(report.sql_before.events,100000);
    const identity=JSON.parse(execFileSync(exe,['runtime','--identity','--source',source],{encoding:'utf8',timeout:15000,windowsHide:true}));
    try {
      await hello(identity,build);
      throw new Error('An existing unowned runtime occupies the fixture; refusing to replace it');
    } catch(error) {
      if(error.code!=='ENOENT') throw error;
    }
    const nonce=crypto.randomBytes(16).toString('hex');
    owner=spawn(exe,['runtime','--owner','--source',source,'--instance',nonce,'--idle-seconds','60'],{stdio:['ignore','ignore','pipe'],windowsHide:true});
    owner.once('exit',(code,signal)=>{
      report.owner_exit={code,signal,observed_at:new Date().toISOString()};save();
    });
    owner.stderr.on('data',bytes=>fs.appendFileSync(path.join(fixture,'owner.stderr.log'),bytes));
    owner.on('error',error=>{report.owner_spawn_error=String(error);save();});
    const readyDeadline=Date.now()+15000; let welcome;
    while(Date.now()<readyDeadline) {
      if(owner.exitCode!==null) throw new Error(`Owned runtime exited ${owner.exitCode}`);
      try { welcome=await hello(identity,build);break; }
      catch(error) { if(error.code!=='ENOENT') throw error; await delay(100); }
    }
    assert(welcome,'Owner readiness deadline');
    assert.equal(welcome.owner_pid,owner.pid); assert.equal(welcome.owner_instance,nonce);
    report.owner={pid:owner.pid,nonce}; report.warmups=[]; save();
    for(let index=0;index<4;index++) {
      const client=proxy(exe,source,fixture,index); clients.push(client);
      const initialized=await client.request('initialize',{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'owned-resource-fixture',version:'1'}});
      assert(!initialized.error,JSON.stringify(initialized));
      const started=performance.now();
      const response=await client.request('tools/call',{name:'devmap_read_map',arguments:{}});
      assert(!response.error && response.result?.isError!==true,JSON.stringify(response));
      const model=response.result.structuredContent;
      assert.equal(model.schema_version,'devmap/dock/4');
      assert.equal(model.repository_id,report.sql_before.repository_id);
      assert.equal(model.lanes.length,20);
      report.warmups.push({proxy_pid:client.child.pid,elapsed_ms:performance.now()-started,
        result_bytes:Buffer.byteLength(JSON.stringify(response.result)),revision:model.revision});save();
    }
    const observed=await hello(identity,build);
    assert.equal(observed.owner_pid,owner.pid);assert.equal(observed.owner_instance,nonce);
    report.idle_started_at=new Date().toISOString();save();
    console.log(JSON.stringify({phase:'no-request idle observation',owner_pid:owner.pid,seconds:600}));
    // No further Hello/Ping/tool calls occur during this observation window.
    observer=spawn(python,[path.join(root,'tests/browser/windows-process-resources.py'),
      '--pid',String(owner.pid),'--exe',exe,'--seconds','600','--interval','5',
      '--output',path.join(fixture,'resources.json')],{stdio:['ignore','pipe','pipe'],windowsHide:true,env:{...process.env,PYTHONIOENCODING:'utf-8'}});
    observer.stdout.on('data',bytes=>fs.appendFileSync(path.join(fixture,'observer.stdout.log'),bytes));
    observer.stderr.on('data',bytes=>fs.appendFileSync(path.join(fixture,'observer.stderr.log'),bytes));
    await new Promise((resolve,reject)=>{
      const timer=setTimeout(()=>reject(new Error('Resource observer exceeded bounded window')),630000);
      observer.once('error',error=>{clearTimeout(timer);reject(error);});
      observer.once('exit',code=>{clearTimeout(timer);code===0?resolve():reject(new Error(`Observer exit ${code}`));});
    });
    report.resources=JSON.parse(fs.readFileSync(path.join(fixture,'resources.json'),'utf8'));
    if(owner.exitCode!==null || owner.signalCode!==null) {
      assert.equal(owner.exitCode,0,'Owned core exited unsuccessfully during observation');
      assert.equal(owner.signalCode,null,'Owned core was terminated during observation');
      assert(report.owner_exit,'Owned core exit status was not recorded');
      report.owner_observation_outcome='successful_exit_without_harness_termination';
    } else {
      assert(!report.owner_spawn_error,'Owned core failed to start');
      report.owner_observation_outcome='alive_at_observation_end';
    }
    report.sql_after=sqlState(source);assert.deepEqual(report.sql_after,report.sql_before);
    report.completed=true;report.completed_at=new Date().toISOString();save();
    console.log(JSON.stringify({report:path.join(fixture,'report.json'),completed:true}));
  } catch(error) {report.error=String(error.stack||error);save();throw error;}
  finally {
    for(const client of clients) client.lines.close();
    const cleanup=await Promise.allSettled([...clients.map(client=>stop(client.child)),stop(observer),stop(owner)]);
    const failures=cleanup.filter(result=>result.status==='rejected');
    if(failures.length){report.cleanup_errors=failures.map(result=>String(result.reason));save();throw new Error('Owned child cleanup failed; see report');}
  }
}
main().catch(error=>{console.error(error);process.exitCode=1;});
