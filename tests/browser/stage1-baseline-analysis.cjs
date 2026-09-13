'use strict';
// Diagnostic old-only A/A intervals. Does not freeze candidate tolerances.
const assert=require('node:assert/strict'),fs=require('node:fs'),crypto=require('node:crypto');
const METHOD={seed:9132026,replicates:5000,interval:0.95,cold_block_size:5,warm_block_size:10,scope:'paired ordered blocks; four warm clients resampled together; only two old-only arms'};
function percentile(values,p){assert(values.length);const sorted=[...values].sort((a,b)=>a-b);return sorted[Math.ceil(sorted.length*p)-1];}
function rng(seed){let x=seed>>>0;return ()=>{x^=x<<13;x^=x>>>17;x^=x<<5;return (x>>>0)/4294967296;};}
function intervals(left,right,size,cap,seed=METHOD.seed){
 assert.equal(left.length,right.length);assert(left.length>0);
 const count=left[0].length;assert(count>0&&count%size===0);
 for(const rows of [...left,...right]){assert.equal(rows.length,count);assert(rows.every(x=>Number.isFinite(x)&&x>=0));}
 const random=rng(seed),deltas=left.map(()=>[]),blocks=count/size;
 for(let n=0;n<METHOD.replicates;n++){
  const indices=[];for(let b=0;b<blocks;b++){const start=Math.floor(random()*blocks)*size;for(let i=0;i<size;i++)indices.push(start+i);}
  for(let c=0;c<left.length;c++)deltas[c].push(percentile(indices.map(i=>right[c][i]),0.95)-percentile(indices.map(i=>left[c][i]),0.95));
 }
 return left.map((a,c)=>{
  const b=right[c],p95a=percentile(a,0.95),p95b=percentile(b,0.95),limit=Math.min(p95a*0.1,cap);
  const low=percentile(deltas[c],0.025),high=percentile(deltas[c],0.975);
  return {client:c,samples_per_arm:count,A1:{p50_ms:percentile(a,0.5),p95_ms:p95a},A2:{p50_ms:percentile(b,0.5),p95_ms:p95b},delta_p95_ms:p95b-p95a,paired_block_95_interval_ms:[low,high],maximum_engineering_allowance_ms:limit,noise_within_maximum_cap:low>=-limit&&high<=limit};
 });
}
function analyze(report){
 assert.equal(report.schema,'devmap/stage1-baseline-calibration/1');assert.equal(report.completed,true);assert.equal(report.preserved,true);assert.deepEqual(report.errors,[]);assert.equal(report.arms.length,2);
 const [a,b]=report.arms.map((arm,i)=>{assert.equal(arm.label,['A1','A2'][i]);assert.equal(arm.code,0);assert.equal(arm.job_result.empty_confirmed,true);assert.equal(arm.job_result.root_exit_code,0);assert.equal(arm.result.cold.count,20);assert.equal(arm.result.cold.samples.length,20);assert.equal(arm.result.cohorts.length,4);return arm.result;});
 assert.equal(a.executable_sha256,b.executable_sha256);assert.equal(a.model_audit.sha256,b.model_audit.sha256);
 const hot=result=>result.cohorts.map((cohort,c)=>{assert.equal(cohort.client,c);assert.equal(cohort.samples.length,100);assert.deepEqual(cohort.errors,[]);return cohort.samples.map((row,i)=>{assert.equal(row.client,c);assert.equal(row.sequence,i);return row.ms;});});
 const cold=intervals([a.cold.samples.map(r=>r.ms)],[b.cold.samples.map(r=>r.ms)],METHOD.cold_block_size,250);
 const warm=intervals(hot(a),hot(b),METHOD.warm_block_size,100);
 return {scope:'cold/warm old-only A/A diagnostic',method:METHOD,cold,warm,all_noise_intervals_within_maximum_caps:[...cold,...warm].every(x=>x.noise_within_maximum_cap),candidate_acceptance:false,tolerance_frozen:false,limitations:['Only two sequential old-only arms; block intervals do not prove future noise is bounded.','No browser-feedback or change-visibility calibration.','The maximum caps are preregistered engineering budgets, not proof of imperceptibility.','If intervals exceed caps, improve the environment; do not widen caps from results.']};
}
if(require.main===module){const bytes=fs.readFileSync(process.argv[2]);const result={...analyze(JSON.parse(bytes)),source_sha256:crypto.createHash('sha256').update(bytes).digest('hex')};fs.writeFileSync(process.argv[3],JSON.stringify(result,null,2),{flag:'wx'});console.log(JSON.stringify(result));}
module.exports={analyze,intervals,METHOD};
