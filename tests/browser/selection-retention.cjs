'use strict';
// Real-browser regression for age redraw and SSE refresh; not an owner restart.
const fs=require('node:fs'),path=require('node:path'),http=require('node:http'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const {execFileSync}=require('node:child_process');
const root=path.resolve(__dirname,'../..');
const modules='C:/Users/user/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules';
const {chromium}=require(path.join(modules,'playwright'));
const {PNG}=require(path.join(modules,'pngjs'));
const sha=b=>crypto.createHash('sha256').update(b).digest('hex');
function different(a,b){const x=PNG.sync.read(a),y=PNG.sync.read(b);assert.equal(x.width,y.width);assert.equal(x.height,y.height);let count=0;for(let i=0;i<x.data.length;i+=4)if([0,1,2,3].some(c=>Math.abs(x.data[i+c]-y.data[i+c])>2))count++;return count;}
async function main(){
 const fixture=path.resolve(process.argv[2]);assert(fixture.startsWith(path.join(root,'target/verification')+path.sep));
 const initial=JSON.parse(fs.readFileSync(fixture));
 const run=fs.mkdtempSync(path.join(root,'target/verification/selection-browser-'));
 const core=fs.readFileSync(path.join(root,'assets/metro-core.js'),'utf8');
 const old=execFileSync('git',['show','520683a:assets/dock.html'],{cwd:root,encoding:'utf8',maxBuffer:1024*1024});
 const candidate=fs.readFileSync(path.join(root,'assets/dock.html'),'utf8');
 function assemble(html){const tail=html.lastIndexOf('})();');assert(tail>0);return (html.slice(0,tail)+'window.__selectionRefresh=refreshDynamicState;'+html.slice(tail)).replace('/* DEVMAP_METRO_CORE */',core).replaceAll('DEVMAP_RUNTIME_VERSION','test').replaceAll('DEVMAP_BUILD_REVISION','test');}
 let html,model;const streams=new Set();
 const server=http.createServer((req,res)=>{const p=new URL(req.url,'http://localhost').pathname;if(p==='/api/v1/dock/snapshot'){res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify(model));}else if(p==='/api/v1/dock/events'){res.writeHead(200,{'Content-Type':'text/event-stream'});res.write(': ready\n\n');streams.add(res);req.on('close',()=>streams.delete(res));}else{res.writeHead(200,{'Content-Type':'text/html; charset=utf-8'});res.end(html);}});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 let browser;const report={scope:'fixture HTTP/SSE and direct age redraw; no native runtime replacement',run,pixel_policy:{maximum_channel_difference:2,excluded_regions:[],reference:'existing stage1 browser comparison tolerance'},source_sha256:{baseline:sha(old),candidate:sha(candidate),core:sha(core)},cases:[],errors:[],passed:false};
 try{
  browser=await chromium.launch({headless:true,executablePath:'C:/Program Files/Google/Chrome/Application/chrome.exe',args:['--disable-gpu']});
  for(const width of [1280,560,360]){
   const pictures={};
   for(const variant of ['baseline','candidate']){
    html=assemble(variant==='baseline'?old:candidate);model=structuredClone(initial);
    const context=await browser.newContext({viewport:{width,height:900},deviceScaleFactor:1,colorScheme:'light',reducedMotion:'reduce'}),page=await context.newPage();
    try{
     page.on('pageerror',e=>report.errors.push(String(e)));await page.clock.install({time:new Date('2026-09-13T10:00:00Z')});await page.clock.pauseAt(new Date('2026-09-13T10:00:00Z'));
     await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.locator('.route-platform').first().waitFor();await page.evaluate(()=>document.fonts.ready);
     await page.locator('.platform-details').first().click();await page.mouse.move(0,0);
     const state=()=>page.evaluate(()=>({selected:[...document.querySelectorAll('[aria-current="true"]')].map(n=>n.dataset.objectId),focus:document.activeElement?.dataset.objectId,expanded:[...document.querySelectorAll('.route-platform')].map(n=>n.getAttribute('aria-expanded')),details:[...document.querySelectorAll('.platform-details')].map(n=>n.getAttribute('aria-expanded')),scroll:[document.querySelector('#topology-viewport').scrollLeft,document.querySelector('#topology-viewport').scrollTop]}));
     const before=await state();assert.equal(before.selected.length,1);
     const capture=async name=>{const b=await page.screenshot({animations:'disabled'});fs.writeFileSync(path.join(run,`${width}-${variant}-${name}.png`),b,{flag:'wx'});return b;};
     const beforeImage=await capture('before');await page.evaluate(()=>window.__selectionRefresh());const after=await state(),afterImage=await capture('after-age');
     assert.deepEqual({...after,selected:[]},{...before,selected:[]});
     const pixels=different(beforeImage,afterImage);
     if(variant==='baseline'){assert.equal(after.selected.length,0);assert(pixels>0);}else{assert.deepEqual(after,before);assert.equal(pixels,0);}
     model.revision++;model.observation_revision++;assert.equal(streams.size,1);for(const stream of streams)stream.write(`event: dock\ndata: ${JSON.stringify(model)}\n\n`);
     await page.waitForFunction(revision=>document.querySelector('#revision').textContent.startsWith(`Revision ${revision} ·`),model.revision);
     const refreshed=await state();assert.deepEqual({...refreshed,selected:[]},{...before,selected:[]});
     assert.deepEqual(refreshed.selected,variant==='candidate'?before.selected:[]);await capture('after-sse');
     report.cases.push({width,variant,before,after,refreshed,age_changed_pixels:pixels});pictures[variant]=beforeImage;
    }finally{await context.close();}
   }
   assert.equal(different(pictures.baseline,pictures.candidate),0,'Pre-refresh appearance changed');
  }
  assert.deepEqual(report.errors,[]);report.passed=true;
 }catch(e){report.errors.push(String(e.stack||e));}
 finally{if(browser)await browser.close();for(const stream of streams)stream.end();await new Promise(resolve=>server.close(resolve));fs.writeFileSync(path.join(run,'report.json'),JSON.stringify(report,null,2),{flag:'wx'});}
 console.log(JSON.stringify({run,passed:report.passed,errors:report.errors}));process.exitCode=report.passed?0:1;
}
main().catch(e=>{console.error(e);process.exitCode=1;});
