#!/usr/bin/env node
'use strict';
/**
 * Castle selfplay coordinator, pilot version.
 * Default --offline-check has no Castle requests. Live mode is capped at ONE
 * game and requires both --live-one and --permit-live explicitly.
 */
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const { spawn, execFileSync } = require('node:child_process');
const { buildSchedule, checkLocalFiles, checkRemote } = require('./pbs-selfplay-controller.js');

const ROOT = path.resolve(__dirname, '..');
const DEFAULT_REMOTE = 'debian@161.34.34.171';
const DEFAULT_KEY = '/home/hm/private_key.txt';
const DEFAULT_REMOTE_ROOT = '/home/debian/nc2000-p2';

function assert(value, message) { if (!value) throw new Error(message); }
function shquote(value) { return "'" + String(value).replace(/'/g, "'\\''") + "'"; }
function safeToken(value) { assert(/^[A-Za-z0-9_-]+$/.test(value), 'invalid token: ' + value); return value; }
function argInt(value, label, min, max) {
  assert(/^[0-9]+$/.test(String(value)), 'invalid ' + label);
  const n = Number(value);
  assert(Number.isSafeInteger(n) && n >= min && n <= max, 'out of range: ' + label);
  return n;
}
function argsParse(argv) {
  const cfg = {
    mode: 'offline-check', game: 1, teamCount: 10, games: 90,
    seed: 20261109, remote: DEFAULT_REMOTE, key: DEFAULT_KEY,
    remoteRoot: DEFAULT_REMOTE_ROOT, room: 'K',
    timeoutSec: 1800, pollMs: 2500, permitLive: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    if (flag === '--offline-check' || flag === '--live-one' || flag === '--self-test') {
      cfg.mode = flag.slice(2); continue;
    }
    if (flag === '--permit-live') { cfg.permitLive = true; continue; }
    const map = {
      '--game': 'game', '--teams': 'teamCount', '--games': 'games',
      '--seed': 'seed', '--remote': 'remote', '--key': 'key',
      '--remote-root': 'remoteRoot', '--room': 'room',
      '--timeout-sec': 'timeoutSec', '--poll-ms': 'pollMs',
    };
    assert(map[flag] && i + 1 < argv.length && !argv[i + 1].startsWith('--'),
      'unknown/incomplete argument: ' + flag);
    cfg[map[flag]] = argv[++i];
  }
  cfg.game = argInt(cfg.game, 'game', 1, 100000);
  cfg.teamCount = argInt(cfg.teamCount, 'teams', 2, 1000);
  cfg.games = argInt(cfg.games, 'games', 1, 100000);
  cfg.seed = argInt(cfg.seed, 'seed', 1, 1000000000);
  cfg.timeoutSec = argInt(cfg.timeoutSec, 'timeout-sec', 60, 7200);
  cfg.pollMs = argInt(cfg.pollMs, 'poll-ms', 500, 10000);
  cfg.room = safeToken(String(cfg.room).toUpperCase());
  assert(/^[C-O]$/.test(cfg.room), 'room must be a letter C through O');
  assert(cfg.game <= cfg.games, '--game exceeds --games');
  assert(cfg.remote === DEFAULT_REMOTE && cfg.remoteRoot === DEFAULT_REMOTE_ROOT,
    'pilot uses the verified P2 remote and root only');
  assert(cfg.key === DEFAULT_KEY, 'pilot uses the verified SSH key only');
  if (cfg.mode === 'live-one') assert(cfg.permitLive, '--live-one requires --permit-live');
  return cfg;
}
function keySsh(cfg, command, opts = {}) {
  return execFileSync('ssh', [
    '-T', '-i', cfg.key, '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=15',
    cfg.remote, command,
  ], { encoding: 'utf8', maxBuffer: 4 * 1024 * 1024, ...opts });
}
function validateTeamEncoding(cfg, selectedIds) {
  const code = `
    process.argv = [process.execPath, 'tools/pbs-client.js', '--random'];
    const fs = require('node:fs');
    const pool = JSON.parse(fs.readFileSync('data/team-pool-v2/team-pool.json', 'utf8'));
    const client = require('./tools/pbs-client.js');
    const ids = ${JSON.stringify(selectedIds)};
    for (const id of ids) {
      const t = pool.teams.find(x => x.id === id);
      if (!t || !Array.isArray(t.sets) || t.sets.length !== 6) throw Error('bad team ' + id);
      const encoded = client.encodeTeamForPbsCookie(t);
      if (!encoded || typeof encoded.cookieValue !== 'string' || encoded.cookieValue.length < 50)
        throw Error('bad PD0 encoding ' + id);
    }
    console.log('P2_TEAM_ENCODINGS=PASS count=' + ids.length);
  `;
  // Local check uses same P2 random path to avoid expensive WASM initialization.
  const local = execFileSync(process.execPath, ['-e', code], {cwd: ROOT, encoding: 'utf8'}).trim();
  assert(local === 'P2_TEAM_ENCODINGS=PASS count=' + selectedIds.length,
    'unexpected local encoding check: ' + local);
  console.log('LOCAL_TEAM_ENCODINGS=PASS (' + selectedIds.length + ')');
  const remoteCmd = 'cd ' + shquote(cfg.remoteRoot) +
    ' && ./bin/node -e ' + shquote(code);
  const remote = keySsh(cfg, remoteCmd).trim();
  assert(remote === local, 'remote team encoder disagreed: ' + remote);
  console.log('REMOTE_TEAM_ENCODINGS=PASS (' + selectedIds.length + ')');
}

function inspectTrace(dir, side, team) {
  const traceDir = path.join(dir, 'traces');
  assert(fs.existsSync(traceDir), side + ': no trace directory');
  const files = fs.readdirSync(traceDir).filter(n => n.endsWith('.jsonl'));
  assert(files.length === 1, side + ': expected one trace file, found ' + files.length);
  const events = fs.readFileSync(path.join(traceDir, files[0]), 'utf8')
    .split(/\r?\n/).filter(Boolean).map((line, i) => {
      try { return JSON.parse(line); }
      catch (e) { throw Error(side + ': invalid trace line ' + (i + 1) + ': ' + e.message); }
    });
  const starts = events.filter(e => e.type === 'match_start');
  const finishes = events.filter(e => e.type === 'finish');
  const decisions = events.filter(e => e.type === 'decision');
  assert(starts.length === 1, side + ': expected one match_start');
  assert(finishes.length === 1, side + ': expected one finish');
  assert(starts[0].team && starts[0].team.id === team,
    side + ': team mismatch expected=' + team + ' actual=' + starts[0].team?.id);
  assert(finishes[0].mySide === (side === 'P1' ? 0 : 1), side + ': side mismatch');
  assert(decisions.length > 0, side + ': no battle decisions');
  const matchLog = path.join(dir, 'matches.jsonl');
  assert(fs.existsSync(matchLog), side + ': missing completed match record');
  const records = fs.readFileSync(matchLog, 'utf8').split(/\r?\n/).filter(Boolean).map(JSON.parse);
  assert(records.length === 1 && records[0].matchId === starts[0].matchId,
    side + ': completed match log does not match trace');
  assert(['WIN', 'LOSE', 'DRAW'].includes(finishes[0].outcome),
    side + ': ambiguous match outcome ' + finishes[0].outcome);
  return { outcome: finishes[0].outcome, decisions: decisions.length,
    trace: files[0], matchId: starts[0].matchId,
    errors: events.flatMap(e => e.errors || []).length };
}
function inspectOutcomes(p1, p2) {
  const a = p1.outcome, b = p2.outcome;
  assert((a === 'DRAW' && b === 'DRAW') ||
    (a === 'WIN' && b === 'LOSE') || (a === 'LOSE' && b === 'WIN'),
    'opponent outcome conflict P1=' + a + ' P2=' + b);
}
function stopRemoteWorker(cfg, remoteDir) {
  const pidFile = path.posix.join(remoteDir, 'p2.pid');
  const cmd = [
    'set -eu',
    'f=' + shquote(pidFile),
    'if test ! -r "$f"; then echo REMOTE_STOP=NO_PID; exit 0; fi',
    'pid=$(cat "$f")',
    'case "$pid" in *[!0-9]*|"") echo REMOTE_STOP=INVALID_PID; exit 0;; esac',
    'command=$(ps -p "$pid" -o args= 2>/dev/null || true)',
    'if printf "%s" "$command" | grep -F -- ' + shquote('tools/pbs-client.js') +
      ' >/dev/null && printf "%s" "$command" | grep -F -- ' + shquote(remoteDir) +
      ' >/dev/null; then kill -TERM "$pid" 2>/dev/null || true; echo REMOTE_STOP=SIGTERM_SENT; ' +
      'else echo REMOTE_STOP=NOT_OUR_PROCESS; fi',
  ].join('; ');
  return keySsh(cfg, cmd).trim();
}
function auditDir(dir) {
  const raw = execFileSync(process.execPath,
    ['tools/pbs-parity-audit.js', path.join(dir, 'traces'), '--no-out'],
    { cwd: ROOT, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 }).trim();
  const report = JSON.parse(raw);
  assert(report.matches === 1, 'audit did not find exactly one match');
  return report;
}
function printLinePrefixed(label, target, chunk, pending) {
  const combined = pending.text + chunk.toString('utf8');
  const lines = combined.split(/\r?\n/);
  pending.text = lines.pop();
  for (const line of lines) target.write('[' + label + '] ' + line + '\n');
}
function startProcess(label, cmd, argv, options = {}) {
  const child = spawn(cmd, argv, { ...options, stdio: ['ignore', 'pipe', 'pipe'] });
  const out = {text: ''}, err = {text: ''};
  child.stdout.on('data', c => printLinePrefixed(label, process.stdout, c, out));
  child.stderr.on('data', c => printLinePrefixed(label, process.stderr, c, err));
  const promise = new Promise(resolve => {
    let failure = null;
    child.on('error', e => { failure = e; });
    child.on('close', (code, signal) => {
      if (out.text) process.stdout.write('[' + label + '] ' + out.text + '\n');
      if (err.text) process.stderr.write('[' + label + '] ' + err.text + '\n');
      resolve({label, code, signal, failure: failure?.message || null});
    });
  });
  return { child, promise };
}
function terminate(proc) {
  if (!proc) return;
  try { if (proc.child.exitCode === null && proc.child.signalCode === null) proc.child.kill('SIGTERM'); }
  catch (_) { /* best effort */ }
  const grace = setTimeout(() => {
    try { if (proc.child.exitCode === null && proc.child.signalCode === null) proc.child.kill('SIGKILL'); }
    catch (_) { /* best effort */ }
  }, 5000);
  grace.unref();
}
async function runPair(p1Spec, p2Spec, seconds) {
  const p1 = startProcess('P1', p1Spec.cmd, p1Spec.argv, p1Spec.options);
  const p2 = startProcess('P2', p2Spec.cmd, p2Spec.argv, p2Spec.options);
  let expired = false;
  let interrupted = false;
  const onSignal = () => {
    interrupted = true; terminate(p1); terminate(p2);
  };
  process.on('SIGINT', onSignal);
  process.on('SIGTERM', onSignal);
  const timer = setTimeout(() => { expired = true; terminate(p1); terminate(p2); }, seconds * 1000);
  try {
    const first = await Promise.race([p1.promise, p2.promise]);
    if (first.code !== 0 || first.signal || first.failure) {
      terminate(first.label === 'P1' ? p2 : p1);
    }
    const both = await Promise.all([p1.promise, p2.promise]);
    return {both, expired, interrupted};
  } finally {
    clearTimeout(timer);
    process.off('SIGINT', onSignal);
    process.off('SIGTERM', onSignal);
  }
}
async function fetchRemoteTraces(cfg, remoteDir, localDir) {
  fs.mkdirSync(localDir, {recursive: true, mode: 0o700});
  const remote = spawn('ssh', [
    '-T', '-i', cfg.key, '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=15',
    cfg.remote, 'tar -C ' + shquote(remoteDir) + ' -cf - .',
  ], { stdio: ['ignore', 'pipe', 'pipe'] });
  const extractor = spawn('tar', ['-C', localDir, '-xf', '-'],
    { stdio: ['pipe', 'ignore', 'pipe'] });
  remote.stdout.pipe(extractor.stdin);
  let rerr = '', terr = '';
  remote.stderr.on('data', x => { rerr += x.toString(); });
  extractor.stderr.on('data', x => { terr += x.toString(); });
  const settled = child => new Promise(resolve => {
    let spawnFailure = null;
    child.on('error', e => { spawnFailure = e.message; });
    child.on('close', (code, signal) => resolve({code, signal, spawnFailure}));
  });
  // Pipe errors can occur if tar exits early; report failure through process statuses.
  extractor.stdin.on('error', () => {});
  const [a, b] = await Promise.all([settled(remote), settled(extractor)]);
  assert(a.code === 0 && b.code === 0 && !a.spawnFailure && !b.spawnFailure,
    'trace transfer failed ssh=' + JSON.stringify(a) + ' tar=' + JSON.stringify(b) +
    ' stderr=' + rerr.slice(-1000) + ' ' + terr.slice(-1000));
}

async function selfTestPair() {
  const mock = text => ({ cmd: process.execPath,
    argv: ['-e', 'setTimeout(()=>{console.log(' + JSON.stringify(text) + ');process.exit(0)},40)'],
    options: {cwd: ROOT} });
  const a = await runPair(mock('mock P1 complete'), mock('mock P2 complete'), 4);
  assert(!a.expired && !a.interrupted && a.both.every(x => x.code === 0),
    'offline process coordinator failed');
  console.log('OFFLINE_PROCESS_PAIR=PASS');
  const failing = {cmd: process.execPath, argv: ['-e', 'process.exit(23)'], options:{cwd:ROOT}};
  const slow = {cmd: process.execPath, argv: ['-e', 'setTimeout(()=>process.exit(0),3000)'], options:{cwd:ROOT}};
  const b = await runPair(failing, slow, 4);
  assert(b.both.some(x => x.code === 23) && b.both.some(x => x.signal === 'SIGTERM'),
    'offline failure-stop did not terminate the other process');
  console.log('OFFLINE_FAILURE_STOP=PASS');
}

async function main() {
  const cfg = argsParse(process.argv.slice(2));
  console.log('MODE=' + cfg.mode);
  console.log('LIVE_BATTLES=' + (cfg.mode === 'live-one' ? 'ONE_EXPLICIT' : 'DISABLED'));
  if (cfg.mode === 'self-test') {
    await selfTestPair();
    console.log('PBS_CONNECTION=NOT_ATTEMPTED');
    console.log('RUNNER_SELF_TEST=PASS');
    return;
  }
  checkLocalFiles();
  const schedule = buildSchedule(cfg);
  const selectedGame = schedule.games[cfg.game - 1];
  console.log('SELECTED_TEAMS=' + schedule.ids.join(','));
  console.log('SELECTED_GAME=' + JSON.stringify(selectedGame));
  checkRemote(cfg);
  validateTeamEncoding(cfg, schedule.ids);
  if (cfg.mode === 'offline-check') {
    await selfTestPair();
    console.log('PBS_CONNECTION=NOT_ATTEMPTED');
    console.log('BATTLE_TEST=NOT_STARTED');
    console.log('COORDINATOR_OFFLINE_CHECK=PASS');
    return;
  }
  // One real Castle game only. Never automatically advance to a second game.
  const id = 'g' + selectedGame.number + '-' + Date.now() + '-' + crypto.randomBytes(3).toString('hex');
  const localDir = path.join(os.homedir(), 'pbs-selfplay-runs', id);
  const p1Dir = path.join(localDir, 'p1');
  const p2Dir = path.join(localDir, 'p2');
  const remoteDir = path.posix.join(cfg.remoteRoot, 'runs', id);
  assert(!fs.existsSync(localDir), 'refusing to overwrite existing run');
  fs.mkdirSync(p1Dir, {recursive: true, mode: 0o700});
  keySsh(cfg, 'mkdir -p ' + shquote(remoteDir));
  console.log('RUN_DIR=' + localDir);
  console.log('REMOTE_RUN_DIR=' + remoteDir);
  const p1Args = [path.join(ROOT, 'tools/pbs-client.js'),
    '--room', cfg.room, '--name', 'nc2testp1', '--pass', 'nc2tP1',
    '--mode', 'host', '--team', 'pool:' + selectedGame.p1,
    '--games', '1', '--seed', String(selectedGame.p1Seed),
    '--poll-ms', String(cfg.pollMs), '--host-timeout-sec', '120',
    '--no-opponent-db', '--trace-dir', path.join(p1Dir, 'traces'),
    '--match-log', path.join(p1Dir, 'matches.jsonl'),
    '--anomaly-log', path.join(p1Dir, 'anomalies.jsonl')];
  const p2Args = ['tools/pbs-client.js',
    '--room', cfg.room, '--name', 'nc2testp2', '--pass', 'nc2tP2',
    '--mode', 'join', '--team', 'pool:' + selectedGame.p2,
    '--games', '1', '--seed', String(selectedGame.p2Seed),
    '--random', '--poll-ms', String(cfg.pollMs),
    '--host-timeout-sec', '120', '--no-opponent-db',
    '--trace-dir', path.posix.join(remoteDir, 'traces'),
    '--match-log', path.posix.join(remoteDir, 'matches.jsonl'),
    '--anomaly-log', path.posix.join(remoteDir, 'anomalies.jsonl')];
  const workerShell = 'echo $$ > ' + shquote(path.posix.join(remoteDir, 'p2.pid')) +
    '; exec timeout --signal=TERM --kill-after=5s ' +
    String(cfg.timeoutSec + 60) + 's ./bin/node ' +
    p2Args.map(shquote).join(' ');
  const remoteCmd = 'cd ' + shquote(cfg.remoteRoot) +
    ' && exec sh -c ' + shquote(workerShell);
  const pair = await runPair(
    {cmd: process.execPath, argv: p1Args, options: {cwd: ROOT}},
    {cmd: 'ssh', argv: ['-T', '-i', cfg.key, '-o', 'BatchMode=yes',
      '-o', 'ConnectTimeout=15', cfg.remote, remoteCmd], options: {cwd: ROOT}},
    cfg.timeoutSec);
  console.log('PROCESS_RESULTS=' + JSON.stringify(pair));
  if (pair.expired || pair.interrupted || pair.both.some(x => x.code !== 0 || x.signal || x.failure)) {
    try { console.log(stopRemoteWorker(cfg, remoteDir)); }
    catch (e) { console.error('REMOTE_STOP=FAILED ' + e.message); }
  }
  let downloadError = null;
  try { await fetchRemoteTraces(cfg, remoteDir, p2Dir); console.log('REMOTE_TRACE_DOWNLOAD=PASS'); }
  catch (e) { downloadError = e.message; console.error('REMOTE_TRACE_DOWNLOAD=FAIL: ' + e.message); }
  const summary = {
    schema: 'nc2000-pbs-selfplay-pilot-v1', id, game: selectedGame,
    localDir, remoteDir, process: pair, remoteDownloadError: downloadError,
    crossSourceIndependentAudit: 'NOT_YET_IMPLEMENTED',
    verdict: 'INCOMPLETE',
  };
  try {
    assert(!downloadError, 'remote trace missing');
    assert(!pair.expired && !pair.interrupted && pair.both.every(x => x.code === 0 && !x.signal && !x.failure),
      'client process failed or timed out');
    summary.p1 = inspectTrace(p1Dir, 'P1', selectedGame.p1);
    summary.p2 = inspectTrace(p2Dir, 'P2', selectedGame.p2);
    inspectOutcomes(summary.p1, summary.p2);
    summary.p1Audit = auditDir(p1Dir);
    summary.p2Audit = auditDir(p2Dir);
    assert(summary.p1.errors === 0 && summary.p2.errors === 0,
      'client logged parser/search errors');
    assert(summary.p1Audit.error_count === 0 && summary.p2Audit.error_count === 0,
      'existing parity auditor reported errors');
    summary.verdict = 'PILOT_PROCESS_AND_EXISTING_AUDIT_PASS';
  } catch (e) {
    summary.verdict = 'PILOT_FAIL_OR_UNVERIFIED';
    summary.failureReason = e.message;
  }
  fs.writeFileSync(path.join(localDir, 'summary.json'), JSON.stringify(summary, null, 2) + '\n', {mode:0o600});
  console.log('SUMMARY=' + JSON.stringify(summary));
  console.log('SUMMARY_FILE=' + path.join(localDir, 'summary.json'));
  if (summary.verdict !== 'PILOT_PROCESS_AND_EXISTING_AUDIT_PASS') process.exitCode = 1;
}
main().catch(e => { console.error('FATAL=' + (e.stack || e.message)); process.exitCode = 1; });
