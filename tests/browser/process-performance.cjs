// Full-map native MCP round trips, including Git refresh. This is NOT the
// compact-summary <=200ms gate, and does not measure a real Codex host.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const crypto=require('node:crypto');
const {spawn}=require('node:child_process');
const {createInterface}=require('node:readline');
const {performance}=require('node:perf_hooks');
const root=path.resolve(__dirname,'../..');
const exe=process.env.DEVMAP_BENCHMARK_EXE;
const source=process.env.DEVMAP_BENCHMARK_SOURCE;
assert.ok(exe&&source,'Provide executable and disposable benchmark source');
const allowed=fs.realpathSync(path.join(root,'target/verification'));
const resolved=fs.realpathSync(source);
assert.ok(resolved.startsWith(allowed+path.sep),'Benchmark source must be a disposable verification repository');
function count(name,fallback) { const n=Number(process.env[name]||fallback);assert.ok(Number.isInteger(n)&&n>=1&&n<=1000);return n; }
const samples=count('DEVMAP_BENCHMARK_SAMPLES',100),coldSamples=count('DEVMAP_BENCHMARK_COLD',20),warmup=count('DEVMAP_BENCHMARK_WARMUP',10);
function client() {
  const child=spawn(exe,['mcp','--source',resolved],{stdio:['pipe','pipe','pipe'],windowsHide:true});
  const waiting=new Map();let id=0,stderr='';
  child.stderr.on('data',b=>{stderr=(stderr+b).slice(-8192);});
  const lines=createInterface({input:child.stdout});
  lines.on('line',line=>{
    try {const result=JSON.parse(line), entry=waiting.get(result.id);assert.ok(entry,'Unexpected response ID');waiting.delete(result.id);clearTimeout(entry.timeout);entry.resolve(result);}
    catch(error){for(const entry of waiting.values()){clearTimeout(entry.timeout);entry.reject(error);}waiting.clear();}
  });
  function fail(error){for(const entry of waiting.values()){clearTimeout(entry.timeout);entry.reject(error);}waiting.clear();}
  child.on('error',fail);child.on('exit',code=>fail(new Error(`MCP exited ${code}: ${stderr}`)));
  function request(method,params) {return new Promise((resolve,reject)=>{
    const current=++id;const timeout=setTimeout(()=>{waiting.delete(current);reject(new Error('Bounded MCP wait expired'));child.kill();},30000);
    waiting.set(current,{resolve,reject,timeout});
    child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:current,method,params})+'\n');
  });}
  return {async initialize(){const r=await request('initialize',{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'devmap-benchmark',version:'1'}});assert.ok(r.result&&!r.error);},
    async map(){const r=await request('tools/call',{name:'devmap_read_map',arguments:{}});assert.ok(r.result&&!r.error&&!r.result.isError,JSON.stringify(r));assert.equal(r.result.structuredContent.schema_version,'devmap/dock/4');return Buffer.byteLength(JSON.stringify(r));},
    async close(){if(child.exitCode!==null)return;const stopped=new Promise(resolve=>child.once('exit',resolve));child.stdin.end();const timer=setTimeout(()=>child.kill(),5000);await stopped;clearTimeout(timer);lines.close();assert.equal(child.exitCode,0,stderr);assert.equal(stderr,'','Unexpected process diagnostics');}};
}
function summarize(rows) {const values=rows.map(x=>x.ms).sort((a,b)=>a-b);return {count:rows.length,p50_ms:values[Math.ceil(values.length*0.5)-1],p95_ms:values[Math.ceil(values.length*0.95)-1],max_response_bytes:Math.max(...rows.map(x=>x.bytes)),samples:rows};}
async function withClient(action) {
  const c=client();let failure;
  try {await action(c);} catch(error){failure=error;}
  try {await c.close();} catch(error){failure=failure?new AggregateError([failure,error],'Benchmark request and cleanup failed'):error;}
  if(failure)throw failure;
}
async function main(){
  const cold=[],hot=[];
  for(let i=0;i<coldSamples;i++){const start=performance.now();await withClient(async c=>{await c.initialize();const bytes=await c.map();cold.push({ms:performance.now()-start,bytes});});}
  await withClient(async c=>{await c.initialize();for(let i=0;i<warmup;i++)await c.map();for(let i=0;i<samples;i++){const start=performance.now(),bytes=await c.map();hot.push({ms:performance.now()-start,bytes});}});
  const report={scope:'native_mcp_full_map_including_git',source:resolved,executable:exe,executable_sha256:crypto.createHash('sha256').update(fs.readFileSync(exe)).digest('hex'),warmup,cold:summarize(cold),hot:summarize(hot),measured_at:new Date().toISOString(),note:'Full map includes Git refresh and IPC, not compact summary. No CPU/RSS/idle or real host measurement. Sample counts below 20 cold and 100 hot are smoke only.'};
  const output=process.env.DEVMAP_BENCHMARK_OUTPUT||path.join(root,'target/verification/process-performance.json');fs.writeFileSync(output,JSON.stringify(report,null,2));console.log(JSON.stringify(report));
}
main().catch(error=>{
  const describe=e=>({name:e.name,message:e.message,errors:e.errors?.map(describe)});
  const output=process.env.DEVMAP_BENCHMARK_OUTPUT||path.join(root,'target/verification/process-performance.json');
  fs.writeFileSync(output,JSON.stringify({scope:'native_mcp_full_map_including_git',passed:false,source:resolved,error:describe(error)},null,2));
  console.error(error);process.exitCode=1;
});
