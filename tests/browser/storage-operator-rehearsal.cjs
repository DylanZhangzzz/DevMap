'use strict';
// Destructive fault is restricted to an exclusive disposable legacy fixture.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const {execFileSync,spawnSync}=require('node:child_process');
const h=require('./shared-summary-performance.cjs');
const root=path.resolve(__dirname,'../..'),allowed=h.checked(path.join(root,'target/verification'),'directory').path;
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function main(){
 const manifestPath=h.checked(path.resolve(process.argv[2]),'file').path;assert(h.within(manifestPath,allowed));
 const manifest=JSON.parse(fs.readFileSync(manifestPath));assert.equal(manifest.scope,'legacy_native_process_fixture');
 const fixture=h.checked(manifest.fixture_root,'directory'),source=h.checked(manifest.source,'directory');assert(h.within(source.path,fixture.path));
 const exe=h.checked(process.env.DEVMAP_CANDIDATE_EXE,'file').path,old=h.checked(process.env.DEVMAP_BASELINE_EXE,'file').path,python=h.checked(process.env.DEVMAP_PYTHON_EXE,'file').path;
 assert(h.within(exe,allowed)&&h.within(old,allowed));assert.equal(hash(exe),'4c9109976a11d1604098a7636930e781b73aa357f62d0a51e814de2d2e3726d9');assert.equal(hash(old),manifest.baseline_sha256.toLowerCase());
 const db=path.join(source.path,'.git/devmap/devmap.db');assert(!fs.existsSync(db),'Only an unmigrated disposable fixture is accepted');
 const run=path.join(fixture.path,'operator-rehearsal');fs.mkdirSync(run); // exclusive ownership; never reuse
 const frozen=path.join(run,'activation'),backup=path.join(run,'accepted.db');
 const originals=manifest.frozen_sources.flatMap(s=>s.files.map(f=>({path:path.join(s.original,f.relative),sha:f.sha256})));
 const preserveOriginals=()=>{for(const f of originals)assert.equal(hash(f.path),f.sha);};
 const report={scope:'current CLI isolated migration, accepted SQL write, backup and late-old-writer refusal',candidate_sha256:hash(exe),source:source.path,steps:[],passed:false};
 function command(label,args,success=true){const r=spawnSync(exe,args,{encoding:'utf8',timeout:30000,windowsHide:true});fs.writeFileSync(path.join(run,label+'.stdout'),r.stdout||'',{flag:'wx'});fs.writeFileSync(path.join(run,label+'.stderr'),r.stderr||'',{flag:'wx'});report.steps.push({label,args,status:r.status,error:r.error?String(r.error):null});assert(!r.error);if(success)assert.equal(r.status,0,r.stderr);else assert.notEqual(r.status,0);return r;}
 function tool(binary,label,name,args){const input=[{jsonrpc:'2.0',id:0,method:'initialize',params:{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'operator-rehearsal',version:'1'}}},{jsonrpc:'2.0',id:1,method:'tools/call',params:{name,arguments:args}}];const output=execFileSync(binary,['mcp','--source',source.path],{input:input.map(JSON.stringify).join('\n')+'\n',encoding:'utf8',timeout:30000,windowsHide:true,maxBuffer:4*1024*1024});fs.writeFileSync(path.join(run,label+'.jsonl'),output,{flag:'wx'});const response=output.trim().split(/\r?\n/).map(JSON.parse).find(r=>r.id===1);assert(response.result&&!response.error&&!response.result.isError,JSON.stringify(response));return response.result.structuredContent;}
 try{
  preserveOriginals();command('before-inspect',['storage','inspect','--source',source.path]);assert(!fs.existsSync(db));
  command('missing-verify',['storage','verify','--source',source.path],false);assert(!fs.existsSync(db));
  command('migration',['storage','migrate','--source',source.path,'--backup-dir',frozen]);command('verify',['storage','verify','--source',source.path]);preserveOriginals();
  const before=h.sqlState(python,db),frozenState=h.inventory([frozen]);
  command('retry-migration',['storage','migrate','--source',source.path,'--backup-dir',frozen]);assert.deepEqual(h.sqlState(python,db),before);
  const map=tool(exe,'before-new-write','devmap_read_map',{}),route=map.route_plans[0];assert.equal(route.revision,2);
  const request={request_id:'operator-rehearsal-sql-1',route_id:route.route_id,worktree_id:route.worktree_id,expected_revision:2,goal:'Accepted after controlled SQL migration',source:'operator rehearsal'};
  const accepted=tool(exe,'accepted-write','devmap_set_route_plan',request);assert.equal(accepted.revision,3);assert.equal(accepted.goal,request.goal);
  assert.deepEqual(tool(exe,'idempotent-retry','devmap_set_route_plan',request),accepted);
  const after=h.sqlState(python,db);assert.notDeepEqual(after.tables.route_records,before.tables.route_records);preserveOriginals();
  command('backup',['storage','backup','--source',source.path,'--destination',backup]);assert.deepEqual(h.sqlState(python,backup),after);const backupHash=hash(backup);
  command('backup-overwrite-refused',['storage','backup','--source',source.path,'--destination',backup],false);assert.equal(hash(backup),backupHash);
  const session=manifest.inventory[0].id;
  tool(old,'late-old-write','devmap_record_evidence',{session_id:session,agent_id:'late-old-fixture-agent',event_id:'operator-rehearsal-late-old-1',occurred_at:'2026-09-13T11:30:00Z',kind:'test',target:'commit:'+route.start_commit,outcome:'pending'});
  const drift=command('drift-verify',['storage','verify','--source',source.path],false);assert.match(drift.stderr,/drift|changed|differ/i);
  command('drift-remigration-refused',['storage','migrate','--source',source.path,'--backup-dir',frozen],false);
  assert.deepEqual(h.sqlState(python,db),after);assert.equal(hash(backup),backupHash);assert.deepEqual(h.inventory([frozen]),frozenState);
  const journal=path.join(source.path,'.git/devmap/sessions',session,'events.ndjson');assert(fs.readFileSync(journal,'utf8').includes('operator-rehearsal-late-old-1'));
  report.accepted_route=accepted;report.sql_before=before;report.sql_after=after;report.backup_sha256=backupHash;report.both_histories_retained=true;report.passed=true;
 }catch(error){report.error=String(error.stack||error);process.exitCode=1;}
 finally{assert.deepEqual(h.checked(fixture.path,'directory'),fixture);assert.deepEqual(h.checked(source.path,'directory'),source);fs.writeFileSync(path.join(run,'report.json'),JSON.stringify(report,null,2),{flag:'wx'});}
 console.log(JSON.stringify({run,passed:report.passed,error:report.error}));
}
main();
