'use strict';
// Native cold-end diagnostic. Never represents automatic host delivery.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),net=require('node:net');
const {spawnSync,execFileSync}=require('node:child_process'),{setTimeout:delay}=require('node:timers/promises');
const h=require('./shared-summary-performance.cjs');
function validateRecords(records,session){
 assert.equal(records.length,2);assert.deepEqual(records.map(r=>r.event.event_type),['session_started','session_stopped']);
 assert(records.every(r=>r.event.context.session_id===session&&r.event.host.name==='codex'));
 assert.deepEqual(records[1].event.context,records[0].event.context);assert.deepEqual(records[1].event.actor,records[0].event.actor);
 assert.equal(records[0].sequence,1);assert.equal(records[1].previous_sha256,records[0].sha256);assert.equal(records[1].sequence,2);
}
async function main(){
 const root=path.resolve(__dirname,'../..'),exe=h.checked(process.env.DEVMAP_CANDIDATE_EXE,'file').path,python=h.checked(process.env.DEVMAP_PYTHON_EXE,'file').path;
 assert(h.within(exe,path.join(root,'target/verification')));assert.equal(h.runtime.hash(fs.readFileSync(exe)),'25bf6631ef7387e524744a1b2b8cce01b76d54c58cdb897353acc5d042dfadec');
 const run=fs.mkdtempSync(path.join(root,'target/verification/hook-cold-end-')),source=path.join(run,'repository');fs.mkdirSync(source);
 for(const args of [['init','-b','main'],['config','user.name','Cold end fixture'],['config','user.email','fixture@example.invalid'],['commit','--allow-empty','-m','base']])execFileSync('git',args,{cwd:source,stdio:'pipe'});
 const inputs=JSON.parse(fs.readFileSync(path.join(root,'tests/fixtures/hooks/codex-events.json'))),session='cold-end-fixture';
 const report={scope:'Native end after idle endpoint disappearance; no host trigger and no three-second kill',run,source,candidate_sha256:h.runtime.hash(fs.readFileSync(exe)),rows:[],completed:false};
 console.log(JSON.stringify({run}));
 function invoke(event){const start=performance.now(),r=spawnSync(exe,['hook','handle','--source',source,'--host','codex','--event',event,'--binding-id',`devmap/v1/codex/${event}`],{cwd:source,input:JSON.stringify({...inputs[event],session_id:session,cwd:source}),encoding:'utf8',timeout:30000,windowsHide:true});report.rows.push({event,elapsed_ms:performance.now()-start,status:r.status,error:r.error?String(r.error):null,stdout:r.stdout,stderr:r.stderr});assert(!r.error);assert.equal(r.status,0,r.stderr);}
 try{
  const identity=JSON.parse(execFileSync(exe,['runtime','--identity','--source',source],{encoding:'utf8',timeout:15000,windowsHide:true}));assert.match(identity.repository,/^[a-f0-9]{64}$/);
  invoke('SessionStart');console.log('Start accepted; waiting 75 seconds without runtime probes.');
  await delay(75000);
  await new Promise((resolve,reject)=>{const socket=net.createConnection(`\\\\.\\pipe\\devmap-${identity.repository}`);const timer=setTimeout(()=>{socket.destroy();reject(new Error('Endpoint probe timeout'));},3000);socket.on('connect',()=>{clearTimeout(timer);socket.destroy();reject(new Error('Owner endpoint still present; cold boundary unproven'));});socket.on('error',e=>{clearTimeout(timer);socket.destroy();e.code==='ENOENT'?resolve():reject(e);});});
  report.endpoint_absent_before_end=true;invoke('SessionEnd');
  const db=path.join(source,'.git/devmap/devmap.db');report.sql_state=h.sqlState(python,db);
  const script='import sqlite3,json,sys,pathlib\nc=sqlite3.connect(pathlib.Path(sys.argv[1]).resolve().as_uri()+"?mode=ro",uri=True)\nprint(json.dumps([json.loads(r[0]) for r in c.execute("select record_json from journal_records order by sequence")]))';
  const records=JSON.parse(execFileSync(python,['-c',script,db],{encoding:'utf8',windowsHide:true,timeout:15000}));
  validateRecords(records,session);
  report.records=records;report.durable_stop_verified=true;report.within_three_seconds=report.rows[1].elapsed_ms<=3000;report.completed=true;
 }catch(e){report.error=String(e.stack||e);process.exitCode=1;}
 finally{fs.writeFileSync(path.join(run,'report.json'),JSON.stringify(report,null,2),{flag:'wx'});console.log(JSON.stringify({run,completed:report.completed,within_three_seconds:report.within_three_seconds,rows:report.rows,error:report.error}));}
}
if(require.main===module)main().catch(e=>{console.error(e);process.exitCode=1;});
module.exports={validateRecords};
