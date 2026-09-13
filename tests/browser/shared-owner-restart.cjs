// Real candidate MCP + HTTP/SSE + Chromium owner replacement gate. Windows only.
// Owns a fresh fixture and the first owner process. Replacement expires naturally.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const net = require('node:net');
const crypto = require('node:crypto');
const {spawn, execFileSync} = require('node:child_process');
const {createInterface} = require('node:readline');
const {setTimeout: delay} = require('node:timers/promises');
const root = path.resolve(__dirname, '../..');
const modules = 'C:/Users/user/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules';
const {chromium} = require(path.join(modules, 'playwright'));
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex');

async function main() {
  assert.equal(process.platform, 'win32', 'Named-pipe fixture requires Windows');
  const input = fs.realpathSync(process.env.DEVMAP_CANDIDATE_EXE);
  const fixture = fs.mkdtempSync(path.join(root, 'target/verification/shared-browser-'));
  console.log(JSON.stringify({fixture}));
  const exe = path.join(fixture, 'devmap.exe');
  fs.copyFileSync(input, exe);
  const repo = path.join(fixture, 'repository'); fs.mkdirSync(repo);
  const git = (...args) => execFileSync('git', args, {cwd:repo, encoding:'utf8'}).trim();
  git('init', '-b', 'main'); git('config', 'user.name', 'Restart fixture');
  git('config', 'user.email', 'fixture@example.invalid'); git('commit', '--allow-empty', '-m', 'base');
  for (let i=0; i<6; i++) git('worktree', 'add', '-b', `lane-${i}`, path.join(fixture, `lane-${i}`));
  execFileSync(exe,['storage','migrate','--source',repo,'--backup-dir',path.join(fixture,'frozen')],{timeout:30000,stdio:'pipe'});
  const identity = JSON.parse(execFileSync(exe, ['runtime', '--identity', '--source', repo], {encoding:'utf8',timeout:10000}));
  const build = hash(fs.readFileSync(exe));
  // Reuse Rust's canonical identity, including Windows extended-path prefix.
  const hello = {protocol:1, repository:identity.repository, build, source:identity.source,
    git_dir:identity.git_dir, client_instance:crypto.randomBytes(16).toString('hex')};
  async function observeOwner() {
    return new Promise((resolve,reject) => {
      const socket = net.createConnection(`\\\\.\\pipe\\devmap-${identity.repository}`);
      const timer = setTimeout(()=>socket.destroy(new Error('Hello deadline')),3000);
      let bytes = Buffer.alloc(0);
      socket.on('error',reject); socket.on('close',()=>clearTimeout(timer));
      socket.on('connect',()=>{const body=Buffer.from(JSON.stringify(hello)), prefix=Buffer.alloc(4);prefix.writeUInt32BE(body.length);socket.write(Buffer.concat([prefix,body]));});
      socket.on('data',chunk=>{
        bytes=Buffer.concat([bytes,chunk]);
        if(bytes.length<4)return;
        const size=bytes.readUInt32BE(); if(size>16384){socket.destroy(new Error('Oversized Hello'));return;}
        if(bytes.length<size+4)return;
        try {const reply=JSON.parse(bytes.subarray(4,4+size));assert.equal(reply.status,'Accepted',JSON.stringify(reply));
          assert.equal(reply.welcome.repository,hello.repository);assert.equal(reply.welcome.build,build);
          assert.equal(reply.welcome.client_instance,hello.client_instance);resolve(reply.welcome);
        } catch(error){reject(error);} finally {socket.destroy();}
      });
    });
  }
  async function waitOwner(predicate) {
    const deadline=Date.now()+15000; let last;
    while(Date.now()<deadline){try {const value=await observeOwner();if(predicate(value))return value;}catch(error){last=error;}await delay(100);}
    throw new Error(`Expected owned runtime not observed: ${last}`);
  }
  const nonce=crypto.randomBytes(16).toString('hex');
  const owner=spawn(exe,['runtime','--owner','--source',repo,'--instance',nonce,'--idle-seconds','60'],{stdio:['ignore','ignore','pipe'],windowsHide:true});
  owner.stderr.on('data',bytes=>fs.appendFileSync(path.join(fixture,'initial-owner-stderr.log'),bytes));
  let mcp, browser, lines; const pending=new Map(); let next=0;
  async function stop(child) {if(!child||child.exitCode!==null||child.signalCode!==null)return;const done=new Promise(resolve=>child.once('exit',resolve));child.kill();await done;}
  try {
    const beforeOwner=await waitOwner(w=>w.owner_pid===owner.pid&&w.owner_instance===nonce);
    mcp=spawn(exe,['mcp','--source',repo],{stdio:['pipe','pipe','pipe'],windowsHide:true});
    let stderr=''; mcp.stderr.on('data',b=>{stderr=(stderr+b).slice(-8192);});
    function fail(error){for(const entry of pending.values()){clearTimeout(entry.timer);entry.reject(error);}pending.clear();}
    mcp.on('error',fail);mcp.on('exit',code=>fail(new Error(`MCP exit ${code}: ${stderr}`)));
    lines=createInterface({input:mcp.stdout});lines.on('line',line=>{try{
      const result=JSON.parse(line),entry=pending.get(result.id);assert.ok(entry,'Unexpected response');pending.delete(result.id);clearTimeout(entry.timer);entry.resolve(result);
    }catch(error){fail(error);}});
    function request(method,params){return new Promise((resolve,reject)=>{const id=++next;
      const timer=setTimeout(()=>{pending.delete(id);reject(new Error('MCP request deadline'));},45000);
      pending.set(id,{resolve,reject,timer});mcp.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
    async function call(name,args={}){const r=await request('tools/call',{name,arguments:args});assert.ok(r.result&&!r.error&&!r.result.isError,JSON.stringify(r));return r.result.structuredContent;}
    const init=await request('initialize',{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'restart-gate',version:'1'}});assert.ok(init.result&&!init.error);
    const tasks=Array.from({length:6},(_,i)=>({id:`01a00000-0000-7000-8000-${String(i+1).padStart(12,'0')}`,
      title:`Restart fixture task ${i+1}`,status:'active',lifecycle:'present',cwd:repo,updatedAt:Math.floor(Date.now()/1000),hostId:'local',kind:'codex'}));
    const initial=await call('devmap_read_map',{codex_tasks:tasks,codex_tasks_complete:true});assert.equal(initial.schema_version,'devmap/dock/4');
    fs.writeFileSync(path.join(fixture,'initial-model.json'),JSON.stringify(initial,null,2));
    assert.equal(initial.lanes.flatMap(lane=>lane.chats).length,tasks.length,'Supplied task rows must be visible before opening browser');
    const opened=await call('devmap_open_map',{surface:'browser'}); const originalPid=mcp.pid;
    browser=await chromium.launch({executablePath:'C:/Program Files/Google/Chrome/Application/chrome.exe',headless:true});
    const page=await browser.newPage({viewport:{width:560,height:900},deviceScaleFactor:1});
    await page.addInitScript(()=>{
      window.__receivedDock=[];const NativeEventSource=window.EventSource;
      window.EventSource=class extends NativeEventSource {
        constructor(...args){super(...args);this.addEventListener('dock',event=>{
          const data=JSON.parse(event.data);window.__receivedDock.push({revision:data.revision,observation_revision:data.observation_revision});
        });}
      };
    });
    const errors=[]; page.on('pageerror',e=>errors.push(String(e)));
    await page.goto(opened.url);await page.locator('.route-platform').first().waitFor();
    await page.screenshot({path:path.join(fixture,'initial.png')});
    await page.locator('.platform-more').first().click();
    await page.locator('.platform-details').first().click();
    await page.locator('#map-tools-trigger').click();await page.locator('#map-direction').selectOption('horizontal');
    if(await page.locator('#zoom-in').isEnabled())await page.locator('#zoom-in').click();
    await page.locator('#map-tools-trigger').click();
    await page.locator('#topology-viewport').evaluate(el=>{el.scrollLeft=140;el.scrollTop=70;});
    await delay(300);
    async function state(){return page.evaluate(()=>({
      details:[...document.querySelectorAll('.platform-details')].map(el=>el.getAttribute('aria-expanded')),
      expanded:[...document.querySelectorAll('.route-platform')].map(el=>el.getAttribute('aria-expanded')),
      more:[...document.querySelectorAll('.platform-more')].map(el=>el.getAttribute('aria-expanded')),
      // Relative age text advances with the real clock. Retain visible identity
      // and heading here; full backend chat rows are compared separately.
      chats:[...document.querySelectorAll('.platform-chat')].map(el=>({id:el.dataset.objectId,heading:el.querySelector('.task-heading').textContent})),
      zoom:document.querySelector('#zoom-level')?.textContent||document.querySelector('#zoom-reset')?.textContent,
      scroll:{left:document.querySelector('#topology-viewport').scrollLeft,top:document.querySelector('#topology-viewport').scrollTop},
      inspectorHidden:document.querySelector('#selection-details').hidden,
    }));}
    async function selectionObservation(){return page.evaluate(()=>({
      current:[...document.querySelectorAll('[aria-current="true"]')].map(el=>el.dataset.objectId||el.id||el.tagName),
      focused:{tag:document.activeElement.tagName,id:document.activeElement.id,object:document.activeElement.dataset?.objectId||null},
    }));}
    const selectionBefore=await selectionObservation();
    const before=await state();assert.ok(before.details.includes('true'),'An expanded workspace is required');
    const beforeObservation=await page.evaluate(()=>window.__receivedDock.at(-1)?.observation_revision||0);
    await page.screenshot({path:path.join(fixture,'before.png')});
    await stop(owner);
    // This same MCP request must cause a replacement. Raw Hello never starts one.
    const recovered=await call('devmap_read_map');
    const afterOwner=await waitOwner(w=>w.owner_pid!==beforeOwner.owner_pid&&w.owner_instance!==nonce);
    assert.equal(mcp.pid,originalPid);assert.equal(mcp.exitCode,null);
    assert.equal(recovered.repository_id,initial.repository_id);assert.equal(recovered.current_worktree_id,initial.current_worktree_id);
    assert.ok(initial.task_observation?.observed_at,'Initial task observation must exist');
    assert.equal(initial.task_observation.complete,true);
    assert.equal(typeof initial.task_inventory_synced_at,'string');
    assert.deepEqual(recovered.task_observation,initial.task_observation,'Reconnect changed retained inventory observation');
    assert.equal(recovered.task_inventory_synced_at,initial.task_inventory_synced_at);
    const taskRows=model=>model.lanes.flatMap(lane=>lane.chats);
    assert.equal(taskRows(initial).length,tasks.length);
    assert.deepEqual(taskRows(recovered),taskRows(initial),'Reconnect changed accepted task rows');
    assert.ok(recovered.revision>=opened.revision);
    await page.waitForFunction(({revision,observation,previous})=>{
      const match=document.querySelector('#revision')?.textContent.match(/^Revision (\d+)/);
      const event=window.__receivedDock.at(-1);
      return match&&Number(match[1])>=revision&&event?.observation_revision>=observation&&event.observation_revision>previous;
    },{revision:recovered.revision,observation:recovered.observation_revision,previous:beforeObservation},{timeout:15000});
    assert.deepEqual(await state(),before,'Browser state changed across actual owner replacement');
    assert.equal(page.url(),opened.url);assert.equal(await page.locator('#snapshot-feedback').isVisible(),false);
    const reused=await call('devmap_open_map',{surface:'browser'});assert.equal(reused.url,opened.url);assert.equal(reused.reused,true);
    const selectionAfter=await selectionObservation();
    assert.ok(selectionBefore.current.length>0,'Restart scenario must begin with a visible selection');
    assert.deepEqual(selectionAfter,selectionBefore,'Selection or keyboard focus changed across actual owner replacement');
    assert.deepEqual(errors,[]);await page.screenshot({path:path.join(fixture,'after.png')});
    const report={scope:'actual_shared_owner_restart_browser',passed:true,candidate_sha256:build,fixture,
      mcp_pid:originalPid,owner_before:beforeOwner,owner_after:afterOwner,ui_state:before,
      selection_observation:{before:selectionBefore,after:selectionAfter,
        retained:JSON.stringify(selectionBefore)===JSON.stringify(selectionAfter)},
      revisions:{initial:initial.revision,opened:opened.revision,recovered:recovered.revision},
      note:'Real HTTP/SSE and persistent MCP: expanded cards, chats, zoom, scroll, aria-current selection and keyboard focus retention are required. This is not a pixel-parity or real host activation test. Replacement owner expires after clients disconnect.'};
    fs.writeFileSync(path.join(fixture,'report.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
  } finally {
    if(browser)await browser.close();for(const entry of pending.values())clearTimeout(entry.timer);pending.clear();
    if(lines)lines.close();await stop(mcp);await stop(owner);
  }
}
main().catch(error=>{console.error(error);process.exitCode=1;});
