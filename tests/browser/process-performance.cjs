// Full-map native MCP round trips, including Git refresh. This is NOT the
// compact-summary <=200ms gate, and does not measure a real Codex host.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const crypto=require('node:crypto');
const {spawn}=require('node:child_process');
const {createInterface}=require('node:readline');
const {performance}=require('node:perf_hooks');
const {fingerprint}=require('./full-map-fingerprint.cjs');
const root=path.resolve(__dirname,'../..');
const exe=process.env.DEVMAP_BENCHMARK_EXE;
const source=process.env.DEVMAP_BENCHMARK_SOURCE;
assert.ok(exe&&source,'Provide executable and disposable benchmark source');
const allowed=fs.realpathSync(path.join(root,'target/verification'));
const resolved=fs.realpathSync(source);
assert.ok(resolved.startsWith(allowed+path.sep),'Benchmark source must be a disposable verification repository');
function count(name,fallback) { const n=Number(process.env[name]||fallback);assert.ok(Number.isInteger(n)&&n>=1&&n<=1000);return n; }
const samples=count('DEVMAP_BENCHMARK_SAMPLES',100),coldSamples=count('DEVMAP_BENCHMARK_COLD',20),warmup=count('DEVMAP_BENCHMARK_WARMUP',10);
const clients=count('DEVMAP_BENCHMARK_CLIENTS',1);
const progress=event=>{if(process.env.DEVMAP_BENCHMARK_PROGRESS==='1')console.log(JSON.stringify({progress:event}));};
assert.ok(clients<=4,'At most four concurrent clients');
const verifyModel=process.env.DEVMAP_BENCHMARK_VERIFY_MODEL==='1';
const modelMode=process.env.DEVMAP_BENCHMARK_MODEL_MODE||'strict';
assert(['strict','fresh-git-observation','legacy-refresh'].includes(modelMode));
const clientObservations=new WeakMap();let initialObservation;
if(modelMode==='legacy-refresh')assert.equal(crypto.createHash('sha256').update(fs.readFileSync(exe)).digest('hex'),'a1cfbb1c46bd9026b18db67da9f70485b0bb20c6c54fd779475b52531731d419','Legacy counter policy is restricted to the frozen old executable');
let firstModel,modelHash=process.env.DEVMAP_BENCHMARK_MODEL_SHA256;
const mismatchedModels=[];
if(modelHash)assert.match(modelHash,/^[a-f0-9]{64}$/);
function audit(c,row){
  if(!verifyModel)return;
  let model=c.model(),mode=modelMode;
  if(modelMode==='legacy-refresh'){
    const counter=model.observation_revision;
    assert(Number.isSafeInteger(counter)&&counter>0);
    const previous=clientObservations.get(c);
    initialObservation??=counter;
    assert.equal(counter,previous===undefined?initialObservation:previous+1,'Legacy refresh counter must start consistently and advance once per map');
    clientObservations.set(c,counter);
    if(row)row.observation_revision=counter;
    model={...model,observation_revision:'@verified-per-client-refresh-counter'};mode='fresh-git-observation';
  }
  const actual=fingerprint(model,mode);
  if(row)row.model_sha256=actual;
  firstModel??=c.model();modelHash??=actual;
  if(actual!==modelHash&&mismatchedModels.length<4)mismatchedModels.push(c.model());
  assert.equal(actual,modelHash,'Full map changed beyond the explicit timestamp policy');
}
function client() {
  const child=spawn(exe,['mcp','--source',resolved],{stdio:['pipe','pipe','pipe'],windowsHide:true});
  const waiting=new Map();let id=0,stderr='',spawnFailure,lastModel;
  child.stderr.on('data',b=>{stderr=(stderr+b).slice(-8192);});
  const lines=createInterface({input:child.stdout});
  lines.on('line',line=>{
    try {const result=JSON.parse(line), entry=waiting.get(result.id);assert.ok(entry,'Unexpected response ID');waiting.delete(result.id);clearTimeout(entry.timeout);entry.resolve(result);}
    catch(error){for(const entry of waiting.values()){clearTimeout(entry.timeout);entry.reject(error);}waiting.clear();}
  });
  function fail(error){for(const entry of waiting.values()){clearTimeout(entry.timeout);entry.reject(error);}waiting.clear();}
  child.on('error',error=>{spawnFailure=error;fail(error);});child.on('exit',code=>fail(new Error(`MCP exited ${code}: ${stderr}`)));
  function request(method,params) {return new Promise((resolve,reject)=>{
    const current=++id;const timeout=setTimeout(()=>{waiting.delete(current);reject(new Error('Bounded MCP wait expired'));child.kill();},30000);
    waiting.set(current,{resolve,reject,timeout});
    child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:current,method,params})+'\n');
  });}
  return {async initialize(){const r=await request('initialize',{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'devmap-benchmark',version:'1'}});assert.ok(r.result&&!r.error);},
    async map(){const r=await request('tools/call',{name:'devmap_read_map',arguments:{}});assert.ok(r.result&&!r.error&&!r.result.isError,JSON.stringify(r));assert.equal(r.result.structuredContent.schema_version,'devmap/dock/4');lastModel=r.result.structuredContent;return Buffer.byteLength(JSON.stringify(r));},
    pid:child.pid,model(){return lastModel;},
    async close(){if(spawnFailure){lines.close();throw spawnFailure;}if(child.exitCode!==null)return;const stopped=new Promise(resolve=>child.once('exit',resolve));child.stdin.end();const timer=setTimeout(()=>child.kill(),5000);await stopped;clearTimeout(timer);lines.close();assert.equal(child.exitCode,0,stderr);assert.equal(stderr,'','Unexpected process diagnostics');}};
}
function summarize(rows) {const values=rows.map(x=>x.ms).sort((a,b)=>a-b);return {count:rows.length,p50_ms:values[Math.ceil(values.length*0.5)-1],p95_ms:values[Math.ceil(values.length*0.95)-1],max_response_bytes:Math.max(...rows.map(x=>x.bytes)),samples:rows};}
async function withClient(action) {
  const c=client();let failure;
  try {await action(c);} catch(error){failure=error;}
  try {await c.close();} catch(error){failure=failure?new AggregateError([failure,error],'Benchmark request and cleanup failed'):error;}
  if(failure)throw failure;
}
let population;
async function main(){
  const cold=[],hot=[],cohorts=Array.from({length:clients},(_,client)=>({client,samples:[],errors:[]}));
  population={cold,cohorts};
  for(let i=0;i<coldSamples;i++){const start=performance.now();await withClient(async c=>{await c.initialize();const bytes=await c.map();const row={ms:performance.now()-start,bytes,pid:c.pid};cold.push(row);audit(c,row);});progress({phase:'cold',completed:cold.length,total:coldSamples});}
  // A shared gate starts measured traffic only after every client has warmed up.
  // A failed warmup rejects that gate so peers cannot wait indefinitely.
  let ready=0,release,rejectGate;
  const gate=new Promise((resolve,reject)=>{release=resolve;rejectGate=reject;});
  gate.catch(()=>{});
  const outcomes=await Promise.allSettled(cohorts.map(cohort=>withClient(async c=>{
    try {
      cohort.pid=c.pid;
      await c.initialize();for(let i=0;i<warmup;i++){await c.map();audit(c);}
      if(++ready===clients)release();await gate;
      for(let i=0;i<samples;i++){
        const start=performance.now(),bytes=await c.map();
        const row={client:cohort.client,sequence:i,ms:performance.now()-start,bytes};
        cohort.samples.push(row);hot.push(row);
        audit(c,row);
        if((i+1)%10===0)progress({phase:'warm',client:cohort.client,completed:i+1,total:samples});
      }
    } catch(error) {rejectGate(error);throw error;}
  }).catch(error=>{cohort.errors.push({name:error.name,message:error.message});throw error;})));
  const failures=outcomes.filter(outcome=>outcome.status==='rejected');
  if(failures.length){
    const error=new AggregateError(failures.map(outcome=>outcome.reason),'Concurrent full-map clients failed');
    error.population={cold,cohorts};throw error;
  }
  const report={scope:'native_mcp_full_map_including_git',source:resolved,executable:exe,executable_sha256:crypto.createHash('sha256').update(fs.readFileSync(exe)).digest('hex'),warmup,cold:summarize(cold),hot:summarize(hot),measured_at:new Date().toISOString(),note:'Full map includes Git refresh and IPC, not compact summary. No CPU/RSS/idle or real host measurement. Sample counts below 20 cold and 100 hot are smoke only.'};
  report.clients=clients;report.cohorts=cohorts.map(cohort=>({...cohort,summary:summarize(cohort.samples)}));
  report.model_audit={enabled:verifyModel,mode:modelMode,sha256:modelHash,first_model:firstModel};
  const output=process.env.DEVMAP_BENCHMARK_OUTPUT||path.join(root,'target/verification/process-performance.json');fs.writeFileSync(output,JSON.stringify(report,null,2));console.log(JSON.stringify(report));
}
main().catch(error=>{
  const describe=e=>({name:e.name,message:e.message,errors:e.errors?.map(describe)});
  const output=process.env.DEVMAP_BENCHMARK_OUTPUT||path.join(root,'target/verification/process-performance.json');
  fs.writeFileSync(output,JSON.stringify({scope:'native_mcp_full_map_including_git',passed:false,source:resolved,population:error.population||population,model_audit:{enabled:verifyModel,mode:modelMode,sha256:modelHash,first_model:firstModel,mismatched_models:mismatchedModels},error:describe(error)},null,2));
  console.error(error);process.exitCode=1;
});
