'use strict';
// Old-only cold/warm A/A calibration. No candidate launch or migration.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),crypto=require('node:crypto'),os=require('node:os');
const {spawn}=require('node:child_process');
const h=require('./shared-summary-performance.cjs');
const root=path.resolve(__dirname,'../..'),allowed=h.checked(path.join(root,'target/verification'),'directory').path;
const sha=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const read=p=>JSON.parse(fs.readFileSync(p,'utf8').replace(/^\uFEFF/,''));
const own=(p,kind)=>{const item=h.checked(p,kind);assert(h.within(item.path,allowed));return item;};
async function main(file){
 assert.equal(process.platform,'win32');
 const configPath=own(path.resolve(file),'file'),config=read(configPath.path);
 assert.equal(config.schema,'devmap/stage1-baseline-calibration/1');
 const receiptPath=own(config.finalized,'file'),inventoryPath=own(config.inventory,'file');
 const receipt=read(receiptPath.path),inventory=read(inventoryPath.path);
 assert.equal(receipt.completed,true);assert.equal(receipt.database_absent,true);
 assert.deepEqual(receipt.dimensions,{worktrees:20,sessions:100,events:100000});
 assert.equal(inventory.sha256,receipt.legacy_inventory_sha256);
 const source=own(receipt.source.path,'directory');assert.deepEqual(source,receipt.source);
 const roots=[...new Set(inventory.inventory.map(e=>own(e.root,'directory').path))];
 assert.equal(roots.length,20);
 const baseline=own(config.baseline,'file'),python=h.checked(config.python,'file');
 assert.equal(sha(baseline.path),'a1cfbb1c46bd9026b18db67da9f70485b0bb20c6c54fd779475b52531731d419');
 const files=[baseline.path,python.path,process.execPath,configPath.path,receiptPath.path,inventoryPath.path,__filename,path.join(__dirname,'process-performance.cjs'),path.join(__dirname,'full-map-fingerprint.cjs'),path.join(__dirname,'shared-summary-performance.cjs'),path.join(__dirname,'windows-owned-generator-job.py')];
 const inputs=files.map(p=>({identity:h.checked(p,'file'),sha256:sha(p)}));
 const run=fs.mkdtempSync(path.join(allowed,'stage1-baseline-aa-')),runIdentity=h.checked(run,'directory');
 const report={schema:config.schema,run,formal_candidate_acceptance:false,scope:'two old-only cold/warm arms; browser and change calibration remain separate',schedule:{arms:['A1','A2'],cold_per_arm:20,clients:4,warmup_per_client:10,warm_per_client:100},inputs,fixture:receipt,host:{platform:process.platform,release:os.release(),cpu:os.cpus()[0]?.model,logical_cpus:os.cpus().length,total_memory:os.totalmem(),free_memory_at_start:os.freemem(),node:process.version},arms:[],errors:[],completed:false};
 function preserve(){
  assert.deepEqual(h.checked(run,'directory'),runIdentity);assert.deepEqual(h.checked(source.path,'directory'),source);
  for(const input of inputs){assert.deepEqual(h.checked(input.identity.path,'file'),input.identity);assert.equal(sha(input.identity.path),input.sha256);}
  assert.deepEqual(h.inventory(roots),inventory);assert(!fs.existsSync(path.join(source.path,'.git/devmap/devmap.db')));
 }
 function save(){fs.writeFileSync(path.join(run,'report.next'),JSON.stringify(report,null,2),{flag:'wx'});fs.renameSync(path.join(run,'report.next'),path.join(run,'report.json'));}
 console.log(JSON.stringify({run,schedule:report.schedule}));save();
 try{
  preserve();let expectedModel;
  for(const label of report.schedule.arms){
   preserve();const output=path.join(run,`${label}.worker.json`),jobPath=path.join(run,`${label}.job.json`);
   const out=fs.openSync(path.join(run,`${label}.stdout`),'wx'),err=fs.openSync(path.join(run,`${label}.stderr`),'wx');
   const env={...process.env,DEVMAP_BENCHMARK_EXE:baseline.path,DEVMAP_BENCHMARK_SOURCE:source.path,DEVMAP_BENCHMARK_OUTPUT:output,DEVMAP_BENCHMARK_COLD:'20',DEVMAP_BENCHMARK_CLIENTS:'4',DEVMAP_BENCHMARK_WARMUP:'10',DEVMAP_BENCHMARK_SAMPLES:'100',DEVMAP_BENCHMARK_VERIFY_MODEL:'1',DEVMAP_BENCHMARK_MODEL_MODE:'legacy-refresh',DEVMAP_BENCHMARK_PROGRESS:'1'};
   delete env.DEVMAP_BENCHMARK_MODEL_SHA256;if(expectedModel)env.DEVMAP_BENCHMARK_MODEL_SHA256=expectedModel;
   const arm={label,started_at:new Date().toISOString(),worker:output,job:jobPath};report.arms.push(arm);save();
   let child;
   try{child=spawn(python.path,[path.join(__dirname,'windows-owned-generator-job.py'),'--report',jobPath,'--exe',process.execPath,'--',path.join(__dirname,'process-performance.cjs')],{cwd:root,env,windowsHide:true,stdio:['pipe',out,err]});}
   finally{fs.closeSync(out);fs.closeSync(err);}
   let expired=false;
   arm.code=await new Promise((resolve,reject)=>{const timer=setTimeout(()=>{expired=true;child.stdin.end('abort');},30*60*1000);child.stdin.on('error',()=>{});child.once('error',e=>{clearTimeout(timer);reject(e);});child.once('close',code=>{clearTimeout(timer);resolve(code);});});
   arm.expired=expired;arm.ended_at=new Date().toISOString();arm.job_result=read(jobPath);if(fs.existsSync(output))arm.result=read(output);save();
   preserve();assert(!expired);assert.equal(arm.code,0);assert.equal(arm.job_result.root_exit_code,0);assert.equal(arm.job_result.empty_confirmed,true);assert.equal(arm.job_result.aborted,false);assert(!arm.job_result.error&&!arm.job_result.cleanup_error&&!arm.job_result.descendants_after_root_exit);
   assert.equal(arm.result.cold.count,20);assert.equal(arm.result.clients,4);assert.equal(arm.result.cohorts.length,4);
   for(const cohort of arm.result.cohorts){assert.equal(cohort.summary.count,100);assert.deepEqual(cohort.errors,[]);}
   assert.equal(arm.result.model_audit.enabled,true);expectedModel??=arm.result.model_audit.sha256;assert.equal(arm.result.model_audit.sha256,expectedModel);
   console.log(JSON.stringify({arm:label,completed:true,cold:arm.result.cold.p95_ms,warm_by_client:arm.result.cohorts.map(c=>c.summary.p95_ms)}));
  }
  report.completed=true;
 }catch(e){report.errors.push(String(e.stack||e));}
 finally{try{preserve();report.preserved=true;}catch(e){report.errors.push(String(e));report.completed=false;}save();}
 console.log(JSON.stringify({run,completed:report.completed,errors:report.errors}));process.exitCode=report.completed?0:1;
}
if(require.main===module)main(process.argv[2]).catch(e=>{console.error(e);process.exitCode=1;});
