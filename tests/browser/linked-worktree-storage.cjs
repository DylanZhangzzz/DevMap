'use strict';
// Disposable actual CLI workflow: add a worktree after legacy activation.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const {execFileSync}=require('node:child_process'),h=require('./shared-summary-performance.cjs');
const root=path.resolve(__dirname,'../..'),allowed=path.join(root,'target/verification');
function main(){
 const exe=h.checked(process.env.DEVMAP_CANDIDATE_EXE,'file').path,old=h.checked(process.env.DEVMAP_BASELINE_EXE,'file').path,python=h.checked(process.env.DEVMAP_PYTHON_EXE,'file').path;
 for(const file of [exe,old])assert(h.within(file,allowed));assert.equal(h.runtime.hash(fs.readFileSync(exe)),'25bf6631ef7387e524744a1b2b8cce01b76d54c58cdb897353acc5d042dfadec');assert.equal(h.runtime.hash(fs.readFileSync(old)),'a1cfbb1c46bd9026b18db67da9f70485b0bb20c6c54fd779475b52531731d419');
 const run=fs.mkdtempSync(path.join(allowed,'linked-storage-')),source=path.join(run,'main'),linked=path.join(run,'added');fs.mkdirSync(source);
 const git=(cwd,...args)=>execFileSync('git',args,{cwd,encoding:'utf8',stdio:'pipe'}).trim();
 for(const args of [['init','-b','main'],['config','user.name','Linked storage fixture'],['config','user.email','fixture@example.invalid'],['commit','--allow-empty','-m','base']])git(source,...args);
 const report={scope:'native CLI linked worktree added after migration; no host hook trust or installation flow',run,source,linked,candidate_sha256:h.runtime.hash(fs.readFileSync(exe)),passed:false};
 function command(binary,label,args,input){const output=execFileSync(binary,args,{cwd:source,input,encoding:'utf8',timeout:30000,windowsHide:true,maxBuffer:4*1024*1024});fs.writeFileSync(path.join(run,label+'.json'),output,{flag:'wx'});return JSON.parse(output);}
 function hook(binary,label,cwd,session,event){return command(binary,label,['hook','handle','--source',cwd,'--host','codex','--event',event,'--binding-id',`devmap/v1/codex/${event}`],JSON.stringify({session_id:session,cwd,hook_event_name:event,source:'startup',reason:'other'}));}
 try{
  hook(old,'old-start',source,'retained-legacy-session','SessionStart');
  const mainData=path.join(source,'.git/devmap'),originals=h.inventory([mainData]);assert(!fs.existsSync(path.join(mainData,'devmap.db')));
  command(exe,'migration',['storage','migrate','--source',source,'--backup-dir',path.join(run,'frozen')]);
  const frozen=h.inventory([path.join(run,'frozen')]);
  git(source,'worktree','add','-b','codex/new-worktree',linked);
  const identity=command(exe,'linked-identity',['runtime','--identity','--source',linked]);
  const admin=h.checked(git(linked,'rev-parse','--absolute-git-dir'),'directory').path;
  const mainInspect=command(exe,'main-inspect',['storage','inspect','--source',source]);
  const linkedInspect=command(exe,'linked-inspect',['storage','inspect','--source',linked]);
  assert.equal(h.checked(mainInspect.database,'file').path,h.checked(linkedInspect.database,'file').path);assert.equal(linkedInspect.backend,'active');
  hook(exe,'new-start',linked,'new-linked-session','SessionStart');hook(exe,'new-end',linked,'new-linked-session','SessionEnd');
  assert(!fs.existsSync(path.join(linked,'.codex')),'No per-worktree adapter install performed');
  assert(!fs.existsSync(path.join(admin,'devmap/devmap.db')),'No independent linked database');
  const linkedData=path.join(admin,'devmap'),linkedFiles=fs.existsSync(linkedData)?h.inventory([linkedData]).inventory:[];
  assert(!linkedFiles.some(f=>f.kind==='file'&&(/events\.ndjson$|route-plans\.jsonl$|presence.*\.json$/.test(f.relative))),'New linked writes created legacy authority');
  for(const f of originals.inventory)if(f.kind==='file')assert.equal(h.runtime.hash(fs.readFileSync(path.join(f.root,f.relative))),f.sha256);
  assert.deepEqual(h.inventory([path.join(run,'frozen')]),frozen);
  const sql=h.sqlState(python,mainInspect.database);assert.equal(sql.tables.journal_records.rows,3);assert.equal(sql.tables.journal_sessions.rows,2);
  const query='import sqlite3,json,sys,pathlib\nc=sqlite3.connect(pathlib.Path(sys.argv[1]).resolve().as_uri()+"?mode=ro",uri=True)\nprint(json.dumps([json.loads(r[0]) for r in c.execute("select record_json from journal_records")]))';
  const records=JSON.parse(execFileSync(python,['-c',query,h.checked(mainInspect.database,'file').path],{encoding:'utf8',timeout:15000,windowsHide:true}));
  const newRows=records.filter(r=>r.event.context.session_id==='new-linked-session').sort((a,b)=>a.sequence-b.sequence);assert.equal(newRows.length,2);assert.deepEqual(newRows.map(r=>r.event.event_type),['session_started','session_stopped']);
  for(const row of newRows)assert.equal(h.checked(row.event.context.worktree,'directory').path,h.checked(linked,'directory').path);
  assert.equal(newRows[1].previous_sha256,newRows[0].sha256);
  const verified=command(exe,'verify',['storage','verify','--source',linked]);assert.equal(verified.verified,true);
  Object.assign(report,{runtime_identity:identity,shared_database:mainInspect.database,original_legacy_files_preserved:true,frozen_preserved:true,no_per_worktree_install:true,linked_files:linkedFiles,sql,records,passed:true});
 }catch(e){report.error=String(e.stack||e);process.exitCode=1;}
 finally{fs.writeFileSync(path.join(run,'report.json'),JSON.stringify(report,null,2),{flag:'wx'});console.log(JSON.stringify({run,passed:report.passed,error:report.error}));}
}
main();
