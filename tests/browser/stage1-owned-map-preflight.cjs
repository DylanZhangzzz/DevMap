'use strict';
// Small transport/ownership preflight only. Not calibration or performance acceptance.
const fs = require('node:fs'), path = require('node:path'), net = require('node:net');
const assert = require('node:assert/strict'), crypto = require('node:crypto');
const {spawn} = require('node:child_process');
const h = require('./shared-summary-performance.cjs');
const root = path.resolve(__dirname, '../..');
const allowed = h.checked(path.join(root, 'target/verification'), 'directory').path;
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const read = file => JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
const owned = (file, kind) => { const item = h.checked(file, kind); assert(h.within(item.path, allowed)); return item; };

async function absent(repository) {
  assert.match(repository, /^[a-f0-9]{64}$/);
  await new Promise((resolve, reject) => {
    const socket = net.createConnection(`\\\\.\\pipe\\devmap-${repository}`);
    const timer = setTimeout(() => finish(new Error('Endpoint absence timeout')), 3000);
    let settled = false;
    function finish(error) { if (settled) return; settled = true; clearTimeout(timer); socket.destroy(); error ? reject(error) : resolve(); }
    socket.on('connect', () => finish(new Error('Endpoint occupied; no external process will be stopped')));
    socket.on('error', error => finish(error.code === 'ENOENT' ? null : error));
  });
}

