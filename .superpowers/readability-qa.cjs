const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('C:/Users/user/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright');
const root = path.resolve(__dirname, '..');
const { snapshot } = require(path.join(root, 'tests/dock_renderer.cjs'));
const out = path.join(root, 'docs/audits/assets/2026-09-05-readability');
fs.mkdirSync(out, { recursive: true });
const value = snapshot();
value.workspace_facts[0].working_state = 'dirty';
let html = fs.readFileSync(path.join(root, 'assets/dock.html'), 'utf8').split(/\r?\n/).map(s => s.trimStart()).join('\n').replace('/* DEVMAP_METRO_CORE */', () => fs.readFileSync(path.join(root, 'assets/metro-core.js'), 'utf8'));
html = html.replace('if (transport === "mcp") initializeMcp(); else { fetchSnapshot(); connectEvents(); } scheduleAge();', 'window.qaAccept = acceptSnapshot;');
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const errors = [], results = [];
  const page = await browser.newPage();
  page.on('pageerror', error => errors.push(error.message));
  for (const width of [560, 360, 1280]) {
    await page.setViewportSize({ width, height: 800 });
    await page.setContent(html);
    await page.evaluate(value => window.qaAccept(value), value);
    await page.screenshot({ path: path.join(out, `current-${width}.png`) });
    results.push(await page.evaluate(() => ({ mode: 'current', width: innerWidth, scale: document.getElementById('relationship-map').dataset.scale, overflow: document.documentElement.scrollWidth > innerWidth })));
    await page.locator('#zoom-fit').click();
    await page.screenshot({ path: path.join(out, `overview-${width}.png`) });
    results.push(await page.evaluate(() => {
      const boxes = [...document.querySelectorAll('.overview-marker')].map(n => n.getBoundingClientRect());
      return { mode: 'overview', width: innerWidth, markers: boxes.length, overlaps: boxes.flatMap((a,i)=>boxes.slice(i+1).filter(b => a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top)).length, overflow: document.documentElement.scrollWidth > innerWidth };
    }));
    const shared = page.locator('.overview-marker').filter({ hasText: 'workspaces at this commit' }).first();
    await shared.click();
    await page.screenshot({ path: path.join(out, `expanded-${width}.png`) });
    await page.locator('.workspace-choice').nth(1).click();
    if (await page.locator('#relationship-map').getAttribute('data-scale') !== '1') throw new Error('Workspace drill-down failed');
  }
  await browser.close();
  fs.writeFileSync(path.join(out, 'browser-checks.json'), JSON.stringify({ fixture: 'Synthetic six-workspace snapshot; not live task state', errors, results }, null, 2));
  console.log(JSON.stringify({ errors, results, out }));
  if (errors.length || results.some(r => r.overflow || r.overlaps)) process.exitCode = 1;
})().catch(e => { console.error(e); process.exitCode = 1; });
