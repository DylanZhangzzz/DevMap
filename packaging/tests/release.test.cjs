const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const release = require('../release.cjs');
const { selectPlatform } = require('../npm/cli.cjs');

test('selects native executable and rejects unsupported architectures and libc', () => {
  assert.equal(selectPlatform('win32', 'x64'), 'win32-x64');
  assert.equal(selectPlatform('darwin', 'arm64'), 'darwin-arm64');
  assert.equal(selectPlatform('linux', 'arm64', '2.35'), 'linux-arm64');
  assert.equal(selectPlatform('linux', 'x64', '2.39'), 'linux-x64');
  assert.throws(() => selectPlatform('linux', 'x64', '2.31'), /2.35/);
  assert.throws(() => selectPlatform('win32', 'arm64'), /Unsupported/);
  assert.throws(() => selectPlatform('linux', 'x64', false), /glibc/);
});

test('rejects a tag different from the binary version and unsafe versions', () => {
  assert.equal(release.validateVersion('0.1.0', 'v0.1.0'), '0.1.0');
  assert.equal(release.validateVersion('1.2.3-rc.1', 'v1.2.3-rc.1'), '1.2.3-rc.1');
  assert.throws(() => release.validateVersion('0.1.0', 'v0.2.0'), /match/);
  for (const version of ['../x', '1.0', '01.0.0', '1.0.0-01']) {
    assert.throws(() => release.validateVersion(version), /version/);
  }
});

test('incomplete release refuses to create a publishable package', t => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'devmap missing '));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  assert.throws(() => release.stagePackage({
    nativeDir: path.join(dir, 'native'),
    outputDir: path.join(dir, 'package'),
    version: '0.1.0', name: '@example/devmap'
  }), /Missing binary/);
  assert.equal(fs.existsSync(path.join(dir, 'package/package.json')), false);
});

test('stages only explicitly selected binaries and keeps executable permissions', t => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'devmap staging '));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const binary = path.join(dir, 'native/darwin-arm64/devmap');
  fs.mkdirSync(path.dirname(binary), { recursive: true });
  fs.writeFileSync(binary, 'fixture binary');
  const outputDir = path.join(dir, 'package');
  release.stagePackage({ nativeDir: path.join(dir, 'native'), outputDir,
    version: '0.1.0', name: '@example/devmap', platforms: ['darwin-arm64'] });
  const pkg = JSON.parse(fs.readFileSync(path.join(outputDir, 'package.json')));
  assert.equal(pkg.name, '@example/devmap');
  assert.equal(pkg.version, '0.1.0');
  assert.equal(pkg.scripts, undefined);
  assert.equal(pkg.dependencies, undefined);
  assert.equal(fs.readFileSync(path.join(outputDir, 'native/darwin-arm64/devmap'), 'utf8'), 'fixture binary');
  if (process.platform !== 'win32') assert.ok(fs.statSync(path.join(outputDir, 'native/darwin-arm64/devmap')).mode & 0o111);
});

test('rejects invalid package names before writing output', () => {
  assert.throws(() => release.stagePackage({ name: 'BAD NAME', version: '0.1.0' }), /package name/);
});