async function main(configFile) {
  assert.equal(process.platform, 'win32');
  const config = read(owned(configFile, 'file').path);
  assert.equal(config.schema, 'devmap/stage1-map-preflight/1');
  const clients = config.clients ?? 1, samples = config.samples ?? 1;
  assert([1, 4].includes(clients));
  assert(Number.isInteger(samples) && samples >= 1 && samples <= 10, 'Preflight is bounded to ten samples/client');
  const manifest = read(owned(config.manifest, 'file').path);
  assert.equal(manifest.scope, 'legacy_native_process_fixture');
  const fixture = owned(manifest.fixture_root, 'directory');
  const source = owned(manifest.source, 'directory');
  assert(h.within(source.path, fixture.path));
  const baseline = owned(config.baseline, 'file'), candidate = owned(config.candidate, 'file');
  assert.equal(hash(fs.readFileSync(baseline.path)), 'a1cfbb1c46bd9026b18db67da9f70485b0bb20c6c54fd779475b52531731d419');
  assert.match(config.candidate_sha256, /^[a-f0-9]{64}$/i);
  assert.equal(hash(fs.readFileSync(candidate.path)), config.candidate_sha256.toLowerCase());
  const executables = [baseline, candidate].map(identity => ({identity, sha: hash(fs.readFileSync(identity.path))}));
  const python = h.checked(config.python, 'file').path;
  const run = fs.mkdtempSync(path.join(allowed, 'stage1-owned-map-'));
  const runIdentity = h.checked(run, 'directory');
  const report = {schema: config.schema, run, source: source.path, config,
    scope: 'small public-map transport and planned Job teardown; not natural lifecycle or performance acceptance',
    formal_acceptance: false, jobs: [], errors: [], samples: [], completed: false};
  console.log(JSON.stringify({run}));
  let sql, immutable;
  const db = path.join(source.path, '.git/devmap/devmap.db');
  const legacyFiles = manifest.frozen_sources.flatMap(origin => origin.files.map(file => ({
    path: owned(path.join(origin.original, file.relative), 'file').path, sha: file.sha256,
  })));
  const backups = manifest.frozen_sources.map(origin => owned(origin.destination, 'directory').path);
  function preserve() {
    assert.deepEqual(h.checked(run, 'directory'), runIdentity);
    assert.deepEqual(h.checked(source.path, 'directory'), source);
    for (const executable of executables) {
      assert.deepEqual(h.checked(executable.identity.path, 'file'), executable.identity);
      assert.equal(hash(fs.readFileSync(executable.identity.path)), executable.sha);
    }
    for (const file of legacyFiles) assert.equal(hash(fs.readFileSync(owned(file.path, 'file').path)), file.sha);
    if (sql) assert.deepEqual(h.sqlState(python, db), sql);
    if (immutable) assert.deepEqual(h.inventory(backups), immutable);
  }
  async function execute(label, executable, args, env, planned) {
    preserve();
    const output = path.join(run, `${label}.stdout`), error = path.join(run, `${label}.stderr`);
    const jobFile = path.join(run, `${label}.job.json`);
    const stdout = fs.openSync(output, 'wx'), stderr = fs.openSync(error, 'wx');
    const argv = [path.join(__dirname, 'windows-owned-generator-job.py'), '--report', jobFile, '--exe', executable];
    if (planned) argv.push('--teardown-descendants-after-success');
    argv.push('--', ...args);
    let child;
    try { child = spawn(python, argv, {cwd: root, env: {...process.env, ...env}, windowsHide: true, stdio: ['pipe', stdout, stderr]}); }
    finally { fs.closeSync(stdout); fs.closeSync(stderr); }
    let expired = false;
    const code = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => { expired = true; child.stdin?.end('abort'); }, 90000);
      child.stdin?.on('error', () => {});
      child.once('error', error => { clearTimeout(timer); reject(error); });
      child.once('close', code => { clearTimeout(timer); resolve(code); });
    });
    const job = read(jobFile);
    report.jobs.push({label, code, expired, ...job});
    assert(!expired); assert.equal(code, 0); assert.equal(job.root_exit_code, 0);
    assert.equal(job.empty_confirmed, true); assert.equal(job.aborted, false);
    for (const key of ['error', 'cleanup_error', 'descendants_after_root_exit', 'suspended_root_reap_failed']) assert(!job[key], key);
    if (planned) { assert.equal(job.cleanup_policy, 'planned_after_success'); assert.equal(job.natural_lifecycle_acceptance, false); }
    preserve();
    return output;
  }
  try {
    sql = h.sqlState(python, db);
    // Include activation snapshots, not merely the legacy generator's copies.
    for (const backup of sql.activation_snapshot_paths) {
      const checked = owned(backup, 'directory').path;
      assert(h.within(checked, fixture.path));
      if (!backups.includes(checked)) backups.push(checked);
    }
    immutable = h.inventory(backups); preserve();
    const identityFile = await execute('identity', candidate.path, ['runtime', '--identity', '--source', source.path], {}, false);
    const identity = read(identityFile);
    assert.equal(h.checked(identity.source, 'directory').path, source.path);
    assert.equal(h.checked(identity.common, 'directory').path, h.checked(path.dirname(path.dirname(db)), 'directory').path);
    report.runtime_identity = identity;
    for (const [side, executable] of [['baseline', baseline.path], ['candidate', candidate.path]]) {
      await absent(identity.repository);
      const workerReport = path.join(run, `${side}.worker.json`);
      await execute(side, process.execPath, [path.join(__dirname, 'process-performance.cjs')], {
        DEVMAP_BENCHMARK_EXE: executable,
        DEVMAP_BENCHMARK_SOURCE: source.path,
        DEVMAP_BENCHMARK_OUTPUT: workerReport,
        DEVMAP_BENCHMARK_COLD: '1', DEVMAP_BENCHMARK_SAMPLES: String(samples), DEVMAP_BENCHMARK_WARMUP: '1',
        DEVMAP_BENCHMARK_CLIENTS: String(clients),
      }, true);
      const result = read(workerReport);
      assert.equal(result.scope, 'native_mcp_full_map_including_git');
      assert.equal(result.executable_sha256, hash(fs.readFileSync(executable)));
      assert.equal(result.cold.count, 1); assert.equal(result.hot.count, clients * samples);
      assert.equal(result.clients, clients); assert.equal(result.cohorts.length, clients);
      for (const cohort of result.cohorts) {
        assert.equal(cohort.summary.count, samples); assert.deepEqual(cohort.errors, []);
      }
      report.samples.push({side, ...result});
      await absent(identity.repository);
    }
    report.completed = true;
  } catch (error) { report.errors.push(String(error.stack || error)); }
  finally {
    try { preserve(); report.preservation = true; } catch (error) { report.errors.push(String(error)); }
    if (report.errors.length) report.completed = false;
    fs.writeFileSync(path.join(run, 'report.json'), JSON.stringify(report, null, 2), {flag: 'wx'});
  }
  console.log(JSON.stringify({run, completed: report.completed, errors: report.errors}));
  process.exitCode = report.completed ? 0 : 1;
}
if (require.main === module) main(path.resolve(process.argv[2])).catch(error => { console.error(error); process.exitCode = 1; });
