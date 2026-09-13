'use strict';
// Read-only second real Codex CLI session on an owned empty-start fixture.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),crypto=require('node:crypto'),net=require('node:net');
const {spawn,execFileSync}=require('node:child_process');
const h=require('./shared-summary-performance.cjs');
const root=path.resolve(__dirname,'../..'),allowed=h.checked(path.join(root,'target/verification'),'directory').path;
const read=p=>JSON.parse(fs.readFileSync(p,'utf8'));
const sha=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function validateReadback(map,first,expected){assert.equal(map.schema_version,'devmap/dock/4');assert.equal(map.repository_id,first.repository_id);assert.equal(map.current_worktree_id,first.current_worktree_id);assert.deepEqual(map.route_plans,[expected]);}
async function absent(repository){assert.match(repository,/^[a-f0-9]{64}$/);await new Promise((resolve,reject)=>{const socket=net.createConnection(`\\\\.\\pipe\\devmap-${repository}`);let settled=false;const finish=e=>{if(settled)return;settled=true;clearTimeout(timer);socket.destroy();e?reject(e):resolve();};const timer=setTimeout(()=>finish(new Error('Endpoint check timed out')),3000);socket.on('connect',()=>finish(new Error('Existing owner is still reachable; do not restart or kill it')));socket.on('error',e=>finish(e.code==='ENOENT'?null:e));});}
async function main(){
 assert.equal(process.platform,'win32');
 const fixture=h.checked(path.resolve(process.argv[2]),'directory');assert(h.within(fixture.path,allowed));
 const manifest=read(path.join(fixture.path,'manifest.json')),verified=read(path.join(fixture.path,'report-validated.json'));
 assert.equal(manifest.scope,'actual_codex_cli_mcp_smoke');assert.equal(manifest.empty_start,true);assert.equal(manifest.manual_migration_performed,false);assert.equal(verified.passed,true);
 const candidate=h.checked(manifest.candidate,'file');assert(h.within(candidate.path,allowed));assert.equal(sha(candidate.path),manifest.candidate_sha256);
 const source=h.checked(manifest.source,'directory');assert(h.within(source.path,fixture.path));
 const python=h.checked(process.env.DEVMAP_PYTHON_EXE,'file').path;
 const db=path.join(source.path,'.git/devmap/devmap.db');
 const baseline=read(path.join(fixture.path,'startup-sql.json'));assert.deepEqual(h.sqlState(python,db),baseline);
 const backups=baseline.activation_snapshot_paths.map(p=>h.checked(p,'directory').path);
 const immutable=h.inventory(backups);
 const original=fs.readFileSync(path.join(fixture.path,'events.jsonl'),'utf8').trim().split(/\r?\n/).map(JSON.parse);
 const result=original.filter(e=>e.type==='item.completed'&&e.item?.type==='mcp_tool_call'&&e.item.tool==='devmap_read_map').at(-1).item.result.structured_content;
 assert.equal(result.route_plans.length,1);const expected=result.route_plans[0];
 const identity=JSON.parse(execFileSync(candidate.path,['runtime','--identity','--source',source.path],{encoding:'utf8',timeout:15000,windowsHide:true}));
 assert.equal(h.checked(identity.source,'directory').path,source.path);await absent(identity.repository);
 const run=fs.mkdtempSync(path.join(fixture.path,'reopen-'));
 const prompt='This is an isolated read-only MCP persistence test. Use only devmap_candidate. Call devmap_read_map exactly once with no arguments. Verify that route_plans contains the following exact persisted route: '+JSON.stringify(expected)+'. Do not use shell, edit files, browse, write data or call any other tools. Report PASS only if the returned route and repository/worktree identity match.';
 const args=manifest.args.slice(0,-1).map(arg=>arg.startsWith('mcp_servers.devmap_candidate.enabled_tools=')?'mcp_servers.devmap_candidate.enabled_tools=["devmap_read_map"]':arg);args.push(prompt);
 const report={scope:'actual_codex_cli_readonly_reopen',run,candidate_sha256:manifest.candidate_sha256,source:source.path,previous_thread:verified.thread_id,endpoint_absent_before:true,expected,passed:false};
 const out=fs.openSync(path.join(run,'events.jsonl'),'wx'),err=fs.openSync(path.join(run,'stderr.log'),'wx');
 let child;try{child=spawn(manifest.codex,args,{cwd:source.path,windowsHide:true,stdio:['ignore',out,err]});}finally{fs.closeSync(out);fs.closeSync(err);}
 console.log(JSON.stringify({run}));let expired=false;
 try{
  report.code=await new Promise((resolve,reject)=>{const timer=setTimeout(()=>{expired=true;child.kill();},180000);child.once('error',e=>{clearTimeout(timer);reject(e);});child.once('close',code=>{clearTimeout(timer);resolve(code);});});report.expired=expired;
  const events=fs.readFileSync(path.join(run,'events.jsonl'),'utf8').trim().split(/\r?\n/).map(JSON.parse),items=events.filter(e=>e.type==='item.completed').map(e=>e.item);
  report.thread_id=events.find(e=>e.type==='thread.started')?.thread_id;assert(report.thread_id&&report.thread_id!==verified.thread_id);
  const calls=items.filter(i=>i?.type==='mcp_tool_call');assert.equal(calls.length,1);const call=calls[0];assert.equal(call.server,'devmap_candidate');assert.equal(call.tool,'devmap_read_map');assert.equal(call.status,'completed');assert(!call.error&&!call.result?.isError&&!call.result?.is_error);
  assert(!items.some(i=>['command_execution','file_change','web_search'].includes(i?.type)));
  validateReadback(call.result.structured_content,result,expected);
  assert.equal(report.code,0);assert(!expired);report.readback_verified=true;
 }catch(e){report.error=String(e.stack||e);}
 finally{
  try{assert.deepEqual(h.checked(fixture.path,'directory'),fixture);assert.deepEqual(h.checked(source.path,'directory'),source);assert.equal(sha(candidate.path),manifest.candidate_sha256);assert.deepEqual(h.sqlState(python,db),baseline);assert.deepEqual(h.inventory(backups),immutable);report.preserved=true;}catch(e){report.preservation_error=String(e);}
  report.passed=Boolean(report.readback_verified&&report.preserved&&!report.error&&!report.preservation_error);fs.writeFileSync(path.join(run,'report.json'),JSON.stringify(report,null,2),{flag:'wx'});
 }
 console.log(JSON.stringify(report));process.exitCode=report.passed?0:1;
}
if(require.main===module)main().catch(e=>{console.error(e);process.exitCode=1;});
module.exports={validateReadback};
