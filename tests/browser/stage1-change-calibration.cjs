'use strict';
const fs=require('node:fs'),path=require('node:path'),os=require('node:os'),assert=require('node:assert/strict'),{spawn}=require('node:child_process');
const h=require('./shared-summary-performance.cjs'),{sample}=require('./host-load-sampler.cjs');
const root=path.resolve(__dirname,'../..'),allowed=path.join(root,'target/verification'),hash=p=>h.runtime.hash(fs.readFileSync(p));
const read=p=>JSON.parse(fs.readFileSync(p,'utf8').replace(/^\uFEFF/,''));
const own=(p,kind='file')=>{const item=h.checked(p,kind);assert(h.within(item.path,allowed));return item;};
async function main(configPath){
 const configIdentity=own(path.resolve(configPath)),config=read(configIdentity.path);assert.equal(config.schema,'devmap/stage1-change-calibration/1');
 const receiptPath=own(config.receipt),receipt=read(receiptPath.path);assert.equal(receipt.completed,true);assert.deepEqual(receipt.dimensions,{worktrees:20,sessions:100,events:100000});
 const exe=own(config.baseline),python=h.checked(config.python,'file');assert.equal(hash(exe.path),'a1cfbb1c46bd9026b18db67da9f70485b0bb20c6c54fd779475b52531731d419');
 const source=own(receipt.source.path,'directory'),probe=own(receipt.probe.path),inventory=read(own(receipt.inventory).path);assert(h.within(probe.path,source.path));
 const roots=receipt.worktrees.map(w=>own(path.join(w.git_dir.path,'devmap'),'directory').path);
 const worker=path.join(__dirname,'full-map-change-preflight.cjs'),wrapper=path.join(__dirname,'windows-owned-generator-job.py');
 const files=[configIdentity.path,receiptPath.path,receipt.inventory,receipt.manifest,exe.path,python.path,process.execPath,__filename,worker,wrapper,...['full-map-change-model.cjs','full-map-fingerprint.cjs','shared-summary-performance.cjs','host-load-sampler.cjs','stage1-change-analysis.cjs','stage1-baseline-analysis.cjs'].map(n=>path.join(__dirname,n))];
 const inputs=files.map(p=>({identity:h.checked(p,'file'),sha256:hash(p)}));
 const run=fs.mkdtempSync(path.join(allowed,'stage1-change-aa-')),report={schema:config.schema,run,inputs,host:{platform:os.platform(),release:os.release(),cpu:os.cpus()[0]?.model,logical_cpus:os.cpus().length,total_memory:os.totalmem(),node:process.version},arms:['A1','A2'].map(label=>({label,status:'not-executed'})),host_load:[],errors:[],completed:false,candidate_acceptance:false};
 function preserve(){for(const input of inputs){assert.deepEqual(h.checked(input.identity.path,'file'),input.identity);assert.equal(hash(input.identity.path),input.sha256);}assert.deepEqual(h.checked(source.path,'directory'),receipt.source);assert.equal(hash(probe.path),receipt.probe.sha256);assert.deepEqual(h.inventory(roots),inventory);assert(!fs.existsSync(path.join(source.path,'.git/devmap/devmap.db')));}
 const save=()=>{fs.writeFileSync(path.join(run,'report.next'),JSON.stringify(report,null,2),{flag:'wx'});fs.renameSync(path.join(run,'report.next'),path.join(run,'report.json'));};
 console.log(JSON.stringify({run}));save();let timer;
 try{
  preserve();report.host_load.push(sample());timer=setInterval(()=>{try{report.host_load.push(sample(report.host_load.at(-1)));}catch(e){report.errors.push(String(e));}},5000);
  await new Promise(resolve=>setTimeout(resolve,30000));report.pre_run_host_samples=report.host_load.length;
  for(const arm of report.arms){
   const {label}=arm;preserve();const armDir=path.join(run,label);fs.mkdirSync(armDir);Object.assign(arm,{status:'running',started_at:new Date().toISOString(),worker:armDir});save();
   const out=fs.openSync(path.join(run,label+'.stdout'),'wx'),err=fs.openSync(path.join(run,label+'.stderr'),'wx'),jobPath=path.join(run,label+'.job.json');let child;
   const env={...process.env,DEVMAP_CHANGE_MODE:'calibration',DEVMAP_CHANGE_RUN:armDir,DEVMAP_CHANGE_SCALE_RECEIPT:receiptPath.path,DEVMAP_BASELINE_EXE:exe.path};delete env.DEVMAP_CHANGE_FAIL_AT;
   try{child=spawn(python.path,[wrapper,'--report',jobPath,'--exe',process.execPath,'--',worker],{cwd:root,env,windowsHide:true,stdio:['pipe',out,err]});}finally{fs.closeSync(out);fs.closeSync(err);}
   arm.expired=false;arm.code=await new Promise((resolve,reject)=>{const bound=setTimeout(()=>{arm.expired=true;child.stdin.end('abort');},45*60*1000);child.stdin.on('error',()=>{});child.once('error',e=>{clearTimeout(bound);reject(e);});child.once('close',code=>{clearTimeout(bound);resolve(code);});});
   arm.ended_at=new Date().toISOString();arm.job=read(jobPath);if(fs.existsSync(path.join(armDir,'report.json')))arm.result=read(path.join(armDir,'report.json'));save();
   assert.equal(arm.code,0);assert.equal(arm.expired,false);assert.equal(arm.job.root_exit_code,0);assert.equal(arm.job.empty_confirmed,true);assert.equal(arm.job.aborted,false);for(const key of ['error','cleanup_error','descendants_after_root_exit'])assert(!arm.job[key]);assert.equal(arm.result.completed,true);assert.equal(arm.result.data_preserved,true);assert.deepEqual(arm.result.population,{expected:100,attempted:100,completed:100,failed:0,not_executed:0,full_population:true});preserve();arm.status='complete';save();console.log(JSON.stringify({arm:label,completed:true}));
  }
  report.completed=report.errors.length===0;
 }catch(e){report.errors.push(String(e.stack||e));for(const arm of report.arms)if(arm.status==='running')arm.status='failed';}
 finally{clearInterval(timer);try{preserve();report.preserved=true;}catch(e){report.errors.push(`Preservation: ${e.stack||e}`);report.completed=false;}save();}
 assert(report.completed,JSON.stringify(report.errors));console.log(JSON.stringify({run,completed:true}));
}
main(process.argv[2]).catch(e=>{console.error(e);process.exitCode=1;});
