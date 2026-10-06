import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { digest, pack, verify, publish, rollback } from './release-package.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const artifacts = path.join(root, 'target/app-backlog/update-tests');
fs.mkdirSync(artifacts, { recursive: true });
const directory = fs.mkdtempSync(path.join(artifacts, 'run-'));
after(() => fs.rmSync(directory, { recursive: true }));
const revision = 'a'.repeat(40);

test('clean package install, upgrade and rollback preserve user files and verified old binaries', () => {
    const first = path.join(directory, 'first'), second = path.join(directory, 'second');
    const firstSha = pack('/usr/bin/true', first, revision), secondSha = pack('/usr/bin/false', second, 'b'.repeat(40));
    const a = verify(first, firstSha), b = verify(second, secondSha);
    const installed = path.join(directory, 'app/omatainer'), project = path.join(directory, 'project.omat');
    fs.writeFileSync(project, 'saved user project');
    assert.equal(publish(path.join(first, 'omatainer'), installed, a.files.omatainer.sha256).before, null);
    const receipt = publish(path.join(second, 'omatainer'), installed, b.files.omatainer.sha256);
    assert.equal(receipt.before, a.files.omatainer.sha256);
    assert.equal(digest(installed), b.files.omatainer.sha256);
    rollback(installed);
    assert.equal(digest(installed), a.files.omatainer.sha256);
    assert.equal(fs.readFileSync(project, 'utf8'), 'saved user project');
});

test('changed manifests, executable corruption, symlinked files and foreign platforms refuse installation', () => {
    for (const [name, change] of [
        ['manifest', dir => fs.appendFileSync(path.join(dir, 'manifest.json'), ' ')],
        ['binary', dir => fs.appendFileSync(path.join(dir, 'omatainer'), 'corrupt')],
        ['symlink', dir => { fs.unlinkSync(path.join(dir, 'omatainer')); fs.symlinkSync('/usr/bin/true', path.join(dir, 'omatainer')); }],
    ]) {
        const dir = path.join(directory, name), sha = pack('/usr/bin/true', dir, revision);
        change(dir);
        assert.throws(() => verify(dir, sha));
    }
    const dir = path.join(directory, 'foreign'), sha = pack('/usr/bin/true', dir, revision);
    assert.throws(() => verify(dir, '0'.repeat(64)), /checksum/);
    const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json')));
    manifest.platform.architecture = 'unsupported';
    fs.writeFileSync(path.join(dir, 'manifest.json'), JSON.stringify(manifest));
    assert.throws(() => verify(dir, digest(path.join(dir, 'manifest.json'))), /platform/);
    assert.notEqual(sha, digest(path.join(dir, 'manifest.json')));
});

test('failed update and failed rollback restore the exact previously installed executable', () => {
    const installed = path.join(directory, 'faults/omatainer'), a = digest('/usr/bin/true'), b = digest('/usr/bin/false');
    publish('/usr/bin/true', installed, a);
    for (const point of ['before_publish', 'after_publish']) {
        assert.throws(() => publish('/usr/bin/false', installed, b, stage => { if (stage === point) throw new Error('injected update fault'); }), /injected/);
        assert.equal(digest(installed), a);
    }
    publish('/usr/bin/false', installed, b);
    assert.throws(() => rollback(installed, stage => { if (stage === 'after_publish') throw new Error('injected rollback fault'); }), /injected/);
    assert.equal(digest(installed), b);
    rollback(installed);
    assert.equal(digest(installed), a);
});

test('unsafe destinations cannot replace user paths', () => {
    const installed = path.join(directory, 'unsafe');
    fs.symlinkSync(path.join(directory, 'does-not-exist'), installed);
    assert.throws(() => publish('/usr/bin/true', installed, digest('/usr/bin/true')), /regular/);
    assert.ok(fs.lstatSync(installed).isSymbolicLink());
});

test('concurrent CLI installers refuse the OS lock and crash releases permit a retry', async () => {
    const pkg = path.join(directory, 'concurrent-package'), sha = pack('/usr/bin/true', pkg, revision);
    const installed = path.join(directory, 'concurrent/omatainer'), lock = installed + '.updates/lock';
    fs.mkdirSync(path.dirname(lock), { recursive: true });
    const child = spawn('flock', ['-n', lock, process.execPath, '-e', 'process.stdout.write("locked");setInterval(()=>{},1000)'], { stdio: ['ignore', 'pipe', 'ignore'] });
    await new Promise((resolve, reject) => { child.stdout.once('data', resolve); child.once('error', reject); });
    const args = [path.join(root, 'scripts/release-package.mjs'), 'install', pkg, sha, installed];
    try {
        const result = spawnSync(process.execPath, args, { encoding: 'utf8' });
        assert.notEqual(result.status, 0);
        assert.equal(fs.existsSync(installed), false);
    } finally {
        const children = fs.readFileSync(`/proc/${child.pid}/task/${child.pid}/children`, 'utf8').trim().split(/\s+/).filter(Boolean);
        for (const pid of children) process.kill(Number(pid), 'SIGKILL');
        child.kill('SIGKILL');
        await new Promise(resolve => child.once('exit', resolve));
    }
    const result = spawnSync(process.execPath, args, { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(digest(installed), digest('/usr/bin/true'));
});

test('an active app process defers replacement until it closes', async () => {
    const installed = path.join(directory, 'live/omatainer');
    publish('/usr/bin/sleep', installed, digest('/usr/bin/sleep'));
    const child = spawn(installed, ['30'], { stdio: 'ignore' });
    await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject); });
    try {
        assert.throws(() => publish('/usr/bin/true', installed, digest('/usr/bin/true')), /Close Omatainer/);
        assert.equal(digest(installed), digest('/usr/bin/sleep'));
    } finally { child.kill(); await new Promise(resolve => child.once('exit', resolve)); }
    publish('/usr/bin/true', installed, digest('/usr/bin/true'));
    assert.equal(digest(installed), digest('/usr/bin/true'));
});
