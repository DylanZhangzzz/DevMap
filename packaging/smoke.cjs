'use strict';
// Real packed-package installation and stdio protocol test; no registry or mocks.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { stagePackage, pack, cargoVersion, npm, run } = require('./release.cjs');
const { selectPlatform } = require('./npm/cli.cjs');

const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'devmap npm smoke '));
try {
  const nativeDir = path.resolve(process.argv[2] || 'dist/native');
  const packageDir = path.join(temp, 'package');
  const name = '@devmap-test/cli';
  const version = cargoVersion();
  stagePackage({ nativeDir, outputDir: packageDir, version, name, platforms: [selectPlatform()] });
  const tarball = pack(packageDir, path.join(temp, 'assets'));
  const consumer = path.join(temp, 'consumer with spaces');
  fs.mkdirSync(consumer);
  fs.writeFileSync(path.join(consumer, 'package.json'), '{"name":"smoke-consumer","version":"1.0.0","private":true}');
  const env = { ...process.env, npm_config_cache: path.join(temp, 'npm-cache'), npm_config_update_notifier: 'false' };
  npm(['install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund', tarball], { cwd: consumer, env });
  const installed = path.join(consumer, 'node_modules/@devmap-test/cli/cli.cjs');
  assert.equal(run(process.execPath, [installed, '--version']).stdout.trim(), 'devmap ' + version);
  // npm exec is the npx implementation; exercise npm's installed bin shim too.
  assert.equal(npm(['exec', '--offline', '--', 'devmap', '--version'], { cwd: consumer, env }).stdout.trim(), 'devmap ' + version);

  const repo = path.join(temp, 'repo with spaces & literal');
  fs.mkdirSync(repo);
  // Exercise the documented pre-publication tarball command outside the installed consumer.
  assert.equal(npm(['exec', '--offline', '--yes', '--package=' + tarball, '--', 'devmap', '--version'], { cwd: repo, env }).stdout.trim(), 'devmap ' + version);
  run('git', ['init', '-b', 'main', repo]);
  run('git', ['-c', 'user.name=Smoke', '-c', 'user.email=smoke@example.invalid', 'commit', '--allow-empty', '-m', 'Initial'], { cwd: repo });
  const args = ['agents', '--source', repo, '--json'];
  const explicit = JSON.parse(run(process.execPath, [installed, ...args], { cwd: consumer }).stdout);
  assert.ok(explicit && typeof explicit === 'object');
  const implicit = JSON.parse(run(process.execPath, [installed, 'agents', '--json'], { cwd: repo }).stdout);
  assert.ok(implicit && typeof implicit === 'object');
  const invalid = spawnSync(process.execPath, [installed, '--not-a-real-option'], { encoding: 'utf8', windowsHide: true });
  assert.equal(invalid.status, 2);
  assert.equal(invalid.stdout, '');
  assert.match(invalid.stderr, /unexpected argument/);

  const requests = [
    { jsonrpc: '2.0', id: 1, method: 'initialize', params: {
      protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'npm-smoke', version: '1' }
    }},
    { jsonrpc: '2.0', method: 'notifications/initialized', params: {} },
    { jsonrpc: '2.0', id: 2, method: 'tools/list', params: {} }
  ];
  const response = run(process.execPath, [installed, 'mcp', '--source', repo], {
    input: requests.map(request => JSON.stringify(request)).join('\n') + '\n', timeout: 30000
  });
  const messages = response.stdout.trim().split('\n').map(line => JSON.parse(line));
  assert.equal(messages.length, 2, 'stdout must contain only MCP responses');
  assert.equal(messages[0].id, 1);
  assert.equal(messages[0].result.protocolVersion, '2025-11-25');
  assert.equal(messages[1].id, 2);
  assert.ok(messages[1].result.tools.some(tool => tool.name === 'devmap_read_map'));
  assert.ok(messages[1].result.tools.some(tool => tool.name === 'devmap_open_map'));

  fs.renameSync(path.join(consumer, 'node_modules/@devmap-test/cli/native'), path.join(consumer, 'omitted-native'));
  const missing = spawnSync(process.execPath, [installed, '--version'], { encoding: 'utf8', windowsHide: true });
  assert.equal(missing.status, 1);
  assert.equal(missing.stdout, '');
  assert.match(missing.stderr, /Missing bundled/);
  console.log('PASS: packed offline install, npm exec, version, cwd/arguments with spaces, exit codes, MCP stdio, missing-binary failure');
} finally {
  // mkdtemp returns this test's own absolute directory; never accept a cleanup path from arguments.
  assert.ok(path.resolve(temp).startsWith(path.resolve(os.tmpdir()) + path.sep));
  fs.rmSync(temp, { recursive: true, force: true });
}
