const assert = require('node:assert/strict');
const { test } = require('node:test');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const vm = require('node:vm');
const { execFileSync } = require('node:child_process');

const plugin = path.resolve(__dirname, '..');
function library(file) {
  const context = vm.createContext({});
  vm.runInContext(fs.readFileSync(path.join(plugin, file), 'utf8').replace(/^\.pragma library\s*/, ''), context);
  return context;
}
const Config = library('Config.js');
const Stats = library('Stats.js');
const plain = value => JSON.parse(JSON.stringify(value));
const collect = (...args) => execFileSync('bash', [path.join(plugin, 'stats.sh'), ...args], { encoding: 'utf8', timeout: 5000 });

test('existing zone configurations keep their behavior and disable usage polling', () => {
  const cfg = Config.parse(`separator: " | "
zones:
  - label: IL
    tz: Asia/Jerusalem
    format: "%a %d %b %H:%M"
  - { label: NY, tz: America/New_York }
format: "%I:%M %p"
`);
  assert.equal(cfg.showCpu || cfg.showMemory || cfg.showDisk, false);
  assert.equal(cfg.zones[0].format, '%a %d %b %H:%M');
  assert.equal(cfg.zones[1].format, '%I:%M %p');
  assert.equal(cfg.separator, ' | ');
  assert.equal(Config.parse(fs.readFileSync(path.join(plugin, 'omaclock.yaml'), 'utf8')).zones.length, 1);
});

test('usage settings work before or after zones, including stats-only configs', () => {
  const cfg = Config.parse(`showCpu: true
zones: []
showMemory: true # memory
showDisk: false
statsInterval: 2
diskPath: "/home/a path"
`);
  assert.equal(cfg.showCpu, true);
  assert.equal(cfg.showMemory, true);
  assert.equal(cfg.showDisk, false);
  assert.equal(cfg.statsInterval, 2);
  assert.equal(cfg.diskPath, '/home/a path');
  assert.equal(cfg.zones.length, 0);
});

test('invalid settings report their line rather than silently enabling a metric', () => {
  for (const line of ['showCpu: yes', 'showMemory: 1', 'showDisk:',
    'statsInterval: 0', 'statsInterval: 1.5', 'statsInterval: 3601',
    'statsInterval: nope', 'diskPath: relative', 'diskPath: "~/data"',
    'diskPath: "/a\\nb"', 'showCpus: true']) {
    assert.throws(() => Config.parse('# comment\n' + line), /line 2:/, line);
  }
  assert.equal(Config.parse('statsInterval: 1').statsInterval, 1);
  assert.equal(Config.parse('statsInterval: 3600').statsInterval, 3600);
});

test('CPU percentage uses deltas, handles warmup and rejects resets', () => {
  const previous = { total: 1000, idle: 800 };
  assert.equal(Stats.cpuPercent(previous, { total: 1200, idle: 950 }), 25);
  assert.equal(Stats.cpuPercent(previous, { total: 1200, idle: 800 }), 100);
  assert.equal(Stats.cpuPercent(previous, { total: 1200, idle: 1000 }), 0);
  assert.equal(Stats.cpuPercent(null, previous), null);
  assert.equal(Stats.cpuPercent(previous, null), null);
  assert.equal(Stats.cpuPercent(previous, previous), null);
  assert.equal(Stats.cpuPercent(previous, { total: 100, idle: 80 }), null);
  assert.equal(Stats.cpuPercent(previous, { total: 1100, idle: 950 }), null);
});

test('memory uses available RAM; disk keeps df percentage and available space', () => {
  const sample = Stats.parse('cpu 1200 950\nmemory 1000 600\ndisk 1000 600 300 67\n');
  assert.deepEqual(plain(sample.cpu), { total: 1200, idle: 950 });
  assert.deepEqual(plain(sample.memory), { total: 1000, used: 400, available: 600, percent: 40 });
  assert.deepEqual(plain(sample.disk), { total: 1000, used: 600, available: 300, percent: 67 });
  assert.equal(Stats.parse('disk 1000 950 -50 106').disk.available, -50);
  assert.deepEqual(plain(Stats.size(1572864)), { value: 1.5, unit: 'GiB' });
});

test('missing and invalid readings stay unavailable independently', () => {
  assert.deepEqual(plain(Stats.parse('')), { cpu: null, memory: null, disk: null });
  assert.deepEqual(plain(Stats.parse('cpu 1 2\nmemory 100 101\ndisk 0 0 0 0')), { cpu: null, memory: null, disk: null });
  const sample = Stats.parse('cpu nope 1\nmemory 100 60\ndisk 10 5 5 NaN');
  assert.equal(sample.cpu, null);
  assert.equal(sample.memory.percent, 40);
  assert.equal(sample.disk, null);
});

test('collector reads live Linux metrics and honors disabled metrics', async () => {
  assert.equal(collect('false', 'false', 'false', '/'), '');
  const before = Stats.parse(collect('true', 'true', 'true', '/'));
  assert.ok(before.cpu.total > 0);
  assert.ok(before.memory.total > 0);
  assert.ok(before.memory.percent >= 0 && before.memory.percent <= 100);
  assert.ok(before.disk.total > 0);
  await new Promise(resolve => setTimeout(resolve, 100));
  const after = Stats.parse(collect('true', 'false', 'false', '/'));
  const cpu = Stats.cpuPercent(before.cpu, after.cpu);
  assert.ok(cpu !== null && cpu >= 0 && cpu <= 100);
  assert.equal(after.memory, null);
  assert.equal(after.disk, null);
});

test('a missing disk path leaves CPU and memory available', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'omaclock-test-'));
  try {
    const sample = Stats.parse(collect('true', 'true', 'true', path.join(dir, 'missing')));
    assert.ok(sample.cpu);
    assert.ok(sample.memory);
    assert.equal(sample.disk, null);
  } finally {
    fs.rmSync(dir, { recursive: true });
  }
});

test('disk paths with spaces, quotes, and shell syntax stay literal arguments', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'omaclock-test-'));
  try {
    const diskPath = path.join(dir, "space ' $(false) ; & <img>");
    fs.mkdirSync(diskPath);
    assert.equal(Config.parse('diskPath: "' + diskPath + '"').diskPath, diskPath);
    const sample = Stats.parse(collect('false', 'false', 'true', diskPath));
    assert.ok(sample.disk.total > 0);
    assert.equal(sample.cpu, null);
    assert.equal(sample.memory, null);
  } finally {
    fs.rmSync(dir, { recursive: true });
  }
});
