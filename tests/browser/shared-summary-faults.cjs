'use strict';
// Transport-only failures using the real exported helpers, never a corpus.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),net=require('node:net'),crypto=require('node:crypto');
const {spawn}=require('node:child_process');
const {setTimeout:delay}=require('node:timers/promises');
const {runtime:api,checked}=require('./shared-summary-performance.cjs');
const MOCK_MCP=String.raw`
const fs=require('node:fs'),readline=require('node:readline');
const config=JSON.parse(fs.readFileSync(process.argv[process.argv.indexOf('--source')+1]));
const lines=readline.createInterface({input:process.stdin});
lines.on('line',line=>{const q=JSON.parse(line);
 if(config.mode==='hang')return;
 if(config.mode==='exit'){process.exitCode=23;lines.close();process.stdin.destroy();return;}
 const result=q.method==='initialize'?{protocolVersion:'2025-11-25',capabilities:{},serverInfo:{name:'owned-mock',version:'1'}}:
 {isError:true,content:[{type:'text',text:'deliberate owned mock failure'}],structuredContent:{schema_version:'devmap-summary/1',error:{code:'owned_mock_failure'}}};
 process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:q.id,result})+'\n');
});
`;
const MOCK_OWNER=String.raw`
const fs=require('node:fs'),net=require('node:net');
const config=JSON.parse(fs.readFileSync(process.argv[process.argv.indexOf('--source')+1]));
const instance=process.argv[process.argv.indexOf('--instance')+1];
const server=net.createServer(socket=>{let bytes=Buffer.alloc(0);socket.on('error',()=>{});
 socket.on('data',chunk=>{bytes=Buffer.concat([bytes,chunk]);if(bytes.length<4)return;const n=bytes.readUInt32BE();if(n>16384){socket.destroy();return;}if(bytes.length<n+4)return;
 const hello=JSON.parse(bytes.subarray(4,n+4));const body=Buffer.from(JSON.stringify({status:'Accepted',welcome:{protocol:1,repository:hello.repository,build:hello.build,client_instance:hello.client_instance,owner_pid:process.pid,owner_instance:instance}}));
 const size=Buffer.alloc(4);size.writeUInt32BE(body.length);socket.end(Buffer.concat([size,body]));
 setTimeout(()=>server.close(()=>{process.exitCode=27;}),150);
 });});
server.listen('\\\\.\\pipe\\devmap-'+config.repository);
`;
function bounded(promise,ms,label){let timer;return Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error(label)),ms);})]).finally(()=>clearTimeout(timer));}
async function worker(run){
  const identity=checked(run,'directory');assert.equal(checked(process.cwd(),'directory').path,identity.path);
  const verify=()=>assert.deepEqual(checked(run,'directory'),identity);
  const outcomes=[],contexts=[];
  const context=(mode)=>{
    verify();const nonce=crypto.randomBytes(16).toString('hex'),source=path.join(run,`source-${nonce}.json`),repository=crypto.randomBytes(32).toString('hex');
    fs.writeFileSync(source,JSON.stringify({mode,repository}),{flag:'wx'});
    const ctx={exe:process.execPath,build:'a'.repeat(64),receipt:{source},runtimeIdentity:{repository,source,git_dir:run},children:[],rows:[],report:{owners:[]},asyncError:null,
      verifyRun:verify,verifyFixture:verify,save:()=>{},append:(name,bytes)=>{verify();assert.equal(path.basename(name),name);assert(Buffer.byteLength(bytes)<=4*1024*1024);fs.appendFileSync(path.join(run,name),bytes);}};
    contexts.push(ctx);return ctx;
  };
  const test=async(name,body)=>{const start=performance.now();try{await body();outcomes.push({name,passed:true,elapsed_ms:performance.now()-start});}
    catch(error){outcomes.push({name,passed:false,error:String(error.stack||error)});throw error;}
    finally{verify();fs.writeFileSync(path.join(run,'fault-outcomes.json'),JSON.stringify(outcomes,null,2));}};
  let failure;
  try {
    await test('occupied-endpoint-refused-before-spawn',async()=>{
      const ctx=context('occupied'),sockets=new Set();
      const server=net.createServer(socket=>{sockets.add(socket);socket.on('close',()=>sockets.delete(socket));socket.on('error',()=>{});let data=Buffer.alloc(0);
        socket.on('data',b=>{data=Buffer.concat([data,b]);assert(data.length<=16388);if(data.length<4)return;const n=data.readUInt32BE();if(data.length<n+4)return;
          const h=JSON.parse(data.subarray(4,n+4)),body=Buffer.from(JSON.stringify({status:'Accepted',welcome:{repository:h.repository,build:h.build,client_instance:h.client_instance,owner_pid:process.pid,owner_instance:'owned-test-endpoint'}}));
          const length=Buffer.alloc(4);length.writeUInt32BE(body.length);socket.end(Buffer.concat([length,body]));});});
      try {await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(`\\\\.\\pipe\\devmap-${ctx.runtimeIdentity.repository}`,resolve);});
        await assert.rejects(api.startOwner(ctx,'must-not-spawn'),/Unowned endpoint already occupied/);assert.equal(ctx.children.length,0);assert(server.listening);
      }finally{for(const socket of sockets)socket.destroy();await new Promise(resolve=>server.close(resolve));}
    });
    await test('missing-executable-retained-and-pending-rejected',async()=>{
      const ctx=context('error'),client=api.proxy(path.join(run,'missing-executable.exe'),ctx.receipt.source,'missing-exe',ctx);
      assert.equal(ctx.children.length,1);await assert.rejects(client.request('initialize',{},1000));await api.stop(client.entry);
      assert(client.entry.spawn_error);assert(ctx.asyncError);
    });
    await test('tool-error-result-is-not-success-sample',async()=>{
      const ctx=context('error'),client=api.proxy(process.execPath,ctx.receipt.source,'tool-error',ctx);
      try {await api.initialize(client);await assert.rejects(api.required(client,'injected-tool-error',0,ctx),/owned_mock_failure/);
        assert.equal(ctx.rows.length,1);assert(ctx.rows[0].error);assert(ctx.rows[0].result_bytes>0&&ctx.rows[0].result_bytes<=32768);
      }finally{await api.stop(client.entry);}
    });
    await test('request-timeout-rejects-real-pending-entry',async()=>{
      const ctx=context('hang'),client=api.proxy(process.execPath,ctx.receipt.source,'timeout',ctx);
      try {await assert.rejects(client.request('tools/call',{},100),/MCP deadline/);}finally{await api.stop(client.entry);}
      assert(client.entry.exit);
    });
    await test('child-exit-rejects-inflight-request',async()=>{
      const ctx=context('exit'),client=api.proxy(process.execPath,ctx.receipt.source,'pending-exit',ctx);
      try {await assert.rejects(client.request('tools/call',{},3000),/Proxy stdout ended|Proxy closed/);
        await bounded(client.entry.closed,3000,'Exited mock not closed');assert.equal(client.entry.exit.code,23);assert(ctx.asyncError);
      }finally{await api.stop(client.entry);}
    });
    await test('accepted-owned-owner-unexpected-exit-is-rejected',async()=>{
      const ctx=context('owner-exit'),owner=await api.startOwner(ctx,'owner-exit');
      assert.equal(ctx.report.owners.length,1);await bounded(owner.entry.closed,3000,'Mock owner exit missing');
      assert.equal(owner.entry.exit.code,27);assert(ctx.asyncError);await assert.rejects(api.sameOwner(ctx,owner),/Owned owner exited/);await api.stop(owner.entry);
    });
    await test('allsettled-does-not-skip-other-owned-child-on-kill-error',async()=>{
      const ctx=context('hang'),first=api.proxy(process.execPath,ctx.receipt.source,'cleanup-first',ctx),second=api.proxy(process.execPath,ctx.receipt.source,'cleanup-second',ctx);
      const kill=first.entry.child.kill;
      try {
        // Deliberate local API failure on a real retained child, not an OS/PID
        // simulation claim. Restore its original kill and reap it in finally.
        first.entry.child.kill=()=>{throw new Error('injected retained-child kill error');};
        const results=await Promise.allSettled([api.stop(first.entry),api.stop(second.entry)]);
        assert.equal(results[0].status,'rejected');assert.match(String(results[0].reason),/injected retained-child kill error/);
        assert.equal(results[1].status,'fulfilled');assert(second.entry.exit);
      }finally{first.entry.child.kill=kill;await api.stop(first.entry);await api.stop(second.entry);}
    });
  }catch(error){failure=error;}
  finally {
    const cleanup=await Promise.allSettled(contexts.flatMap(ctx=>ctx.children).map(entry=>api.stop(entry)));
    verify();fs.writeFileSync(path.join(run,'worker-report.json'),JSON.stringify({scope:'exported transport helpers with owned mocks; no fixture/SQL/real owner acceptance',outcomes,
      cleanup:cleanup.map(r=>({status:r.status,error:r.status==='rejected'?String(r.reason):null})),children:contexts.flatMap(ctx=>ctx.children.map(e=>({label:e.label,pid:e.child.pid,exit:e.exit,spawn_error:e.spawn_error})))},null,2));
    assert(cleanup.every(r=>r.status==='fulfilled'),'Fault suite final cleanup failed');
  }
  if(failure)throw failure;assert.equal(outcomes.length,7);
}
async function run(config){
  assert.equal(process.platform,'win32');const parent=checked(config.run_parent,'directory'),python=checked(config.python,'file');
  const dir=fs.mkdtempSync(path.join(parent.path,'summary-faults-')),id=checked(dir,'directory');
  fs.writeFileSync(path.join(dir,'creation.json'),JSON.stringify({nonce:crypto.randomBytes(16).toString('hex'),identity:id}),{flag:'wx'});
  fs.writeFileSync(path.join(dir,'mcp'),MOCK_MCP,{flag:'wx'});fs.writeFileSync(path.join(dir,'runtime'),MOCK_OWNER,{flag:'wx'});
  const jobReport=path.join(dir,'worker-job.json'),stdout=fs.openSync(path.join(dir,'worker.stdout'),'wx'),stderr=fs.openSync(path.join(dir,'worker.stderr'),'wx');
  const child=spawn(python.path,[path.join(__dirname,'windows-owned-generator-job.py'),'--report',jobReport,'--exe',process.execPath,'--',__filename,'--worker',dir],{cwd:dir,windowsHide:true,stdio:['pipe',stdout,stderr]});
  let closed=false,error,primaryError,resolveClose;
  const actualClose=new Promise(resolve=>{resolveClose=resolve;});
  const done=new Promise(resolve=>{child.once('error',e=>{error=e;resolve();});child.once('close',(code,signal)=>{closed=true;resolveClose({code,signal});resolve({code,signal});});});
  child.stdin.on('error',e=>{error??=e;});
  try {
    const status=await bounded(done,60000,'Fault worker deadline');if(error)throw error;assert.equal(status.code,0,'See retained worker report');
    const proof=JSON.parse(fs.readFileSync(jobReport,'utf8'));assert.equal(proof.empty_confirmed,true);assert.equal(proof.root_exit_code,0);
  } catch(e) {primaryError=e;throw e;
  } finally {
    let cleanupError,proof;
    try {
      if(Number.isInteger(child.pid)){
        if(!closed){
          if(child.stdin.writable&&!child.stdin.destroyed)child.stdin.end('abort\n');
          else child.kill(); // Retained wrapper only; Job closes, confirmation still required.
          try{await bounded(actualClose,12000,'Fault Job abort deadline');}
          catch(e){child.kill();await bounded(actualClose,5000,'Fault wrapper reap deadline');throw e;}
        }
        assert(closed,'Retained wrapper did not close');
        proof=JSON.parse(fs.readFileSync(jobReport,'utf8'));
        assert.equal(proof.empty_confirmed,true,'Failure cleanup lacks Job-empty confirmation');
      }
    } catch(e){cleanupError=e;}
    finally {
      fs.closeSync(stdout);fs.closeSync(stderr);assert.deepEqual(checked(dir,'directory'),id);
      fs.writeFileSync(path.join(dir,'parent-cleanup.json'),JSON.stringify({pid:child.pid,closed,
        primary_error:String(primaryError||''),transport_error:String(error||''),cleanup_error:String(cleanupError||''),job:proof,
        empty_confirmed:proof?.empty_confirmed===true},null,2),{flag:'wx'});
    }
    if(cleanupError)throw cleanupError;
    if(error)throw error;
  }
  console.log(JSON.stringify({run:dir,completed:true,cases:7,performance_acceptance:false}));
}
if(require.main===module){
  if(process.argv[2]==='--worker')worker(path.resolve(process.argv[3])).catch(e=>{console.error(e);process.exitCode=1;});
  else if(process.argv[2]==='--config')run(JSON.parse(fs.readFileSync(process.argv[3],'utf8'))).catch(e=>{console.error(e);process.exitCode=1;});
  else console.log('Root-run only: node shared-summary-faults.cjs --config {run_parent, python} JSON path');
}
