// Real-browser frontend compatibility gate. A frozen fixture run proves only
// rendering/interaction compatibility; SQL migration is a separate Rust gate.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const http = require('node:http');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');
const { createRequire } = require('node:module');
const { pathToFileURL } = require('node:url');

const root = path.resolve(__dirname, '../..');
const output = process.env.DEVMAP_BROWSER_OUTPUT || path.join(root, 'target/verification/sqlite-browser');
const modules = process.env.CODEX_DOC_MODULES || 'C:/Users/user/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules';
const { chromium } = require(path.join(modules, 'playwright'));
const { PNG } = require(path.join(modules, 'pngjs'));
const baselineRef = '520683a';
const stamp = '2026-09-08T16:00:00.000Z';
const fixedTime = Date.parse(stamp);
const sha256 = value => crypto.createHash('sha256').update(value).digest('hex').toUpperCase();

function referenceAsset(file) {
  return execFileSync('git', ['show', `${baselineRef}:${file}`], {cwd:root, maxBuffer:1024*1024});
}

function fixture() {
  const file = path.join(root, 'tests/dock_renderer.cjs');
  const localRequire = createRequire(file);
  const module = {exports:{}};
  class Clock extends Date { constructor(...args) { super(...(args.length ? args : [fixedTime])); } static now() { return fixedTime; } }
  const sandbox = {module, exports:module.exports, __dirname:path.dirname(file), console, structuredClone, Date:Clock,
    require:name => name === 'node:test' ? () => {} : localRequire(name)};
  vm.runInNewContext(fs.readFileSync(file,'utf8'),sandbox,{filename:file});
  return JSON.parse(JSON.stringify(module.exports.snapshot()));
}

function assemble(template, core) {
  assert.equal(template.split('/* DEVMAP_METRO_CORE */').length,2);
  const rustLines = text => text.replace(/\r\n/g,'\n').replace(/\n$/,'').split('\n').map(x=>x.trimStart()).join('\n');
  return rustLines(template).replaceAll('DEVMAP_RUNTIME_VERSION','0.1.1')
    .replaceAll('DEVMAP_BUILD_REVISION','db696768-dirty')
    .replace('/* DEVMAP_METRO_CORE */',rustLines(core));
}

