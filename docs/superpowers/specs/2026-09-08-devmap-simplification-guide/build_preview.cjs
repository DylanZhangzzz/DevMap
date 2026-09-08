// Generate a self-contained reading page and inspect it with the local browser.
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const crypto = require('node:crypto');
const child = require('node:child_process');
const modules = process.env.CODEX_DOC_MODULES || 'C:/Users/user/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules';
const { marked } = require(path.join(modules, 'marked'));
const { chromium } = require(path.join(modules, 'playwright'));
const root = __dirname;
const pages = JSON.parse(fs.readFileSync(path.join(root, 'diagrams.json'), 'utf8')).pages;
const docs = [
  ['overview', '设计总览', 'README.md'],
  ['study', 'CodeGraph 实现研究', '01-codegraph-study.md'],
  ['framework', 'DevMap 技术框架', '02-technical-framework.md'],
  ['roadmap', '技术路线图', '03-roadmap.md'],
];
const fileToId = Object.fromEntries(docs.map(([id, , file]) => [file, id]));
const escape = s => s.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
const broken = [];
function renderDoc(file) {
  const source = fs.readFileSync(path.join(root, file), 'utf8');
  for (const match of source.matchAll(/\]\(([^)]+)\)/g)) {
    const target = match[1];
    if (target !== 'guide.html' && !/^(https?:|#)/.test(target) && !fs.existsSync(path.join(root, target))) broken.push({file, target});
  }
  let html = marked.parse(source);
  html = html.replace(/href="([^"]+\.md)"/g, (all, file) => fileToId[file] ? `href="#${fileToId[file]}"` : all);
  html = html.replace(/<img src="([^"]+\.svg)" alt="([^"]*)">/g, (all, file, alt) => {
    const svg = fs.readFileSync(path.join(root, file)).toString('base64');
    return `<a class="figure-link" href="${file}" title="打开全尺寸架构图"><img src="data:image/svg+xml;base64,${svg}" alt="${alt}"></a>`;
  });
  return html;
}

