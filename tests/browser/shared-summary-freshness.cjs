'use strict';
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {execFileSync}=require('node:child_process');
const {setTimeout:delay}=require('node:timers/promises');
const runtime=require('./shared-summary-runtime.cjs');
function policy(mode){assert(['smoke','acceptance'].includes(mode));return {clients:4,warmups:mode==='acceptance'?10:2,measured:mode==='acceptance'?100:4,cadence_ms:100,trial_ms:15000,overall_ms:45*60*1000,p95_ms:2000};}
function nextTick(start,now,cadence=100){return start+(Math.floor((now-start)/cadence)+1)*cadence;}
function beforeDeadline(validated,deadline){return Number.isFinite(validated)&&validated<deadline;}
function qualifies(lane,model,dirty,prior,head){return lane?.git_status?.status_observed===true&&lane.git_status.dirty===dirty&&lane.git_status.changed_file_count===(dirty?1:0)&&lane.head===head&&Number.isSafeInteger(model.observations?.git_cycle)&&model.observations.git_cycle>prior;}
function statistics(trials,expected,clients=4){
  const completed=trials.filter(t=>t.status==='complete');
  const rows=client=>completed.map(t=>({elapsed_ms:t.clients[client].elapsed_ms}));
  const per_client=Array.from({length:clients},(_,i)=>runtime.nearestRank(rows(i)));
  const maxima=runtime.nearestRank(completed.map(t=>({elapsed_ms:Math.max(...t.clients.map(c=>c.elapsed_ms))})));
  const full=trials.length===expected&&completed.length===expected&&trials.every(t=>t.clients.length===clients&&t.clients.every(c=>c.status==='converged'&&!c.error&&Number.isFinite(c.elapsed_ms)&&c.elapsed_ms>=0));
  return {attempted:trials.filter(t=>t.status!=='not-executed').length,completed:completed.length,not_executed:trials.filter(t=>t.status==='not-executed').length,per_client,maxima,full_population:full,
    latency_gate:full&&per_client.every(p=>p.p95_ms<=2000)&&maxima.p95_ms<=2000};
}
async function readProbe(client,ctx,phase,index,probeId,deadline){
  const bounded=Object.create(ctx);bounded.phaseDeadline=deadline;
  const first=await runtime.required(client,phase,index,bounded);
  const envelope=runtime.envelope(first.model), seen=new Set(),tokens=new Set();
  let model=first.model, part=model.pages.workspaces, pages=0;
  for(;;){
    assert(performance.now()<deadline,'Trial deadline during workspace traversal');
    assert.equal(part.total,ctx.receipt.dimensions.worktrees);assert.equal(part.included,part.items.length);
    assert.equal(part.incomplete,Boolean(part.next_cursor));
    let found=null;
    for(const item of part.items){assert(!seen.has(item.worktree_id),'Duplicate workspace page item');seen.add(item.worktree_id);if(item.worktree_id===probeId)found=item;}
    if(found)return {model:first.model,lane:found,validated_ms:performance.now(),pages:pages+1};
    assert(part.next_cursor,'Probe workspace absent from captured summary');assert(!tokens.has(part.next_cursor),'Workspace cursor cycle');tokens.add(part.next_cursor);assert(++pages<=4096);
    const next=await runtime.required(client,phase,index,bounded,part.next_cursor);
    assert.deepEqual(runtime.envelope(next.model),envelope);model=next.model;part=model.pages.workspaces;
  }
}
function gitBytes(root,args){return execFileSync('git',['-C',root,...args],{timeout:10000,maxBuffer:4*1024*1024,windowsHide:true,env:{...process.env,GIT_TERMINAL_PROMPT:'0',GIT_NO_LAZY_FETCH:'1',GIT_NO_REPLACE_OBJECTS:'1',GIT_OPTIONAL_LOCKS:'0'}});}
function gitEvidence(receipt){return receipt.worktrees.map(w=>({id:w.worktree_id,head:runtime.hash(gitBytes(w.root,['rev-parse','HEAD'])),refs:runtime.hash(gitBytes(w.root,['for-each-ref','--format=%(refname):%(objectname)'])),index:runtime.hash(gitBytes(w.root,['ls-files','--stage'])),config:runtime.hash(gitBytes(w.root,['config','--null','--list','--show-origin']))}));}
function prepareProbe(ctx){
  const probe=ctx.receipt.change_probe;assert(probe,'Owned change probe required');
  const saved=runtime.checked(probe.path,'file');assert(runtime.within(saved.path,ctx.receipt.allocation_root));
  const worktree=ctx.receipt.worktrees.find(w=>w.worktree_id===probe.worktree_id);assert(worktree);
  assert(runtime.within(saved.path,runtime.checked(worktree.root,'directory').path));
  const original=fs.readFileSync(probe.path);assert(original.length<=65536);assert.equal(runtime.hash(original),probe.sha256);
  let fd=runtime.openProbe(probe.path,saved);fs.closeSync(fd);
  const relative=path.relative(worktree.root,probe.path).split(path.sep).join('/');
  assert.equal(gitBytes(worktree.root,['ls-files','--error-unmatch','-z','--',relative]).toString('utf8'),relative+'\0');
  const changed=Buffer.concat([original,Buffer.from('\n# devmap-owned-freshness-probe\n')]);
  let expected=original;
  return {probe,original,worktree,rewrite(dirty,restoring=false){
    if(!restoring)assert.equal(runtime.hash(fs.readFileSync(probe.path)),runtime.hash(expected),'Probe modified by another writer');
    const bytes=dirty?changed:original;fd=runtime.openProbe(probe.path,saved);
    try{let offset=0;while(offset<bytes.length){const n=fs.writeSync(fd,bytes,offset,bytes.length-offset,offset);assert(n>0);offset+=n;}
      fs.ftruncateSync(fd,bytes.length);expected=bytes;fs.fsyncSync(fd);
      return {change_completed_monotonic_ms:performance.now(),sha256:runtime.hash(bytes)};
    } finally{fs.closeSync(fd);}
  },verify(){assert.deepEqual(runtime.checked(probe.path,'file'),saved);assert.equal(runtime.hash(fs.readFileSync(probe.path)),probe.sha256);}};
}
async function changeTrial(clients,ctx,owner,probe,head,phase,index,dirty,overall){
  ctx.health();assert(performance.now()<overall,'Overall freshness deadline');ctx.verifyFixture();await runtime.sameOwner(ctx,owner);
  const prior=clients.map(c=>c.previousCycle);assert(prior.every(Number.isSafeInteger));
  const row={index,phase,dirty,status:'running',mutation_started_monotonic_ms:performance.now(),clients:clients.map((c,i)=>({client:c.entry.label,prior_cycle:prior[i],status:'pending',polls:[]}))};
  if(phase==='measured')ctx.report.trials[index]=row;else ctx.report.warmups.push(row);
  try{
    Object.assign(row,probe.rewrite(dirty));const start=row.change_completed_monotonic_ms, deadline=Math.min(start+ctx.report.protocol.trial_ms,overall);
    let round=0,scheduled=start;
    while(row.clients.some(c=>c.status==='pending')){
      assert(performance.now()<deadline,'Change convergence deadline');assert(round<151,'Polling opportunity bound');ctx.health();
      const active=row.clients.map((c,i)=>({c,i})).filter(v=>v.c.status==='pending');
      const dispatched=performance.now();
      const results=await Promise.allSettled(active.map(({i})=>readProbe(clients[i],ctx,`${phase}-${index}-poll`,round,probe.probe.worktree_id,deadline)));
      let failed=false;
      for(let n=0;n<results.length;n++){
        const {c,i}=active[n], result=results[n];
        const poll={round,scheduled_monotonic_ms:scheduled,dispatched_monotonic_ms:dispatched};
        if(result.status==='rejected'){c.error=String(result.reason.stack||result.reason);c.status='error';poll.error=c.error;failed=true;}
        else {const data=result.value;poll.validated_monotonic_ms=data.validated_ms;poll.observations=data.model.observations;poll.git_status=data.lane.git_status;poll.pages=data.pages;
          if(!beforeDeadline(data.validated_ms,deadline)){c.error='Response validated after trial deadline';c.status='timeout';failed=true;}
          else if(qualifies(data.lane,data.model,dirty,prior[i],head)){c.status='converged';c.elapsed_ms=data.validated_ms-start;c.observations=data.model.observations;}
        }c.polls.push(poll);
      }
      if(failed)throw new Error('A measured client failed; remaining trial samples are not replaced');
      if(row.clients.every(c=>c.status==='converged'))break;
      const next=nextTick(start,performance.now(),ctx.report.protocol.cadence_ms);
      row.skipped_ticks=(row.skipped_ticks||0)+Math.max(0,Math.round((next-scheduled)/100)-1);scheduled=next;round++;
      await delay(Math.max(0,Math.min(next,deadline)-performance.now()));
    }
    row.status='complete';row.elapsed_ms=Math.max(...row.clients.map(c=>c.elapsed_ms));await runtime.sameOwner(ctx,owner);
  }catch(error){row.status='failed';row.error=String(error.stack||error);for(const c of row.clients)if(c.status==='pending'){c.status='not-converged';c.error=row.error;}throw error;}
  finally{ctx.append('trials.ndjson',JSON.stringify(row)+'\n');ctx.save();}
}
async function main(configPath){
  const config=JSON.parse(fs.readFileSync(configPath,'utf8')), protocol=policy(config.mode);
  const ctx=runtime.createRun(config,[__filename,require.resolve('./shared-summary-runtime.cjs'),require.resolve('./shared-summary-performance.cjs')],protocol);
  const overall=performance.now()+protocol.overall_ms;ctx.phaseDeadline=overall;let probe,gitBefore;
  try{
    ctx.prepare();probe=prepareProbe(ctx);gitBefore=gitEvidence(ctx.receipt);
    const owner=await runtime.startOwner(ctx,'freshness-owner'),clients=[];
    for(let i=0;i<protocol.clients;i++){const client=runtime.proxy(ctx.exe,ctx.receipt.source,`freshness-proxy-${i}`,ctx);clients.push(client);await runtime.initialize(client);}
    let head;
    for(const client of clients){const data=await runtime.audit(client,ctx,`baseline-${client.entry.label}`);const lane=data.all.workspaces.find(w=>w.worktree_id===probe.probe.worktree_id);
      assert(lane?.git_status.status_observed&&!lane.git_status.dirty&&lane.git_status.changed_file_count===0,'Probe must begin clean');head??=lane.head;assert.equal(lane.head,head);}
    for(let i=0;i<protocol.warmups;i++)await changeTrial(clients,ctx,owner,probe,head,'warmup',i,i%2===0,overall);
    probe.verify();ctx.preserve();
    for(let i=0;i<protocol.measured;i++)await changeTrial(clients,ctx,owner,probe,head,'measured',i,i%2===0,overall);
    probe.verify();ctx.preserve();await runtime.sameOwner(ctx,owner);
    for(const client of clients)await runtime.audit(client,ctx,`final-${client.entry.label}`);
  }catch(error){ctx.report.errors.push(String(error.stack||error));}
  finally{
    try{if(probe){probe.rewrite(false,true);probe.verify();ctx.report.probe_restored=true;
      const status=gitBytes(probe.worktree.root,['status','--porcelain=v1','-z','--untracked-files=normal']);assert.equal(status.length,0,'Final probe worktree not clean');}}
    catch(error){ctx.report.errors.push(`Restoration: ${error.stack||error}`);}
    const cleanup=await Promise.allSettled(ctx.children.map(runtime.stop));
    ctx.report.cleanup_errors=cleanup.filter(r=>r.status==='rejected').map(r=>String(r.reason));
    ctx.report.child_lifecycle=ctx.children.map(c=>({label:c.label,pid:c.child.pid,exit:c.exit,spawn_error:c.spawn_error}));
    try{ctx.preserve();if(gitBefore)assert.deepEqual(gitEvidence(ctx.receipt),gitBefore);ctx.report.final_preservation=true;}
    catch(error){ctx.report.errors.push(`Preservation: ${error.stack||error}`);}
    if(ctx.asyncError)ctx.report.errors.push(`Asynchronous failure: ${ctx.asyncError.stack||ctx.asyncError}`);
    ctx.report.sqlite_sidecars_after=ctx.sidecars();ctx.report.statistics=statistics(ctx.report.trials,protocol.measured);
    ctx.report.request_failures=ctx.rows.filter(r=>r.error);
    if(config.mode==='acceptance'&&ctx.report.statistics.full_population&&!ctx.report.statistics.latency_gate)ctx.report.errors.push('Freshness p95 gate failed');
    ctx.report.completed=ctx.report.errors.length===0&&ctx.report.cleanup_errors.length===0&&ctx.report.statistics.full_population;
    ctx.report.freshness_acceptance=ctx.report.completed&&config.mode==='acceptance'&&ctx.report.statistics.latency_gate;
    ctx.report.performance_acceptance=false;ctx.report.finished_at=new Date().toISOString();ctx.closeLogs();ctx.save();
  }
  assert(ctx.report.completed,`Freshness failed; retained report: ${ctx.run}`);
  assert(config.mode!=='acceptance'||ctx.report.freshness_acceptance,`Freshness p95 gate failed; retained report: ${ctx.run}`);
  console.log(JSON.stringify({run:ctx.run,completed:true,freshness_acceptance:ctx.report.freshness_acceptance,performance_acceptance:false}));
}
function selfTest(){
  assert.throws(()=>runtime.validateDimensions('acceptance',{worktrees:1,sessions:1,events:2}),/fixed 20/);
  assert.throws(()=>runtime.validateDimensions('acceptance',{worktrees:20,sessions:100,events:99999}),/fixed 20/);
  runtime.validateDimensions('acceptance',{worktrees:20,sessions:100,events:100000});
  runtime.validateDimensions('smoke',{worktrees:1,sessions:1,events:2});
  assert.deepEqual([policy('acceptance').warmups,policy('acceptance').measured,policy('acceptance').clients],[10,100,4]);
  assert(!beforeDeadline(15000,15000));assert(beforeDeadline(14999,15000));
  assert.equal(nextTick(1000,1000),1100);assert.equal(nextTick(1000,1251),1300);
  const lane={head:'h',git_status:{status_observed:true,dirty:true,changed_file_count:1}};
  assert(qualifies(lane,{observations:{git_cycle:2}},true,1,'h'));
  assert(!qualifies(lane,{observations:{git_cycle:1}},true,1,'h'));
  assert(!qualifies(lane,{observations:{git_cycle:2}},false,1,'h'));
  const trial=()=>({status:'complete',clients:Array.from({length:4},()=>({status:'converged',elapsed_ms:100}))});
  let trials=Array.from({length:100},trial);assert(statistics(trials,100).latency_gate);
  trials[0].status='failed';assert(!statistics(trials,100).latency_gate);assert(!statistics([],100).latency_gate);
  trials=Array.from({length:100},trial);for(let i=0;i<8;i++)trials[i].clients[i%4].elapsed_ms=2200;
  const stats=statistics(trials,100);assert(stats.per_client.every(s=>s.p95_ms===100));assert.equal(stats.maxima.p95_ms,2200);assert(!stats.latency_gate);
  console.log('Pure freshness policy/predicate/population/max-p95 checks passed; no filesystem/process/corpus opened.');
}
if(require.main===module){if(process.argv[2]==='--self-test')selfTest();else if(process.argv[2]==='--config'&&process.argv[3])main(path.resolve(process.argv[3])).catch(error=>{console.error(error);process.exitCode=1;});else console.log('Usage: node shared-summary-freshness.cjs --self-test | --config <owned-config.json>');}
module.exports={policy,nextTick,beforeDeadline,qualifies,statistics};
