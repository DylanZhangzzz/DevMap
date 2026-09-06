#!/usr/bin/env node
'use strict';
const path = require('node:path');
const fs = require('node:fs');
const os = require('node:os');
const { spawn } = require('node:child_process');

const platforms = {
  'win32-x64': { target: 'x86_64-pc-windows-msvc', binary: 'devmap.exe' },
  'darwin-x64': { target: 'x86_64-apple-darwin', binary: 'devmap' },
  'darwin-arm64': { target: 'aarch64-apple-darwin', binary: 'devmap' },
  'linux-x64': { target: 'x86_64-unknown-linux-gnu', binary: 'devmap' },
  'linux-arm64': { target: 'aarch64-unknown-linux-gnu', binary: 'devmap' }
};

function selectPlatform(platform = process.platform, arch = process.arch, glibc) {
  const key = platform + '-' + arch;
  if (!Object.hasOwn(platforms, key)) throw new Error('Unsupported DevMap platform: ' + key);
  if (platform === 'linux') {
    const version = glibc ?? process.report.getReport().header.glibcVersionRuntime;
    const match = typeof version === 'string' && version.match(/^(\d+)\.(\d+)(?:\.|$)/);
    if (!match || Number(match[1]) < 2 || (Number(match[1]) === 2 && Number(match[2]) < 35)) {
      throw new Error('DevMap prebuilt Linux binaries support glibc 2.35+; build from source on older glibc or musl/Alpine.');
    }
  }
  return key;
}

function main() {
  const key = selectPlatform();
  const binary = path.join(__dirname, 'native', key, platforms[key].binary);
  if (!fs.existsSync(binary)) throw new Error('Missing bundled DevMap binary for ' + key + '. Reinstall the complete release package.');
  const child = spawn(binary, process.argv.slice(2), {
    stdio: 'inherit', shell: false, windowsHide: true
  });
  const handlers = new Map();
  for (const signal of ['SIGINT', 'SIGTERM']) {
    const handler = () => child.kill(signal);
    handlers.set(signal, handler);
    process.on(signal, handler);
  }
  const cleanup = () => {
    for (const [signal, handler] of handlers) process.removeListener(signal, handler);
  };
  child.on('error', error => {
    cleanup();
    console.error('DevMap could not start: ' + error.message);
    process.exitCode = 1;
  });
  child.on('exit', (code, signal) => {
    cleanup();
    process.exitCode = code ?? (128 + (os.constants.signals[signal] || 1));
  });
}

module.exports = { platforms, selectPlatform };
if (require.main === module) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