async function main() {
  fs.mkdirSync(output,{recursive:true});
  const {default:pixelmatch} = await import(pathToFileURL(path.join(modules,'pixelmatch/index.js')).href);
  const baselineAssets = {html:referenceAsset('assets/dock.html'), core:referenceAsset('assets/metro-core.js')};
  const candidateAssets = {html:fs.readFileSync(path.join(root,'assets/dock.html')),core:fs.readFileSync(path.join(root,'assets/metro-core.js'))};
  // Git may normalize line endings; the semantic resource bytes assembled by Rust must match exactly.
  const baselineHtml = assemble(baselineAssets.html.toString('utf8'),baselineAssets.core.toString('utf8'));
  const candidateHtml = assemble(candidateAssets.html.toString('utf8'),candidateAssets.core.toString('utf8'));
  assert.equal(candidateHtml,baselineHtml,'Current frontend differs from the verified installed baseline');
  const baselineSnapshot = process.env.DEVMAP_BASELINE_SNAPSHOT ? JSON.parse(fs.readFileSync(process.env.DEVMAP_BASELINE_SNAPSHOT,'utf8')) : fixture();
  const candidateSnapshot = process.env.DEVMAP_CANDIDATE_SNAPSHOT ? JSON.parse(fs.readFileSync(process.env.DEVMAP_CANDIDATE_SNAPSHOT,'utf8')) : structuredClone(baselineSnapshot);
  assert.equal(Boolean(process.env.DEVMAP_BASELINE_SNAPSHOT),Boolean(process.env.DEVMAP_CANDIDATE_SNAPSHOT),'Provide both backend snapshots or neither');
  const sameSnapshotPath = process.env.DEVMAP_BASELINE_SNAPSHOT && path.resolve(process.env.DEVMAP_BASELINE_SNAPSHOT)===path.resolve(process.env.DEVMAP_CANDIDATE_SNAPSHOT);
  const scope = sameSnapshotPath ? 'same_snapshot_control' : process.env.DEVMAP_BASELINE_SNAPSHOT ? 'backend_snapshot_pair' : 'frontend_frozen_fixture';
  let servedHtml, snapshot;
  const streams = new Set();
  const server = http.createServer((req,res)=>{
    const pathname = new URL(req.url,'http://localhost').pathname;
    if(pathname==='/api/v1/dock/snapshot') {res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify(snapshot));}
    else if(pathname==='/api/v1/dock/events') {res.writeHead(200,{'Content-Type':'text/event-stream','Cache-Control':'no-cache'});res.write(': connected\n\n');streams.add(res);req.on('close',()=>streams.delete(res));}
    else {res.writeHead(200,{'Content-Type':'text/html; charset=utf-8'});res.end(servedHtml);}
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const url = `http://127.0.0.1:${server.address().port}/`;
  const browser = await chromium.launch({headless:true,args:['--disable-gpu'],executablePath:process.env.CODEX_DOC_BROWSER||'C:/Program Files/Google/Chrome/Application/chrome.exe'});
  const reports = [], captures = new Map();
  try {
    for(const viewport of [{width:1280,height:900},{width:560,height:900},{width:360,height:800}]) {
      const variantStates = {};
      for(const variant of ['baseline','control','candidate']) {
        servedHtml = variant!=='candidate'?baselineHtml:candidateHtml;
        snapshot = structuredClone(variant!=='candidate'?baselineSnapshot:candidateSnapshot);
        const context = await browser.newContext({viewport,deviceScaleFactor:1,colorScheme:'light',reducedMotion:'reduce'});
        const page=await context.newPage(), errors=[];
        await page.clock.install({time:new Date(fixedTime-1000)});
        await page.clock.pauseAt(new Date(fixedTime));
        page.on('pageerror',error=>errors.push(error.message));
        await page.goto(url);
        await page.locator('.route-platform').first().waitFor({state:'visible'});
        await page.evaluate(()=>document.fonts.ready);
        await page.waitForTimeout(200);
        const states=[];
        async function capture(name) {
          await page.mouse.move(0,0);
          await page.clock.runFor(32);
          const state=await page.evaluate(()=>({
            platforms:document.querySelectorAll('.route-platform').length,
            chatTitles:[...document.querySelectorAll('.platform-chat')].map(x=>x.textContent),
            details:[...document.querySelectorAll('.platform-details')].map(x=>x.getAttribute('aria-expanded')),
            more:[...document.querySelectorAll('.platform-more')].map(x=>x.getAttribute('aria-expanded')),
            zoom:document.querySelector('#zoom-level')?.textContent||document.querySelector('#zoom-reset')?.textContent,
            scroll:{left:document.querySelector('#topology-viewport').scrollLeft,top:document.querySelector('#topology-viewport').scrollTop},
            inspector:document.querySelector('#selection-details').hidden,
            horizontalOverflow:document.documentElement.scrollWidth>innerWidth,
          }));
          states.push({name,...state});
          const file=path.join(output,`${viewport.width}-${variant}-${name}.png`);
          const bytes=await page.screenshot({path:file,animations:'disabled',timeout:15000});
          captures.set(`${viewport.width}-${variant}-${name}`,bytes);
        }
        await capture('initial');
        const more=page.locator('.platform-more').first();
        if(await more.count()){await more.click();await capture('expanded-chats');}
        await page.locator('.platform-details').first().click();
        await capture('workspace-details');
        await page.locator('#map-tools-trigger').click();
        await page.locator('#map-direction').selectOption('horizontal');
        if(await page.locator('#zoom-in').isEnabled()) await page.locator('#zoom-in').click();
        await page.locator('#map-tools-trigger').click();
        await page.locator('#topology-viewport').evaluate(el=>{el.scrollLeft=140;el.scrollTop=70;});
        await capture('zoom-pan');
        snapshot.observation_revision+=1;
        snapshot.revision+=1;
        assert.equal(streams.size,1,'Expected one connected SSE consumer');
        for(const stream of streams) stream.write(`event: dock\ndata: ${JSON.stringify(snapshot)}\n\n`);
        await page.waitForFunction(revision=>document.querySelector('#revision').textContent.startsWith(`Revision ${revision} ·`),snapshot.revision);
        await capture('refresh-retained');
        assert.equal(await page.locator('#snapshot-feedback').isVisible(),false,'Snapshot was rejected');
        assert.deepEqual(errors,[],`${variant} browser errors`);
        variantStates[variant]=states;
        await context.close();
      }
      assert.deepEqual(variantStates.control,variantStates.baseline,'Same-source control interaction mismatch');
      assert.deepEqual(variantStates.candidate,variantStates.baseline,`Interactions changed at ${viewport.width}px`);
      for(const comparedVariant of ['control','candidate']) for(const state of variantStates.baseline) {
        const a=PNG.sync.read(captures.get(`${viewport.width}-baseline-${state.name}`));
        const b=PNG.sync.read(captures.get(`${viewport.width}-${comparedVariant}-${state.name}`));
        const diff=new PNG({width:a.width,height:a.height});
        const pixels=pixelmatch(a.data,b.data,diff.data,a.width,a.height,{threshold:0,includeAA:true});
        let maxChannelDelta=0;
        for(let i=0;i<a.data.length;i++) maxChannelDelta=Math.max(maxChannelDelta,Math.abs(a.data[i]-b.data[i]));
        const visiblePixels=pixelmatch(a.data,b.data,null,a.width,a.height,{threshold:0.01,includeAA:true});
        fs.writeFileSync(path.join(output,`${viewport.width}-${comparedVariant}-diff-${state.name}.png`),PNG.sync.write(diff));
        // Same-source investigation localized rounding to border pixels, with
        // at most two RGB levels. Apply the same fixed bound to the reference
        // repeat control and candidate; retain all raw differences, no masks.
        assert.equal(visiblePixels,0,`Visible pixel mismatch at ${viewport.width}/${state.name}`);
        assert.ok(maxChannelDelta<=2,`Channel delta exceeds measured raster rounding at ${viewport.width}/${state.name}: ${maxChannelDelta}`);
        reports.push({compared_variant:comparedVariant,width:viewport.width,state:state.name,raw_different_pixels:pixels,visible_different_pixels:visiblePixels,max_channel_delta:maxChannelDelta});
      }
    }
    // Negative control: prove pixel comparison is capable of detecting a changed image.
    const example=PNG.sync.read(captures.values().next().value);
    const changed=Buffer.from(example.data);changed[0]^=255;
    assert.ok(pixelmatch(example.data,changed,null,example.width,example.height,{threshold:0.01,includeAA:true})>0);
    const evidence={scope,reference_commit:baselineRef,resource_sha256:sha256(baselineHtml),fixed_time:stamp,
      baseline_snapshot_file:process.env.DEVMAP_BASELINE_SNAPSHOT||null,candidate_snapshot_file:process.env.DEVMAP_CANDIDATE_SNAPSHOT||null,
      excluded_pixels:0,clock_frozen:true,build_metadata_fixed:true,animations_disabled:true,max_allowed_channel_delta:2,negative_control:'pass',reports,
      note:'Fixture rendering is not evidence that the SQLite backend or a real host lifecycle has passed. Backend/migration gates are separate.'};
    fs.writeFileSync(path.join(output,'report.json'),JSON.stringify(evidence,null,2));
    console.log(JSON.stringify(evidence));
  } finally {
    await browser.close();for(const stream of streams) stream.end();
    server.closeAllConnections();await new Promise(resolve=>server.close(resolve));
  }
}
main().catch(error=>{console.error(error);process.exitCode=1;});
