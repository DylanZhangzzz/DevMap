'use strict';
// Native command timing only; not host trigger/trust or a percentile gate.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const {execFileSync,spawnSync}=require('node:child_process');
const root=path.resolve(__dirname,'../..');
const exe=fs.realpathSync(process.env.DEVMAP_CANDIDATE_EXE);assert(exe.startsWith(path.join(root,'target')+path.sep));
const sha=crypto.createHash('sha256').update(fs.readFileSync(exe)).digest('hex');assert.equal(sha,'4c9109976a11d1604098a7636930e781b73aa357f62d0a51e814de2d2e3726d9');
const run=fs.mkdtempSync(path.join(root,'target/verification/hook-budget-')),source=path.join(run,'repository');fs.mkdirSync(source);
for(const args of [['init','-b','main'],['config','user.name','Hook budget fixture'],['config','user.email','fixture@example.invalid'],['commit','--allow-empty','-m','base']])execFileSync('git',args,{cwd:source,stdio:'pipe'});
const fixture=JSON.parse(fs.readFileSync(path.join(root,'tests/fixtures/hooks/codex-events.json'))),rows=[];
const report={scope:'Three native start/end pairs on a fresh disposable repo; no host hooks enabled and no enforced one-second kill',run,source,candidate_sha256:sha,rows,completed:false};
try{
 for(let i=0;i<3;i++)for(const event of ['SessionStart','SessionEnd']){
  const body={...fixture[event],cwd:source,session_id:`hook-budget-${i}`};
  const start=performance.now(),r=spawnSync(exe,['hook','handle','--source',source,'--host','codex','--event',event,'--binding-id',`devmap/v1/codex/${event}`],{cwd:source,input:JSON.stringify(body),encoding:'utf8',timeout:30000,windowsHide:true});
  rows.push({event,index:i,elapsed_ms:performance.now()-start,status:r.status,error:r.error?String(r.error):null,stdout:r.stdout,stderr:r.stderr});assert(!r.error);assert.equal(r.status,0,r.stderr);assert.deepEqual(JSON.parse(r.stdout),{});
 }
 report.completed=true;report.ends_within_documented_default=rows.filter(r=>r.event==='SessionEnd').every(r=>r.elapsed_ms<=1000);
}catch(e){report.error=String(e.stack||e);process.exitCode=1;}
finally{fs.writeFileSync(path.join(run,'report.json'),JSON.stringify(report,null,2),{flag:'wx'});console.log(JSON.stringify(report));}
