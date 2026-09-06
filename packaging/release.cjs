'use strict';
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawnSync } = require('node:child_process');
const { platforms } = require('./npm/cli.cjs');
const root = path.resolve(__dirname, '..');

function validateVersion(version, tag) {
  const number = '(?:0|[1-9][0-9]*)';
  const identifier = '(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)';
  const re = new RegExp('^' + number + '\\.' + number + '\\.' + number + '(?:-' + identifier + '(?:\\.' + identifier + ')*)?$');
  if (!re.test(version)) throw new Error('Invalid release version: ' + version);
  if (tag && tag !== 'v' + version) throw new Error('Release tag must match Cargo.toml version: v' + version);
  return version;
}

function cargoVersion(tag) {
  const cargo = fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8');
  const section = cargo.split('[package]')[1]?.split(/\n\[/)[0];
  const version = section?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  return validateVersion(version || '', tag);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: 'utf8', windowsHide: true, ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(command + ' failed (' + result.status + '):\n' + result.stdout + result.stderr);
  return result;
}

function npm(args, options = {}) {
  if (process.platform !== 'win32') return run('npm', args, options);
  // Execute npm's JS entry point directly: no cmd.exe, quoting or shell expansion.
  const directories = [path.dirname(process.execPath), ...(process.env.PATH || '').split(path.delimiter)];
  const candidates = [process.env.npm_execpath, ...directories.map(dir => path.join(dir, 'node_modules/npm/bin/npm-cli.js'))];
  const cli = candidates.find(file => file && fs.existsSync(file));
  if (!cli) throw new Error('Cannot locate npm-cli.js. Install Node.js with npm or run via npm.');
  return run(process.execPath, [cli, ...args], options);
}

function stagePackage({ nativeDir, outputDir, version, name, platforms: selected = Object.keys(platforms) }) {
  if (!/^(@[a-z0-9][a-z0-9._-]*\/)?[a-z0-9][a-z0-9._-]*$/.test(name || '') || name.length > 214) {
    throw new Error('Invalid npm package name');
  }
  validateVersion(version);
  if (!selected.length) throw new Error('No platforms selected');
  // Preflight all binaries before creating a package that could be published.
  for (const key of selected) {
    if (!Object.hasOwn(platforms, key)) throw new Error('Unsupported platform: ' + key);
    const file = path.join(nativeDir, key, platforms[key].binary);
    if (!fs.existsSync(file) || !fs.statSync(file).isFile() || fs.statSync(file).size === 0) {
      throw new Error('Missing binary: ' + file);
    }
  }
  if (fs.existsSync(outputDir)) throw new Error('Output directory already exists: ' + outputDir);
  fs.mkdirSync(outputDir, { recursive: true });
  const metadata = {
    name, version, description: 'A local Git worktree map for humans and AI agents',
    license: 'Apache-2.0',
    repository: { type: 'git', url: 'git+https://github.com/DylanZhangzzz/DevMap.git' },
    homepage: 'https://github.com/DylanZhangzzz/DevMap#readme',
    bin: { devmap: 'cli.cjs' }, engines: { node: '>=22' },
    files: ['cli.cjs', 'native', 'README.md'], publishConfig: { access: 'public' }
  };
  fs.writeFileSync(path.join(outputDir, 'package.json'), JSON.stringify(metadata, null, 2) + '\n');
  fs.copyFileSync(path.join(__dirname, 'npm/cli.cjs'), path.join(outputDir, 'cli.cjs'));
  fs.chmodSync(path.join(outputDir, 'cli.cjs'), 0o755);
  fs.copyFileSync(path.join(__dirname, 'npm/README.md'), path.join(outputDir, 'README.md'));
  for (const key of selected) {
    const relative = path.join(key, platforms[key].binary);
    const destination = path.join(outputDir, 'native', relative);
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.copyFileSync(path.join(nativeDir, relative), destination);
    fs.chmodSync(destination, 0o755);
  }
  return outputDir;
}

function stageNative(key, binary, dist, version) {
  if (!Object.hasOwn(platforms, key)) throw new Error('Unsupported platform: ' + key);
  validateVersion(version);
  const directory = path.join(dist, 'native', key);
  fs.mkdirSync(directory, { recursive: true });
  const destination = path.join(directory, platforms[key].binary);
  fs.copyFileSync(binary, destination);
  fs.chmodSync(destination, 0o755);
  const assets = path.join(dist, 'assets');
  fs.mkdirSync(assets, { recursive: true });
  const archive = path.resolve(assets, 'devmap-' + version + '-' + platforms[key].target + '.tar.gz');
  run('tar', ['-czf', archive, '-C', directory, platforms[key].binary]);
  return archive;
}

function pack(outputDir, assets) {
  fs.mkdirSync(assets, { recursive: true });
  const result = npm(['pack', '--json', '--ignore-scripts', '--pack-destination', path.resolve(assets)], { cwd: outputDir });
  return path.resolve(assets, JSON.parse(result.stdout)[0].filename);
}

function checksums(assets) {
  const files = fs.readdirSync(assets).filter(file => file !== 'SHA256SUMS').sort();
  if (!files.length) throw new Error('No release assets');
  const lines = files.map(file => crypto.createHash('sha256').update(fs.readFileSync(path.join(assets, file))).digest('hex') + '  ' + file);
  fs.writeFileSync(path.join(assets, 'SHA256SUMS'), lines.join('\n') + '\n');
}

function main() {
  const [command, ...args] = process.argv.slice(2);
  const tag = process.env.GITHUB_REF_TYPE === 'tag' ? process.env.GITHUB_REF_NAME : undefined;
  const version = cargoVersion(tag);
  if (command === 'version') console.log(version);
  else if (command === 'native') console.log(stageNative(args[0], args[1], args[2], version));
  else if (command === 'assemble') {
    const [dist, name] = args;
    const outputDir = path.join(dist, 'npm-package');
    stagePackage({ nativeDir: path.join(dist, 'native'), outputDir, version, name });
    console.log(pack(outputDir, path.join(dist, 'assets')));
    checksums(path.join(dist, 'assets'));
  } else throw new Error('Usage: node packaging/release.cjs version | native PLATFORM BINARY DIST | assemble DIST PACKAGE_NAME');
}

module.exports = { validateVersion, cargoVersion, stagePackage, stageNative, pack, checksums, run, npm };
if (require.main === module) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