async function main() {
  const sections = docs.map(([id, title, file]) => `<section id="${id}" aria-label="${title}">${renderDoc(file)}</section>`).join('\n');
  if (broken.length) throw Error(JSON.stringify(broken));
  const html = `<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>DevMap 简化设计指导方针</title><style>
*{box-sizing:border-box}html{scroll-behavior:smooth;scroll-padding-top:24px}body{margin:0;background:#f5f7fa;color:#24364b;font-family:"Microsoft YaHei",system-ui,sans-serif;line-height:1.85;font-size:16px}nav{position:fixed;inset:0 auto 0 0;width:250px;padding:36px 24px;background:#152d43;color:#fff;overflow:auto}nav .brand{font-size:24px;font-weight:700;letter-spacing:.04em}nav p{font-size:13px;color:#bbcada;line-height:1.7}nav a{display:block;margin:12px 0;padding:8px 0;color:#e5eef6;text-decoration:none}nav a:hover{color:#8bddba}nav hr{border:0;border-top:1px solid #ffffff30;margin:24px 0}main{margin-left:250px;padding:40px 48px;max-width:1520px}section{padding:32px 38px 46px;margin-bottom:34px;background:#fff;border:1px solid #dde4ec;border-radius:10px}h1{font-size:30px;line-height:1.4;letter-spacing:-.02em;color:#15334b;margin:0 0 24px}h2{font-size:23px;line-height:1.5;margin:38px 0 16px;color:#176346;border-bottom:1px solid #e2e9ee;padding-bottom:10px}h3{font-size:18px}p{margin:14px 0}a{color:#176b58;text-underline-offset:3px;overflow-wrap:anywhere}li{margin:8px 0}table{width:100%;border-collapse:collapse;font-size:14px;margin:22px 0;display:block;overflow-x:auto}th,td{border-bottom:1px solid #dce4eb;padding:12px 14px;vertical-align:top;min-width:110px}th{background:#edf3f7;text-align:left}code{font-family:Consolas,monospace;font-size:.92em;background:#eef3f7;padding:2px 5px;border-radius:4px;overflow-wrap:anywhere}pre{overflow:auto;background:#112b40;color:#dcebf4;border-radius:8px;padding:20px;line-height:1.6}pre code{background:transparent;padding:0}img{display:block;width:100%;height:auto;border:1px solid #dce4eb;border-radius:8px;margin:25px 0}.figure-link{display:block}.stamp{font-size:12px;letter-spacing:.1em;color:#86d2b1}strong{color:#173d50}@media(max-width:800px){nav{position:static;width:auto;padding:20px}nav a{display:inline-block;margin:2px 16px 2px 0}nav hr,nav p{display:none}main{margin-left:0;padding:16px}section{padding:24px 18px}h1{font-size:25px}h2{font-size:21px}}@media print{nav{display:none}main{margin:0;padding:0}section{border:0;page-break-before:always}a{color:inherit}table{display:table;font-size:11px}h1{font-size:24px}h2{break-after:avoid}tr,img{break-inside:avoid}}
</style></head><body><nav aria-label="文档导航"><div class="brand">DevMap</div><p class="stamp">SIMPLIFICATION GUIDE</p><p>简化设计指导方针<br>2026-09-08 · v0.1 评审稿</p>${docs.map(([id,title])=>`<a href="#${id}">${title}</a>`).join('')}<hr><a href="devmap-simplification.drawio">四页可编辑架构图</a><a href="04-roadmap.svg">全尺寸路线图</a><p>研究基线<br>CodeGraph · 195888d<br>DevMap main · db69676</p></nav><main>${sections}</main></body></html>`;
  fs.writeFileSync(path.join(root, 'guide.html'), html);
  const browser = await chromium.launch({headless: true, args:['--disable-gpu'], executablePath: process.env.CODEX_DOC_BROWSER || 'C:/Program Files/Google/Chrome/Application/chrome.exe'});
  const checks = {checked_at: new Date().toISOString(), product_code_changed: false, codegraph_runtime_tested: false, drawio_editor_roundtrip_tested: false, local_links: 'pass', diagrams: [], browser: {}};
  try {
    const context = await browser.newContext({viewport:{width:1440,height:1050}, deviceScaleFactor:1});
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    for (const diagram of pages) {
      await page.setViewportSize({width:diagram.width,height:diagram.height});
      await page.setContent('<!doctype html><html><head><meta charset="utf-8"></head><body style="margin:0">'+fs.readFileSync(path.join(root, diagram.file+'.svg'),'utf8')+'</body></html>');
      await page.evaluate(()=>document.fonts.ready);
      const bounds = await page.evaluate(() => {
        const svg = document.querySelector('svg');
        const box = svg.viewBox.baseVal;
        return [...document.querySelectorAll('text')].filter(el => {
          const r=el.getBBox();return r.x<0 || r.y<0 || r.x+r.width>box.width || r.y+r.height>box.height;
        }).map(el=>el.textContent);
      });
      if (bounds.length) throw Error('SVG clipped text '+JSON.stringify(bounds));
      await page.screenshot({path:path.join(root,diagram.file+'.png'),clip:{x:0,y:0,width:diagram.width,height:diagram.height},timeout:15000});
      checks.diagrams.push({page:diagram.id,svg_render:'pass',text_in_canvas:'pass'});
    }
    await page.setViewportSize({width:1440,height:1000});
    await page.goto(pathToFileURL(path.join(root, 'guide.html')).href);
    await page.evaluate(()=>document.fonts.ready);
    const desktop = await page.evaluate(()=>({images:[...document.images].every(x=>x.complete&&x.naturalWidth>0),sections:document.querySelectorAll('main section').length,overflow:document.documentElement.scrollWidth>innerWidth}));
    await page.screenshot({path:path.join(root,'guide-desktop.png')});
    await page.getByRole('link',{name:'技术路线图',exact:true}).first().click();
    await page.waitForTimeout(450);
    const anchor = await page.evaluate(()=>location.hash);
    await page.setViewportSize({width:390,height:844});
    await page.goto(pathToFileURL(path.join(root, 'guide.html')).href);
    await page.evaluate(()=>document.fonts.ready);
    const mobile = await page.evaluate(()=>({overflow:document.documentElement.scrollWidth>innerWidth,images:[...document.images].every(x=>x.complete&&x.naturalWidth>0)}));
    await page.screenshot({path:path.join(root,'guide-mobile.png')});
    checks.browser = {desktop,mobile,navigation_anchor:anchor,errors};
    if (!desktop.images || desktop.sections!==4 || desktop.overflow || mobile.overflow || !mobile.images || anchor!=='#roadmap' || errors.length) throw Error('Browser QA failed '+JSON.stringify(checks.browser));
  } finally { await browser.close(); }
  checks.artifacts = fs.readdirSync(root).filter(f=>/\.(md|svg|drawio|html)$/.test(f)).map(file=>({file,bytes:fs.statSync(path.join(root,file)).size,sha256:crypto.createHash('sha256').update(fs.readFileSync(path.join(root,file))).digest('hex')}));
  fs.writeFileSync(path.join(root,'verification.json'), JSON.stringify(checks,null,2));
  console.log(JSON.stringify({local_links:checks.local_links,diagrams:checks.diagrams.length,browser:checks.browser}));
}
main().catch(error=>{console.error(error);process.exitCode=1;});
