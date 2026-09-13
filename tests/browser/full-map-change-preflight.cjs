'use strict';
// Frozen-old public full-map change probe. Small tooling preflight, not calibration.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const {execFileSync,spawn}=require('node:child_process'),{setTimeout:delay}=require('node:timers/promises');
const h=require('./shared-summary-performance.cjs'),{fingerprint}=require('./full-map-fingerprint.cjs');
const {auditChangeModel,population}=require('./full-map-change-model.cjs');
function qualifies(model,id,head,dirty,prior){
 const facts=model.workspace_facts?.find(f=>f.worktree_id===id);
 return facts?.head_oid===head&&facts.working_state===(dirty?'dirty':'clean')&&
  typeof model.generated_at==='string'&&facts.git_observed_at===model.generated_at&&Number.isSafeInteger(model.observation_revision)&&model.observation_revision>prior;
}
async function main(){
 const allowed=path.resolve(__dirname,'../../target/verification'),run=h.checked(process.env.DEVMAP_CHANGE_RUN,'directory').path;
 assert(h.within(run,allowed));assert.equal(fs.readdirSync(run).length,0,'Fresh empty owned output required');
 const exe=h.checked(process.env.DEVMAP_BASELINE_EXE,'file').path;assert(h.within(exe,allowed));
 const sha=h.runtime.hash(fs.readFileSync(exe));assert.equal(sha,'a1cfbb1c46bd9026b18db67da9f70485b0bb20c6c54fd779475b52531731d419');
 const source=path.join(run,'repository');fs.mkdirSync(source);const probe=path.join(source,'probe.txt'),original=Buffer.from('owned full-map change probe\n');fs.writeFileSync(probe,original,{flag:'wx'});
 const git=(...args)=>execFileSync('git',args,{cwd:source,windowsHide:true,encoding:'utf8',timeout:10000,stdio:'pipe'}).trim();
 for(const args of [['init','-b','main'],['config','user.name','Full map probe'],['config','user.email','fixture@example.invalid'],['add','probe.txt'],['commit','-m','fixture']])git(...args);
 const head=git('rev-parse','HEAD'),probeIdentity=h.checked(probe,'file'),sourceIdentity=h.checked(source,'directory'),runIdentity=h.checked(run,'directory');
 const hook=execFileSync(exe,['hook','handle','--source',source,'--host','codex','--event','SessionStart','--binding-id','devmap/v1/codex/SessionStart'],{cwd:source,input:JSON.stringify({session_id:'public-change-probe',cwd:source,hook_event_name:'SessionStart',source:'startup'}),encoding:'utf8',windowsHide:true,timeout:30000});assert.deepEqual(JSON.parse(hook),{});
 const legacy=h.inventory([path.join(source,'.git/devmap')]);
 const report={scope:'four-client old-only full-map change tooling preflight; not A/A or candidate/absolute acceptance',run,source,baseline_sha256:sha,protocol:{clients:4,warmups:2,measured:4,cadence_ms:100,trial_ms:30000,start:'after probe write and fsync',end:'parsed public full-map response validates changed worktree, unchanged HEAD and newer observation'},trials:[],errors:[],completed:false};
 report.trials=Array.from({length:6},(_,index)=>({index,phase:index<2?'warmup':'measured',dirty:index%2===0,status:'not-executed',clients:Array.from({length:4},(_,client)=>({client,status:'not-executed',polls:[]}))}));
 const failAt=process.env.DEVMAP_CHANGE_FAIL_AT===undefined?null:Number(process.env.DEVMAP_CHANGE_FAIL_AT);assert(failAt===null||(Number.isInteger(failAt)&&failAt>=0&&failAt<6));report.injected_failure_at=failAt;
 const ctx={children:[],verifyRun(){assert.deepEqual(h.checked(run,'directory'),runIdentity);},verifyFixture(){assert.deepEqual(h.checked(source,'directory'),sourceIdentity);},append(name,bytes){fs.appendFileSync(path.join(run,name),bytes);}};
 const clients=[];let expected=original,baselineHash,id,baselineRevision,baselineModels=[];
 function save(){fs.writeFileSync(path.join(run,'report.json'),JSON.stringify(report,null,2));}
 function rewrite(dirty){
  assert.deepEqual(h.checked(probe,'file'),probeIdentity);assert.deepEqual(fs.readFileSync(probe),expected,'Concurrent probe edit');
  const bytes=dirty?Buffer.concat([original,Buffer.from('changed\n')]):original,fd=h.runtime.openProbe(probe,probeIdentity);
  try{let offset=0;while(offset<bytes.length){const n=fs.writeSync(fd,bytes,offset,bytes.length-offset,offset);assert(n>0);offset+=n;}fs.ftruncateSync(fd,bytes.length);fs.fsyncSync(fd);expected=bytes;return performance.now();}finally{fs.closeSync(fd);}
 }
 function normalized(model,revision){assert.equal(model.schema_version,'devmap/dock/4');assert.equal(model.revision,revision,'Expected semantic revision after observed changes');return fingerprint({...model,revision:'@separately-verified-semantic-revision',observation_revision:'@separately-verified-newer'},'fresh-git-observation');}
 async function read(client,label){
  const packet=await client.request('tools/call',{name:'devmap_read_map',arguments:{}},30000);
  assert(!packet.response.error&&!packet.response.result?.isError);const model=packet.response.result?.structuredContent;
  assert.equal(model?.schema_version,'devmap/dock/4');assert(Number.isSafeInteger(model.observation_revision));
  const expectedObservation=client.previousCycle===null?model.observation_revision:client.previousCycle+1;
  assert.equal(model.observation_revision,expectedObservation,'Frozen old per-client refresh counter');
  client.previousCycle=model.observation_revision;
  const parsed=performance.now();return {model,parsed,bytes:packet.wire_bytes,packet,label,expectedObservation};
 }
 const retain=data=>fs.writeFileSync(path.join(run,data.label+'.json'),JSON.stringify(data.packet),{flag:'wx'});
 try{
  save();
  for(let i=0;i<4;i++){const c=h.runtime.proxy(exe,source,`proxy-${i}`,ctx);clients.push(c);await h.runtime.initialize(c);}
  for(let i=0;i<4;i++){
   const data=await read(clients[i],`initial-${i}`),{model}=data;retain(data);baselineModels.push(model);const facts=model.workspace_facts.find(f=>f.identity.is_current_workspace);assert(facts);id??=facts.worktree_id;assert(qualifies(model,id,head,false,0));
   baselineRevision??=model.revision;assert(Number.isSafeInteger(baselineRevision));baselineHash??=normalized(model,baselineRevision);assert.equal(normalized(model,baselineRevision),baselineHash);
  }
  report.baseline_model_sha256=baselineHash;
  for(let trial=0;trial<6;trial++){
   const row=report.trials[trial];row.status='running';row.clients=clients.map((c,i)=>({client:i,prior:c.previousCycle,polls:[],status:'pending'}));
   try{
    if(failAt===trial)throw new Error('Deliberate preflight trial failure');
    const start=rewrite(row.dirty),deadline=start+30000;row.change_completed_ms=start;
    for(let round=0;row.clients.some(c=>c.status==='pending');round++){
     assert(performance.now()<deadline,'Change visibility deadline');assert(round<301);if(ctx.asyncError)throw ctx.asyncError;
     const active=row.clients.filter(c=>c.status==='pending'),dispatch=performance.now();
     const outcomes=await Promise.allSettled(active.map(async c=>{
      const data=await read(clients[c.client],`trial-${trial}-poll-${round}-client-${c.client}`);
      assert.equal(data.model.repository_id,baselineModels[c.client].repository_id);assert.equal(data.model.current_worktree_id,id);
      const valid=qualifies(data.model,id,head,row.dirty,c.prior);if(valid)assert.equal(data.model.revision,baselineRevision+trial+1,'Exactly one semantic change per trial');const validated=performance.now();
      return {...data,valid,validated};
     }));
     let failed=false;
     outcomes.forEach((outcome,i)=>{const c=active[i];if(outcome.status==='rejected'){c.status='error';c.error=String(outcome.reason.stack||outcome.reason);failed=true;return;}
      const data=outcome.value;retain(data);c.polls.push({round,dispatch_ms:dispatch,parsed_ms:data.parsed,validated_ms:data.validated,bytes:data.bytes,revision:data.model.revision,observation_revision:data.model.observation_revision,qualifies:data.valid});
      if(data.valid&&data.validated<deadline){
       c.elapsed_ms=data.validated-start;
       try{c.model_sha256=auditChangeModel(baselineModels[c.client],data.model,{worktreeId:id,dirty:row.dirty,revision:baselineRevision+trial+1,observationRevision:data.expectedObservation});c.model_verified=true;c.status='converged';}
       catch(e){c.status='invalid-model';c.error=String(e.stack||e);failed=true;}
      }else if(data.validated>=deadline){c.status='timeout';failed=true;}
     });
     assert(!failed,'Client failure retained');if(row.clients.every(c=>c.status==='converged'))break;
     const next=start+(Math.floor((performance.now()-start)/100)+1)*100;await delay(Math.max(0,Math.min(next,deadline)-performance.now()));
    }
    row.status='complete';
   }catch(e){row.status='failed';row.error=String(e.stack||e);for(const c of row.clients)if(c.status==='pending')c.status='not-converged';throw e;}finally{save();}
  }
  for(let i=0;i<4;i++){const data=await read(clients[i],`final-${i}`);retain(data);assert.equal(normalized(data.model,baselineRevision+6),baselineHash,'Full clean model preserved after changes');}
 }catch(e){report.errors.push(String(e.stack||e));}
 finally{
  try{rewrite(false);assert.equal(git('status','--porcelain','--untracked-files=normal'),'');assert.equal(git('rev-parse','HEAD'),head);assert.deepEqual(h.inventory([path.join(source,'.git/devmap')]),legacy);assert(!fs.existsSync(path.join(source,'.git/devmap/devmap.db')));report.data_preserved=true;}catch(e){report.errors.push(`Preservation: ${e.stack||e}`);}
  const cleanup=await Promise.allSettled(clients.map(async c=>{c.entry.stopping=true;c.entry.child.stdin.end();let timer;try{await Promise.race([c.entry.closed,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('EOF cleanup timeout')),5000);})]);assert.deepEqual(c.entry.exit,{code:0,signal:null});}finally{clearTimeout(timer);}}));
  report.errors.push(...cleanup.filter(r=>r.status==='rejected').map(r=>String(r.reason)));if(ctx.asyncError)report.errors.push(String(ctx.asyncError));
  report.children=ctx.children.map(c=>({pid:c.child.pid,exit:c.exit}));report.population=population(report.trials);report.completed=report.errors.length===0&&report.population.full_population&&report.trials.every(t=>t.status==='complete');save();
 }
 assert(report.completed,JSON.stringify(report.errors));console.log(JSON.stringify({run,completed:true}));
}
async function owned(){
 const run=fs.mkdtempSync(path.resolve(__dirname,'../../target/verification/full-map-change-'));
 const python=h.checked(process.env.DEVMAP_PYTHON_EXE,'file').path,jobPath=run+'.job.json';
 const child=spawn(python,[path.join(__dirname,'windows-owned-generator-job.py'),'--report',jobPath,'--exe',process.execPath,'--',__filename],{env:{...process.env,DEVMAP_CHANGE_RUN:run},windowsHide:true,stdio:['pipe','inherit','inherit']});
 let expired=false;const code=await new Promise((resolve,reject)=>{const timer=setTimeout(()=>{expired=true;child.stdin.end('abort');},180000);child.stdin.on('error',()=>{});child.once('error',e=>{clearTimeout(timer);reject(e);});child.once('close',c=>{clearTimeout(timer);resolve(c);});});
 const job=JSON.parse(fs.readFileSync(jobPath));assert(!expired);assert.equal(code,0);assert.equal(job.root_exit_code,0);assert.equal(job.empty_confirmed,true);assert.equal(job.aborted,false);assert(!job.descendants_after_root_exit&&!job.error&&!job.cleanup_error);
 console.log(JSON.stringify({run,job:jobPath,strict_job_passed:true}));
}
if(require.main===module)(process.argv[2]==='--owned'?owned():main()).catch(e=>{console.error(e);process.exitCode=1;});
module.exports={qualifies};
