// Produce disposable legacy state through the frozen real executable, not SQL.
// This is a process fixture; it does not claim real Codex host integration.
const fs=require('node:fs');
const path=require('node:path');
const assert=require('node:assert/strict');
const crypto=require('node:crypto');
const {execFileSync}=require('node:child_process');
const root=path.resolve(__dirname,'../..');
const exe=process.env.DEVMAP_BASELINE_EXE;
assert.ok(exe,'Set DEVMAP_BASELINE_EXE to the verified frozen baseline binary');
const expected='A1CFBB1C46BD9026B18DB67DA9F70485B0BB20C6C54FD779475B52531731D419';
const hash=crypto.createHash('sha256').update(fs.readFileSync(exe)).digest('hex').toUpperCase();
assert.equal(hash,expected,'Unexpected baseline executable');
const output=path.join(root,'target/verification');
fs.mkdirSync(output,{recursive:true});
const fixtureRoot=fs.mkdtempSync(path.join(output,'legacy-process-'));
const repo=path.join(fixtureRoot,'main'), linked=path.join(fixtureRoot,'feature');
fs.mkdirSync(repo);
function git(cwd,...args) { return execFileSync('git',args,{cwd,encoding:'utf8',stdio:['pipe','pipe','pipe']}).trim(); }
git(repo,'init','-b','main');
git(repo,'config','user.name','DevMap Compatibility Fixture');
git(repo,'config','user.email','fixture@example.invalid');
git(repo,'commit','--allow-empty','-m','Fixture base');
git(repo,'worktree','add','-b','codex/fixture',linked);
git(linked,'commit','--allow-empty','-m','Fixture development');
const taskIds=['019a0000-0000-7000-8000-000000000001','019a0000-0000-7000-8000-000000000002'];
const inventory=taskIds.map((id,i)=>({id,title:i?'Feature fixture':'Main fixture',status:i?'waiting':'active',lifecycle:'present',cwd:i?linked:repo,updatedAt:Date.now(),hostId:'local',kind:'codex'}));
const init={jsonrpc:'2.0',id:0,method:'initialize',params:{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'devmap-fixture',version:'1'}}};
const exchanges=[];
function tool(source,name,args) {
  const request={jsonrpc:'2.0',id:1,method:'tools/call',params:{name,arguments:args}};
  const result=execFileSync(exe,['mcp','--source',source],{input:JSON.stringify(init)+'\n'+JSON.stringify(request)+'\n',encoding:'utf8',timeout:30000,maxBuffer:4*1024*1024});
  const lines=result.trim().split(/\r?\n/).map(JSON.parse);
  const response=lines.find(x=>x.id===1);
  assert.ok(response?.result&&!response.error&&!response.result.isError,JSON.stringify(response));
  exchanges.push({source,request,response});
  return response.result.structuredContent;
}
const before=tool(repo,'devmap_read_map',{codex_tasks:inventory,codex_tasks_complete:true});
assert.equal(before.schema_version,'devmap/dock/4');
const input={request_id:'fixture-route-1',expected_revision:0,worktree_id:before.current_worktree_id,goal:'Preserve current web behavior',source:'isolated acceptance fixture'};
const first=tool(repo,'devmap_set_route_plan',input);
assert.deepEqual(tool(repo,'devmap_set_route_plan',input),first);
tool(repo,'devmap_set_route_plan',{...input,request_id:'fixture-route-2',route_id:first.route_id,expected_revision:1,goal:'Preserve current web behavior and durable state'});
for(let i=0;i<2;i++) {
  tool(i?linked:repo,'devmap_record_evidence',{session_id:taskIds[i],agent_id:`fixture-agent-${i}`,event_id:`fixture-evidence-${i}`,occurred_at:'2026-09-08T16:00:00Z',kind:'test',target:'commit:'+git(i?linked:repo,'rev-parse','HEAD'),outcome:'pending'});
}
const snapshot=tool(repo,'devmap_read_map',{codex_tasks:inventory,codex_tasks_complete:true});
assert.equal(snapshot.route_plans[0].revision,2);
assert.equal(fs.existsSync(path.join(repo,'.git/devmap/devmap.db')),false);
const frozenSources=[];
for(const [index,source] of [repo,linked].entries()) {
  const original=path.join(git(source,'rev-parse','--absolute-git-dir'),'devmap');
  const destination=path.join(fixtureRoot,'frozen',String(index));
  const files=[];
  function copy(directory,relative='') {
    for(const name of fs.readdirSync(directory).sort()) {
      const from=path.join(directory,name), rel=path.join(relative,name);
      const meta=fs.lstatSync(from);
      assert.equal(meta.isSymbolicLink(),false,'Unexpected fixture symlink');
      if(meta.isDirectory()) copy(from,rel);
      else {
        assert.ok(meta.isFile());
        const bytes=fs.readFileSync(from), to=path.join(destination,rel);
        fs.mkdirSync(path.dirname(to),{recursive:true});fs.writeFileSync(to,bytes,{flag:'wx'});
        files.push({relative:rel,bytes:bytes.length,sha256:crypto.createHash('sha256').update(bytes).digest('hex')});
      }
    }
  }
  copy(original);
  frozenSources.push({original,destination,files});
}
const manifest={scope:'legacy_native_process_fixture',baseline_sha256:hash,fixture_root:fixtureRoot,source:repo,linked_source:linked,inventory,frozen_sources:frozenSources,created_at:new Date().toISOString(),note:'Disposable legacy fixture generated through frozen MCP process. Host integration and migrated parity remain separate gates.'};
fs.writeFileSync(path.join(fixtureRoot,'manifest.json'),JSON.stringify(manifest,null,2));
fs.writeFileSync(path.join(fixtureRoot,'baseline-snapshot.json'),JSON.stringify(snapshot,null,2));
fs.writeFileSync(path.join(fixtureRoot,'exchanges.json'),JSON.stringify(exchanges,null,2));
console.log(JSON.stringify(manifest));
