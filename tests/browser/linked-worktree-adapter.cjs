'use strict';
// Native adapter configuration boundary only; does not enable or trust host hooks.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const {execFileSync,spawnSync}=require('node:child_process'),h=require('./shared-summary-performance.cjs');
const root=path.resolve(__dirname,'../..'),allowed=path.join(root,'target/verification');
const variants=[['baseline',process.env.DEVMAP_BASELINE_EXE,'a1cfbb1c46bd9026b18db67da9f70485b0bb20c6c54fd779475b52531731d419'],['candidate',process.env.DEVMAP_CANDIDATE_EXE,'25bf6631ef7387e524744a1b2b8cce01b76d54c58cdb897353acc5d042dfadec']].map(([name,file,sha])=>{const exe=h.checked(file,'file').path;assert(h.within(exe,allowed));assert.equal(h.runtime.hash(fs.readFileSync(exe)),sha);return {name,exe,sha};});
const run=fs.mkdtempSync(path.join(allowed,'linked-adapter-')),report={run,scope:'native adapter discovery in a newly added sibling worktree; no actual host discovery or trust changes',variants:[],passed:false};
try{
 for(const variant of variants){
  const base=path.join(run,variant.name),main=path.join(base,'main'),linked=path.join(base,'added');fs.mkdirSync(main,{recursive:true});
  const git=(...args)=>execFileSync('git',args,{cwd:main,encoding:'utf8',windowsHide:true,timeout:15000,stdio:'pipe'});
  for(const args of [['init','-b','main'],['config','user.name','Adapter fixture'],['config','user.email','fixture@example.invalid'],['commit','--allow-empty','-m','base']])git(...args);
  const call=(label,action,source,extra=[],expectedStatus=0)=>{const result=spawnSync(variant.exe,['adapter',action,'--source',source,'--host','codex',...extra],{cwd:source,encoding:'utf8',timeout:30000,windowsHide:true});fs.writeFileSync(path.join(base,label+'.json'),JSON.stringify({status:result.status,error:result.error?String(result.error):null,stdout:result.stdout,stderr:result.stderr},null,2),{flag:'wx'});assert(!result.error);assert.equal(result.status,expectedStatus,result.stderr);return result.stdout;};
  const plan=call('main-plan','plan',main),digest=plan.match(/^plan_digest=(.+)$/m)?.[1].trim();assert(digest);
  assert.match(call('main-install','install',main,['--plan-digest',digest]),/^changed=true\r?$/m);
  const config=path.join(main,'.codex/hooks.json'),before=fs.readFileSync(config);
  assert.match(call('main-verify','verify',main),/^configured=true\r?$/m);
  git('worktree','add','-b','codex/added',linked);
  const verification=call('linked-verify','verify',linked,[],1),linkedPlan=call('linked-plan','plan',linked);
  assert.match(verification,/^configured=false\r?$/m);assert.match(verification,/^activation_verified=false\r?$/m);
  const missing=verification.match(/^missing=(.*)$/m)?.[1].trim().split(',');assert.equal(missing.length,10);
  const planned=linkedPlan.match(/^config_path=(.+)$/m)?.[1].trim();assert.equal(path.resolve(planned),path.join(linked,'.codex/hooks.json'));
  assert(!fs.existsSync(path.join(linked,'.codex')));assert.deepEqual(fs.readFileSync(config),before);
  const adapterFiles=h.inventory([path.join(main,'.git/devmap')]).inventory.filter(f=>f.kind==='file').map(f=>f.relative);
  assert.deepEqual(adapterFiles,['adapter-install.lock']);
  report.variants.push({name:variant.name,sha256:variant.sha,main_configured:true,linked_configured:false,linked_missing:missing,linked_planned_config:planned,main_config_sha256:h.runtime.hash(before),main_config_preserved:true,linked_config_absent:true,git_devmap_files:adapterFiles});
 }
 assert.deepEqual(report.variants[0].linked_missing,report.variants[1].linked_missing);report.passed=true;
}catch(e){report.error=String(e.stack||e);process.exitCode=1;}
finally{fs.writeFileSync(path.join(run,'report.json'),JSON.stringify(report,null,2),{flag:'wx'});console.log(JSON.stringify({run,passed:report.passed,error:report.error}));}
