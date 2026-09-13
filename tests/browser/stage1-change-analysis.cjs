'use strict';
const fs=require('node:fs'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const {intervals,METHOD}=require('./stage1-baseline-analysis.cjs');
function analyze(report){
 assert.equal(report.schema,'devmap/stage1-change-calibration/1');assert.equal(report.completed,true);assert.equal(report.preserved,true);assert.deepEqual(report.errors,[]);assert.equal(report.arms.length,2);
 const populations=report.arms.map((arm,i)=>{assert.equal(arm.label,['A1','A2'][i]);assert.equal(arm.code,0);assert.equal(arm.expired,false);assert.equal(arm.job.empty_confirmed,true);assert.equal(arm.job.root_exit_code,0);assert.equal(arm.job.aborted,false);for(const key of ['error','cleanup_error','descendants_after_root_exit'])assert(!arm.job[key]);
  const r=arm.result;assert.equal(r.mode,'calibration');assert.equal(r.completed,true);assert.equal(r.data_preserved,true);assert.equal(r.injected_failure_at,null);assert.deepEqual(r.errors,[]);assert.equal(r.protocol.clients,4);assert.equal(r.protocol.warmups,10);assert.equal(r.protocol.measured,100);assert.equal(r.trials.length,110);
  for(const [n,t] of r.trials.entries()){assert.equal(t.index,n);assert.equal(t.phase,n<10?'warmup':'measured');assert.equal(t.status,'complete');assert.equal(t.clients.length,4);assert(t.clients.every(c=>c.status==='converged'&&c.model_verified===true));}
  const rows=r.trials.filter(t=>t.phase==='measured');assert.equal(rows.length,100);return Array.from({length:4},(_,c)=>rows.map((t,n)=>{assert.equal(t.index,n+10);assert.equal(t.dirty,n%2===0);const row=t.clients[c];assert.equal(row.client,c);assert(Number.isFinite(row.elapsed_ms)&&row.elapsed_ms>=0);return row.elapsed_ms;}));
 });
 const [a,b]=report.arms.map(a=>a.result);assert.equal(a.baseline_sha256,'a1cfbb1c46bd9026b18db67da9f70485b0bb20c6c54fd779475b52531731d419');assert.equal(a.baseline_sha256,b.baseline_sha256);assert.equal(a.baseline_model_sha256,b.baseline_model_sha256);assert.equal(a.scale_inventory_sha256,b.scale_inventory_sha256);assert.equal(a.source,b.source);
 const result=intervals(populations[0],populations[1],10,250);
 const maxima=populations.map(p=>[p[0].map((_,i)=>Math.max(...p.map(c=>c[i])))]);
 const maximum=intervals(maxima[0],maxima[1],10,250)[0];
 return {scope:'old-only full-map scale change A/A noise diagnostic',method:{seed:METHOD.seed,replicates:METHOD.replicates,interval:METHOD.interval,block_trials:10,paired_four_clients:true},per_client:result,per_trial_maximum:maximum,all_noise_intervals_within_maximum_caps:[...result,maximum].every(r=>r.noise_within_maximum_cap),candidate_acceptance:false,tolerance_frozen:false};
}
if(require.main===module){const bytes=fs.readFileSync(process.argv[2]),result={...analyze(JSON.parse(bytes)),source_sha256:crypto.createHash('sha256').update(bytes).digest('hex')};fs.writeFileSync(process.argv[3],JSON.stringify(result,null,2),{flag:'wx'});console.log(JSON.stringify(result));}
module.exports={analyze};
