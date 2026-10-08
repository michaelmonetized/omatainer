#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import cp from 'node:child_process';
import crypto from 'node:crypto';
import { setTimeout as sleep } from 'node:timers/promises';

const [mode, directory, application, tests, user] = process.argv.slice(2);
const root = path.resolve(directory || '');
const project = path.resolve(import.meta.dirname, '..');
if (!root.startsWith(project + '/t/') || root.includes('\n')) throw Error('Keep this private volume under the project t directory');

/** Run a bounded native command. Takes executable and literal arguments; returns its stdout or stops qualification on failure. */
function run(command, args, timeout = 15000) {
    return cp.execFileSync(command, args, { encoding: 'utf8', timeout, maxBuffer: 1024 * 1024 });
}
/** Wait for an owned test phase. Takes a marker and child; refuses failed children and missing acknowledgment after twenty seconds. */
async function phase(name, child) {
    const deadline = Date.now() + 20000;
    while (!fs.existsSync(path.join(root, name))) {
        if (child.exitCode !== null || child.signalCode !== null) throw Error('Native volume fixture exited before ' + name);
        if (Date.now() > deadline) throw Error('Native volume fixture exceeded its ' + name + ' deadline');
        await sleep(20);
    }
}
if (mode === 'run') {
    fs.mkdirSync(root, { mode: 0o700 });
    const owner = run('id', ['-un']).trim();
    run('sudo', ['-n', 'unshare', '--mount', '--propagation', 'private', process.execPath, import.meta.filename,
        '_worker', root, path.resolve(application), path.resolve(tests), owner], 150000);
    console.log(fs.readFileSync(path.join(root, 'volume-remount.json'), 'utf8'));
} else if (mode === '_worker') {
    if (process.getuid() !== 0 || !/^[a-z_][a-z0-9_-]*[$]?$/.test(user || '') || fs.realpathSync(root) !== root) throw Error('Private namespace worker requires the original owner and an owned real path');
    const image = path.join(root, 'volume.ext4');
    const first = path.join(root, 'first'), second = path.join(root, 'second'), temporary = path.join(root, 'temporary');
    for (const folder of [first, second, temporary]) fs.mkdirSync(folder, { mode: 0o755 });
    run('chown', [user, temporary]);
    const fd = fs.openSync(image, 'wx', 0o600);
    fs.ftruncateSync(fd, 64 * 1024 * 1024);
    fs.closeSync(fd);
    run('mkfs.ext4', ['-q', '-F', image]);
    let loop, mounted, child, error;
    const started = new Date().toISOString();
    try {
        loop = run('losetup', ['--find', '--show', image]).trim();
        if (!/^\/dev\/loop[0-9]+$/.test(loop)) throw Error('Unexpected owned loop device');
        run('udevadm', ['settle', '--timeout=10']);
        run('mount', ['-o', 'rw,nodev,nosuid,noexec', loop, first]); mounted = first;
        fs.mkdirSync(path.join(first, 'music'));
        const frames = 4410, data = Buffer.alloc(44 + frames * 4);
        data.write('RIFF'); data.writeUInt32LE(data.length - 8, 4); data.write('WAVEfmt ', 8);
        data.writeUInt32LE(16, 16); data.writeUInt16LE(1, 20); data.writeUInt16LE(2, 22);
        data.writeUInt32LE(44100, 24); data.writeUInt32LE(44100 * 4, 28); data.writeUInt16LE(4, 32);
        data.writeUInt16LE(16, 34); data.write('data', 36); data.writeUInt32LE(frames * 4, 40);
        for (let frame = 0; frame < frames; frame++) for (let channel = 0; channel < 2; channel++) data.writeInt16LE(Math.round(Math.sin(frame * 2 * Math.PI * 440 / 44100) * 1000), 44 + frame * 4 + channel * 2);
        fs.writeFileSync(path.join(first, 'music/track.wav'), data);
        fs.writeFileSync(path.join(first, 'owned.m3u8'), '#EXTM3U\nmusic/track.wav\n');
        run('mount', ['-o', 'remount,ro,nodev,nosuid,noexec', first]);
        const output = fs.openSync(path.join(root, 'native.log'), 'wx');
        child = cp.spawn('runuser', ['-u', user, '--', 'env', 'RUST_MIN_STACK=67108864', 'TMPDIR=' + temporary,
            'OMATAINER_VOLUME_DIR=' + root, 'OMATAINER_TEST_BIN=' + application, tests,
            'dj_library::tests::native_read_only_volume_remount_retains_uuid_and_discovers_new_mount_path',
            '--ignored', '--exact', '--test-threads=1', '--nocapture'], { stdio: ['ignore', output, output], detached: true });
        fs.closeSync(output);
        await phase('first.ready', child);
        run('umount', [first]); mounted = undefined;
        fs.writeFileSync(path.join(root, 'offline.ready'), 'Owned volume unmounted in private namespace\n');
        await phase('offline.checked', child);
        run('mount', ['-o', 'ro,nodev,nosuid,noexec', loop, second]); mounted = second;
        fs.writeFileSync(path.join(root, 'remounted.ready'), 'Same owned volume at a different path\n');
        await phase('volume-remount.json', child);
        const deadline = Date.now() + 5000;
        while (child.exitCode === null && child.signalCode === null && Date.now() < deadline) await sleep(20);
        if (child.exitCode !== 0) throw Error('Native volume fixture failed or did not retire');
    } catch (caught) { error = caught; }
    finally {
        if (child && child.exitCode === null && child.signalCode === null) {
            process.kill(-child.pid, 'SIGTERM');
            for (let attempt = 0; attempt < 100 && child.exitCode === null && child.signalCode === null; attempt++) await sleep(20);
            if (child.exitCode === null && child.signalCode === null) process.kill(-child.pid, 'SIGKILL');
        }
        if (mounted) run('umount', [mounted]);
        if (loop) {
            if (path.resolve(run('losetup', ['--noheadings', '--output', 'BACK-FILE', loop]).trim()) !== image) throw Error('Refusing to detach a changed loop device');
            run('losetup', ['--detach', loop]);
        }
        fs.writeFileSync(path.join(root, 'driver.json'), JSON.stringify({ status: error ? 'failed' : 'pass', started, finished: new Date().toISOString(), application, tests,
            applicationSha256: crypto.createHash('sha256').update(fs.readFileSync(application)).digest('hex'), testsSha256: crypto.createHash('sha256').update(fs.readFileSync(tests)).digest('hex'),
            child: child && { pid: child.pid, exitCode: child.exitCode, signal: child.signalCode }, privateMountNamespace: true, loopDetached: !!loop, error: error?.message }, null, 2) + '\n');
    }
    if (error) throw error;
} else throw Error('Usage: qualify-mounted-volume.mjs run PROJECT/t/NEW_DIRECTORY NATIVE_APP TEST_BINARY');
