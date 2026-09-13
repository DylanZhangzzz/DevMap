'use strict';
// Explicit new scale allocation. Never alters a registered corpus.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const {spawn,execFileSync}=require('node:child_process'),h=require('./shared-summary-performance.cjs');
const root=path.resolve(__dirname,'../..'),allowed=path.join(root,'target/verification');
const hash=p=>h.runtime.hash(fs.readFileSync(p));
async function main(){
 const exe=h.checked(process.env.DEVMAP_SCALE_GENERATOR_EXE,'file'),python=h.checked(process.env.DEVMAP_PYTHON_EXE,'file'),temp=h.checked(process.env.DEVMAP_SCALE_TEMP,'directory');assert(h.within(exe.path,allowed));
 const sha=hash(exe.path);assert.equal(sha,'8c84e04ccb2669cd85d4fe1dd3a480be16134e201605e1dbeaa6884bf63d3b66');
 const disk=fs.statfsSync(allowed,{bigint:true});assert(disk.bavail*disk.bsize>2n*1024n**3n);
 const existing=new Set(fs.readdirSync(allowed)),run=fs.mkdtempSync(path.join(allowed,'legacy-change-generation-'));
 const jobPath=path.join(run,'job.json'),wrapper=path.join(__dirname,'windows-owned-generator-job.py');
 const report={run,generator:exe,generator_sha256:sha,wrapper_sha256:hash(wrapper),dimensions:{worktrees:20,sessions:100,events:100000},completed:false,performance_acceptance:false};
 console.log(JSON.stringify({run}));
 try{
  const stdout=fs.openSync(path.join(run,'stdout.log'),'wx'),stderr=fs.openSync(path.join(run,'stderr.log'),'wx');let child;
  try{child=spawn(python.path,[wrapper,'--report',jobPath,'--exe',exe.path,'--','--ignored','--exact','generate_disposable_legacy_scale_corpus','--nocapture','--test-threads=1'],{cwd:root,windowsHide:true,stdio:['pipe',stdout,stderr],env:{...process.env,TEMP:temp.path,TMP:temp.path,LOCALAPPDATA:temp.path,XDG_STATE_HOME:temp.path,DEVMAP_SCALE_WORKTREES:'20',DEVMAP_SCALE_SESSIONS:'100',DEVMAP_SCALE_EVENTS:'1000',DEVMAP_SCALE_TRACKED_PROBE:'1'}});}finally{fs.closeSync(stdout);fs.closeSync(stderr);}
  let expired=false;report.code=await new Promise((resolve,reject)=>{const timer=setTimeout(()=>{expired=true;child.stdin.end('abort');},10*60*1000);child.stdin.on('error',()=>{});child.once('error',e=>{clearTimeout(timer);reject(e);});child.once('close',code=>{clearTimeout(timer);resolve(code);});});
  report.expired=expired;report.job=JSON.parse(fs.readFileSync(jobPath));assert(!expired);assert.equal(report.code,0);assert.equal(report.job.root_exit_code,0);assert.equal(report.job.empty_confirmed,true);assert.equal(report.job.aborted,false);for(const key of ['error','cleanup_error','descendants_after_root_exit'])assert(!report.job[key]);
  assert.deepEqual(h.checked(exe.path,'file'),exe);assert.equal(hash(exe.path),sha);
  const matches=[...fs.readFileSync(path.join(run,'stdout.log'),'utf8').matchAll(/SCALE_FIXTURE ([^\r\n]+)/g)];assert.equal(matches.length,1);
  const manifestPath=h.checked(matches[0][1].trim(),'file').path,allocation=h.checked(path.dirname(manifestPath),'directory');
  assert(h.within(allocation.path,allowed));assert(!existing.has(path.basename(allocation.path)));
  const manifest=JSON.parse(fs.readFileSync(manifestPath));assert.equal(manifest.scope,'synthetic_legacy_scale_fixture');assert.equal(manifest.worktrees.length,20);assert.equal(manifest.sessions,100);assert.equal(manifest.events,100000);
  const source=h.checked(manifest.source,'directory');assert(h.within(source.path,allocation.path));assert(!fs.existsSync(path.join(source.path,'.git/devmap/devmap.db')));
  const git=(cwd,...args)=>execFileSync('git',args,{cwd,encoding:'utf8',timeout:10000,windowsHide:true,stdio:'pipe'}).trim();
  const worktrees=manifest.worktrees.map(p=>{const identity=h.checked(p,'directory');assert(h.within(identity.path,allocation.path));assert.equal(git(identity.path,'status','--porcelain','--untracked-files=normal'),'');const gitDir=h.checked(git(identity.path,'rev-parse','--absolute-git-dir'),'directory');assert(h.within(gitDir.path,allocation.path));return {identity,git_dir:gitDir,head:git(identity.path,'rev-parse','HEAD')};});
  const probe=h.checked(manifest.change_probe.path,'file');assert(h.within(probe.path,source.path));assert.equal(manifest.change_probe.tracked,true);assert.equal(fs.readFileSync(probe.path,'utf8'),manifest.change_probe.initial_bytes);assert.equal(git(source.path,'ls-files','--error-unmatch','--','devmap-change-probe.txt'),'devmap-change-probe.txt');
  const legacy=h.inventory(worktrees.map(w=>path.join(w.git_dir.path,'devmap'))),events=legacy.inventory.filter(f=>f.kind==='file'&&f.relative.endsWith('events.ndjson'));assert.equal(events.length,100);
  let recordCount=0;
  for(const entry of events){const workspace=worktrees.find(w=>path.join(w.git_dir.path,'devmap')===entry.root);assert(workspace);
   const lines=fs.readFileSync(path.join(entry.root,entry.relative),'utf8').trim().split('\n');assert.equal(lines.length,1000);
   for(const line of lines){const record=JSON.parse(line);assert.equal(record.event.context.head,workspace.head);recordCount++;}
  }
  assert.equal(recordCount,100000);
  const inventoryPath=path.join(run,'legacy-inventory.json');fs.writeFileSync(inventoryPath,JSON.stringify(legacy,null,2),{flag:'wx'});
  Object.assign(report,{manifest:manifestPath,manifest_sha256:hash(manifestPath),allocation,source,worktrees,probe:{...probe,sha256:hash(probe.path),head:worktrees[0].head},inventory:inventoryPath,legacy_inventory_sha256:legacy.sha256,event_bytes:events.reduce((sum,e)=>sum+Number(e.size),0),record_count:recordCount,database_absent:true,completed:true});
 }catch(e){report.error=String(e.stack||e);process.exitCode=1;}
 finally{fs.writeFileSync(path.join(run,'report.json'),JSON.stringify(report,null,2),{flag:'wx'});console.log(JSON.stringify({run,completed:report.completed,error:report.error}));}
}
main().catch(e=>{console.error(e);process.exitCode=1;});
