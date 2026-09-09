'use strict';
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {spawn}=require('node:child_process');
const runtime=require('./shared-summary-runtime.cjs');
function policy(mode){assert(['smoke','acceptance'].includes(mode));return {clients:4,warmups:10,measured:0,seconds:mode==='acceptance'?600:3,interval:mode==='acceptance'?5:.25,cpu_percent_one_core:1,rss_bytes:150*1024*1024};}
function analyze(resources,exit,seconds){
  const samples=resources.samples;assert(Array.isArray(samples)&&samples.length>=2);
  const first=samples[0],last=samples.at(-1);assert(first.alive,'Owner must be alive when retained-handle sampling begins');
  assert(samples.every(s=>s.created_filetime===first.created_filetime),'Process creation identity changed');
  let ended=false;for(let i=0;i<samples.length;i++){const s=samples[i];assert(!ended||!s.alive,'Exited process became alive');ended||=!s.alive;
    assert(Number.isFinite(s.cpu_seconds)&&Number.isFinite(s.elapsed_seconds));
    if(i){assert(s.elapsed_seconds>=samples[i-1].elapsed_seconds);assert(s.cpu_seconds>=samples[i-1].cpu_seconds);}}
  const elapsed=last.elapsed_seconds-first.elapsed_seconds;assert(elapsed>=seconds-.05,'Observation window incomplete');
  const cpu=last.cpu_seconds-first.cpu_seconds, alive=samples.filter(s=>s.alive),lastAlive=alive.at(-1),firstExit=samples.find(s=>!s.alive);
  const lower=lastAlive.elapsed_seconds-first.elapsed_seconds,upper=firstExit?firstExit.elapsed_seconds-first.elapsed_seconds:elapsed;
  let activeSeconds=elapsed;
  if(firstExit){
    assert(exit,'Missing retained ChildProcess exit event');assert.equal(exit.code,0,'Owner exited unsuccessfully');assert.equal(exit.signal,null,'Owner was signalled');
    assert.equal(exit.phase,'idle','Owner exit was not natural within idle window');assert.equal(last.exit_code,0,'Native handle reports nonzero exit');
    assert(BigInt(last.exit_filetime)>0n,'Missing native exit FILETIME');
    activeSeconds=Number(BigInt(last.exit_filetime)-BigInt(first.sampled_filetime))/10000000;
    assert(activeSeconds>0&&activeSeconds>=lower-.1&&activeSeconds<=upper+.1,'Native exit time inconsistent with monotonic sample bounds');
  }else {assert(!exit,'Node/native liveness disagreement');}
  const peak=Math.max(...samples.map(s=>s.rss_bytes)),lifetimePeak=Math.max(...samples.map(s=>s.lifetime_peak_rss_bytes||0));
  const activeCpu=100*cpu/activeSeconds,windowCpu=100*cpu/elapsed;
  return {elapsed_seconds:elapsed,first_sample:first,last_alive_sample:lastAlive,first_exited_sample:firstExit||null,
    normal_exit_confirmed:Boolean(firstExit),alive_for_entire_window:!firstExit,
    cpu_seconds_in_window:cpu,active_interval_seconds:activeSeconds,active_interval_sample_bounds_seconds:{lower,upper},
    active_interval_cpu_percent_one_core:activeCpu,full_window_cpu_percent_one_core:windowCpu,
    peak_sampled_rss_bytes:peak,lifetime_peak_rss_bytes:lifetimePeak,
    active_cpu_gate:activeCpu<1,full_window_cpu_gate:windowCpu<1,rss_gate:Math.max(peak,lifetimePeak)<=150*1024*1024,
    note:'CPU denominator separates actual alive interval from full window. Lifetime peak includes startup. Owner only; excludes proxies, Git descendants, Python and Node.'};
}
function startObserver(ctx,owner,script,protocol){
  ctx.verifyRun();ctx.verifyFixture();
  const output=path.join(ctx.run,'resources.json');assert(!fs.existsSync(output));
  const child=spawn(ctx.python,[script,'--owned-run-receipt',path.join(ctx.run,'creation.json'),'--pid',String(owner.entry.child.pid),'--exe',ctx.exe,
    '--seconds',String(protocol.seconds),'--interval',String(protocol.interval),'--output',output],{windowsHide:true,stdio:['ignore','pipe','pipe'],env:{...process.env,PYTHONIOENCODING:'utf-8'}});
  const entry={child,label:'resource-observer',spawn_error:null,exit:null};ctx.children.push(entry);
  entry.closed=new Promise(resolve=>{child.once('error',error=>{entry.spawn_error=String(error);ctx.asyncError??=error;resolve();});child.once('close',(code,signal)=>{entry.exit={code,signal};resolve();});});
  for(const [name,stream] of [['stdout',child.stdout],['stderr',child.stderr]]){
    stream.on('data',bytes=>{try{ctx.append(`observer.${name}`,bytes);}catch(error){ctx.asyncError??=error;}});stream.on('error',error=>{ctx.asyncError??=error;});
  }
  return {entry,output};
}
async function main(configPath){
  const config=JSON.parse(fs.readFileSync(configPath,'utf8')),protocol=policy(config.mode);
  const observerSource=path.join(__dirname,'windows-process-resources.py');
  const ctx=runtime.createRun(config,[__filename,require.resolve('./shared-summary-runtime.cjs'),require.resolve('./shared-summary-performance.cjs'),observerSource],protocol);
  Object.assign(ctx.report,{schema:'devmap/shared-summary-resources/1',scope:'Owner-only no-request idle; 600-second default lifecycle observation is distinct from continuous residency',
    default_idle_resource_acceptance:false,continuous_residency_acceptance:false,owner_exit:null,resource_budgets:{cpu_percent_one_core:'<1',rss_bytes:protocol.rss_bytes}});
  let owner,phase='setup',observer,killOwnerRequested=false;
  try{
    ctx.prepare();owner=await runtime.startOwner(ctx,'resource-owner');
    owner.entry.child.on('close',(code,signal)=>{
      ctx.report.owner_exit={code,signal,phase,harness_termination_requested:killOwnerRequested,observed_at:new Date().toISOString(),observed_monotonic_ms:performance.now()};
      if(phase==='idle'&&(code!==0||signal!==null||killOwnerRequested))ctx.asyncError??=new Error('Owned core did not exit normally');
    });
    const clients=[];
    for(let i=0;i<protocol.clients;i++){const c=runtime.proxy(ctx.exe,ctx.receipt.source,`resource-proxy-${i}`,ctx);clients.push(c);await runtime.initialize(c);}
    for(let round=0;round<protocol.warmups;round++){
      ctx.health();const results=await Promise.allSettled(clients.map(c=>runtime.required(c,'resource-warmup',round,ctx)));
      for(const result of results)if(result.status==='rejected')throw result.reason;
    }
    await runtime.audit(clients[0],ctx,'resource-baseline-cursor');
    ctx.preserve();await runtime.sameOwner(ctx,owner);
    ctx.report.last_request_completed_at=new Date().toISOString();ctx.report.last_request_completed_monotonic_ms=performance.now();
    ctx.report.last_request_kind='identity continuity Hello after four-client summary warmup';
    // Existing child helper's stopping flag suppresses its generic unexpected-exit
    // callback. No kill is issued here: the handler above checks natural code/signal.
    owner.entry.stopping=true;phase='idle';
    const stopped=await Promise.allSettled(clients.map(c=>runtime.stop(c.entry)));
    for(const result of stopped)if(result.status==='rejected')throw result.reason;
    ctx.health();assert(!owner.entry.exit,'Owner exited before retained sampler start');
    ctx.report.proxies_closed_at=new Date().toISOString();ctx.report.proxies_closed_monotonic_ms=performance.now();
    ctx.report.idle_started_at=new Date().toISOString();ctx.report.idle_started_monotonic_ms=performance.now();ctx.save();
    // No Hello, Ping, tools/call, Git or SQL is sent until this OS-only window ends.
    observer=startObserver(ctx,owner,path.join(ctx.run,path.basename(observerSource)),protocol);
    let timer;
    try{await Promise.race([observer.entry.closed,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('Retained observer exceeded bounded window')),protocol.seconds*1000+30000);})]);}
    finally{clearTimeout(timer);}
    assert(!observer.entry.spawn_error,observer.entry.spawn_error);assert.deepEqual(observer.entry.exit,{code:0,signal:null});ctx.health();ctx.verifyRun();
    const resources=JSON.parse(fs.readFileSync(observer.output,'utf8'));
    assert.equal(resources.pid,owner.entry.child.pid);assert.equal(runtime.checked(resources.image,'file').path,runtime.checked(ctx.exe,'file').path);
    assert.equal(resources.requested_seconds,protocol.seconds);ctx.report.resources=resources;
    ctx.report.owner_window_outcome=ctx.report.owner_exit?{...ctx.report.owner_exit}:{alive:true,phase:'idle-window-end'};
    ctx.report.analysis=analyze(resources,ctx.report.owner_exit,protocol.seconds);
    ctx.report.sample_window_finished_at=new Date().toISOString();phase='after-window';
    ctx.preserve();ctx.report.preservation_after_window=true;
  }catch(error){ctx.report.errors.push(String(error.stack||error));}
  finally{
    phase='cleanup';killOwnerRequested=Boolean(owner&&!owner.entry.exit&&!owner.entry.spawn_error);
    const cleanup=await Promise.allSettled(ctx.children.map(runtime.stop));
    ctx.report.cleanup_errors=cleanup.filter(r=>r.status==='rejected').map(r=>String(r.reason));
    ctx.report.child_lifecycle=ctx.children.map(c=>({label:c.label,pid:c.child.pid,exit:c.exit,spawn_error:c.spawn_error}));
    try{ctx.preserve();ctx.report.final_preservation=true;}catch(error){ctx.report.errors.push(`Final preservation: ${error.stack||error}`);}
    if(ctx.asyncError)ctx.report.errors.push(`Asynchronous failure: ${ctx.asyncError.stack||ctx.asyncError}`);
    ctx.report.sqlite_sidecars_after=ctx.sidecars();ctx.report.completed=ctx.report.errors.length===0&&ctx.report.cleanup_errors.length===0&&Boolean(ctx.report.analysis);
    const a=ctx.report.analysis,acceptance=ctx.report.completed&&config.mode==='acceptance'&&protocol.seconds===600;
    ctx.report.default_idle_resource_acceptance=Boolean(acceptance&&a.normal_exit_confirmed&&a.active_cpu_gate&&a.full_window_cpu_gate&&a.rss_gate);
    ctx.report.continuous_residency_acceptance=Boolean(acceptance&&a.alive_for_entire_window&&a.active_cpu_gate&&a.rss_gate);
    ctx.report.performance_acceptance=false;ctx.report.finished_at=new Date().toISOString();ctx.closeLogs();ctx.save();
  }
  assert(ctx.report.completed,`Resource observation failed; retained report: ${ctx.run}`);
  assert(config.mode!=='acceptance'||ctx.report.default_idle_resource_acceptance||ctx.report.continuous_residency_acceptance,`Owner resource budget failed; retained report: ${ctx.run}`);
  console.log(JSON.stringify({run:ctx.run,completed:true,default_idle_resource_acceptance:ctx.report.default_idle_resource_acceptance,continuous_residency_acceptance:ctx.report.continuous_residency_acceptance,performance_acceptance:false}));
}
function selfTest(){
  const sample=(t,alive,cpu,rss=1024)=>({elapsed_seconds:t,alive,cpu_seconds:cpu,rss_bytes:alive?rss:0,lifetime_peak_rss_bytes:alive?rss:null,created_filetime:1,sampled_filetime:String(10000000000n+BigInt(t*10000000)),exit_filetime:alive?'0':'10600000000',exit_code:alive?null:0});
  const exited={samples:[sample(0,true,1),sample(55,true,1.1),sample(60,false,1.2),sample(600,false,1.2)]};
  const exit={code:0,signal:null,phase:'idle'};const result=analyze(exited,exit,600);
  assert.equal(result.active_interval_seconds,60);assert.equal(result.alive_for_entire_window,false);
  assert(result.active_interval_cpu_percent_one_core>result.full_window_cpu_percent_one_core);assert(result.normal_exit_confirmed);
  assert.throws(()=>analyze(exited,{...exit,code:1},600));assert.throws(()=>analyze(exited,{...exit,signal:'SIGTERM'},600));
  assert.throws(()=>analyze({samples:exited.samples.slice(0,3)},exit,600));
  assert.throws(()=>analyze(exited,null,600));
  const resident=analyze({samples:[sample(0,true,1),sample(600,true,2)]},null,600);assert(resident.alive_for_entire_window);
  const highCpu=analyze({samples:[sample(0,true,1),sample(55,true,1.1),sample(60,false,2),sample(600,false,2)]},exit,600);
  assert(highCpu.full_window_cpu_gate&&!highCpu.active_cpu_gate,'Exited zero-CPU tail must not hide active CPU violation');
  assert.equal(policy('acceptance').seconds,600);assert.equal(policy('acceptance').rss_bytes,157286400);
  console.log('Pure resource window/lifecycle/budget tests passed; no owner/observer/fixture opened.');
}
if(require.main===module){if(process.argv[2]==='--self-test')selfTest();else if(process.argv[2]==='--config'&&process.argv[3])main(path.resolve(process.argv[3])).catch(error=>{console.error(error);process.exitCode=1;});else console.log('Usage: node shared-summary-resources.cjs --self-test | --config <owned-config.json>');}
module.exports={policy,analyze};
