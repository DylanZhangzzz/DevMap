// Explicit, ephemeral real Codex CLI MCP smoke. No global config/plugin edits.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');
const {execFileSync, spawn} = require('node:child_process');
const root = path.resolve(__dirname, '../..');
function assess(events, code, expired) {
  const completed = events.filter(e => e.type === 'item.completed').map(e => e.item);
  const calls = completed.filter(i => i?.type === 'mcp_tool_call');
  const maps = calls.filter(c => c.tool === 'devmap_read_map' && c.server === 'devmap_candidate');
  const plans = calls.filter(c => c.tool === 'devmap_set_route_plan' && c.server === 'devmap_candidate');
  const expected = calls.map(c => c.tool).join(',') === 'devmap_read_map,devmap_set_route_plan,devmap_read_map'
    && calls.every(c => c.server === 'devmap_candidate')
    && !completed.some(i => ['command_execution', 'file_change', 'web_search'].includes(i?.type));
  const successful = calls.every(c => c.status === 'completed' && !c.error && !c.result?.isError && !c.result?.is_error);
  const first = maps[0]?.result?.structured_content;
  const accepted = plans[0]?.result?.structured_content;
  const readback = maps.at(-1)?.result?.structured_content;
  const route = readback?.route_plans?.find(r => r.route_id === accepted?.route_id);
  const roundtrip = first?.schema_version === 'devmap/dock/4' && readback?.schema_version === 'devmap/dock/4'
    && accepted?.goal === 'Actual Codex host acceptance fixture' && route?.goal === accepted.goal
    && route?.revision === 1 && route?.worktree_id === first.current_worktree_id
    && route?.repository_id === first.repository_id && readback.repository_id === first.repository_id;
  return {scope: 'actual_codex_cli_mcp_smoke', code, expired,
    thread_id: events.find(e => e.type === 'thread.started')?.thread_id,
    calls: calls.length, map_calls: maps.length, plan_calls: plans.length,
    expected, successful, roundtrip: Boolean(roundtrip),
    passed: code === 0 && !expired && expected && successful && Boolean(roundtrip)};
}
if (process.env.DEVMAP_RECHECK_HOST_FIXTURE) {
  const fixture = fs.realpathSync(process.env.DEVMAP_RECHECK_HOST_FIXTURE);
  const rel = path.relative(path.join(root, 'target/verification'), fixture);
  assert.ok(rel && !rel.startsWith('..') && !path.isAbsolute(rel));
  assert.equal(JSON.parse(fs.readFileSync(path.join(fixture, 'manifest.json'))).scope, 'actual_codex_cli_mcp_smoke');
  const previous = JSON.parse(fs.readFileSync(path.join(fixture, 'report.json')));
  const events = fs.readFileSync(path.join(fixture, 'events.jsonl'), 'utf8').trim().split(/\r?\n/).map(JSON.parse);
  const report = assess(events, previous.code, previous.expired);
  const invalid = structuredClone(events);
  const last = invalid.filter(e => e.type === 'item.completed' && e.item?.type === 'mcp_tool_call').at(-1);
  last.item.result.structured_content.route_plans[0].goal = 'wrong readback';
  assert.equal(assess(invalid, 0, false).passed, false, 'Readback negative control');
  fs.writeFileSync(path.join(fixture, 'report-validated.json'), JSON.stringify(report, null, 2), {flag: 'wx'});
  console.log(JSON.stringify(report));
  process.exitCode = report.passed ? 0 : 1;
  return;
}
const candidate = fs.realpathSync(process.env.DEVMAP_CANDIDATE_EXE);
const codex = fs.realpathSync(process.env.DEVMAP_CODEX_EXE);
const relative = path.relative(path.join(root, 'target'), candidate);
assert.ok(relative && !relative.startsWith('..') && !path.isAbsolute(relative), 'Candidate must be in checkout target');
const output = fs.mkdtempSync(path.join(root, 'target/verification/codex-host-'));
const source = path.join(output, 'repository');
fs.mkdirSync(source);
function git(...args) { execFileSync('git', args, {cwd: source, stdio: 'pipe'}); }
git('init', '-b', 'main');
git('config', 'user.name', 'Isolated host fixture');
git('config', 'user.email', 'fixture@example.invalid');
git('commit', '--allow-empty', '-m', 'Host fixture base');
execFileSync(candidate, ['storage', 'migrate', '--source', source, '--backup-dir', path.join(output, 'frozen')], {cwd: source, timeout: 30000, stdio: 'pipe'});
const prompt = 'This is an isolated MCP integration test. Use only the devmap_candidate MCP server. '
  + 'First call devmap_read_map with no arguments. Then call devmap_set_route_plan using the returned current_worktree_id, '
  + 'request_id "actual-host-route-1", expected_revision 0, goal "Actual Codex host acceptance fixture", source "isolated actual host smoke". '
  + 'Then call devmap_read_map again and verify that exact route goal is present. '
  + 'Do not use shell, edit source files, perform Git operations, browse, or invoke other servers. '
  + 'Return PASS and the route_id only after the actual tool results prove the round trip; otherwise report the failure.';
const configs = {
  'approval_policy': 'never',
  'mcp_servers.devmap_candidate.command': candidate,
  'mcp_servers.devmap_candidate.args': ['mcp', '--source', source],
  'mcp_servers.devmap_candidate.required': true,
  'mcp_servers.devmap_candidate.enabled_tools': ['devmap_read_map', 'devmap_set_route_plan'],
  'mcp_servers.devmap_candidate.default_tools_approval_mode': 'approve',
};
const args = ['exec', '--ephemeral', '--ignore-user-config', '--json', '--sandbox', 'read-only', '--cd', source];
for (const [key, value] of Object.entries(configs)) args.push('-c', `${key}=${JSON.stringify(value)}`);
args.push(prompt);
const metadata = {scope: 'actual_codex_cli_mcp_smoke', source, candidate,
  candidate_sha256: crypto.createHash('sha256').update(fs.readFileSync(candidate)).digest('hex'),
  codex, args, started_at: new Date().toISOString(),
  note: 'Ephemeral actual CLI MCP flow. This does not establish automatic hooks, desktop navigation, shared owner restart, or final resource acceptance.'};
fs.writeFileSync(path.join(output, 'manifest.json'), JSON.stringify(metadata, null, 2));
console.log(output);
const stdout = fs.createWriteStream(path.join(output, 'events.jsonl'), {flags: 'wx'});
const stderr = fs.createWriteStream(path.join(output, 'stderr.log'), {flags: 'wx'});
const child = spawn(codex, args, {cwd: source, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true});
child.stdout.pipe(stdout);
child.stderr.pipe(stderr);
let expired = false;
const timer = setTimeout(() => { expired = true; child.kill(); }, 180000);
child.on('error', error => { clearTimeout(timer); console.error(error); process.exitCode = 1; });
child.on('close', async code => {
  clearTimeout(timer);
  await Promise.all([new Promise(resolve => stdout.closed ? resolve() : stdout.on('close', resolve)), new Promise(resolve => stderr.closed ? resolve() : stderr.on('close', resolve))]);
  const events = fs.readFileSync(path.join(output, 'events.jsonl'), 'utf8').split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line));
  const report = assess(events, code, expired);
  fs.writeFileSync(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
  if (!report.passed) process.exitCode = 1;
});
