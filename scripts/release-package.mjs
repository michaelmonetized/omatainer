#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const architectures = new Map([[183, 'arm64'], [62, 'x64']]);

/** Hash a regular file. Takes its path; returns SHA-256 using bounded read storage. */
export function digest(file) {
    const stat = fs.lstatSync(file);
    if (!stat.isFile() || stat.isSymbolicLink()) throw new Error(`Expected a regular file: ${file}`);
    const hash = crypto.createHash('sha256'), buffer = Buffer.allocUnsafe(128 * 1024), fd = fs.openSync(file, 'r');
    try { for (;;) { const count = fs.readSync(fd, buffer, 0, buffer.length, null); if (!count) break; hash.update(buffer.subarray(0, count)); } }
    finally { fs.closeSync(fd); }
    return hash.digest('hex');
}

/** Read an executable platform. Takes a Linux ELF file; returns its supported architecture or refuses another format. */
function architecture(file) {
    const header = Buffer.alloc(20), fd = fs.openSync(file, 'r');
    try { if (fs.readSync(fd, header, 0, 20, 0) !== 20) throw new Error('Truncated executable'); }
    finally { fs.closeSync(fd); }
    if (!header.subarray(0, 4).equals(Buffer.from([127, 69, 76, 70])) || header[4] !== 2 || header[5] !== 1 || !architectures.has(header.readUInt16LE(18))) throw new Error('Package requires a supported 64-bit little-endian Linux ELF executable');
    return architectures.get(header.readUInt16LE(18));
}

/** Flush a directory entry. Takes an existing directory; returns after its entries reach the filesystem. */
function syncDirectory(directory) {
    const fd = fs.openSync(directory, 'r');
    try { fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
}

/** Write a durable file. Takes a new path, bytes and permissions; refuses replacement and returns after flushing. */
function write(file, content, mode = 0o600) {
    const fd = fs.openSync(file, 'wx', mode);
    try { fs.writeFileSync(fd, content); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
}

/** Package a source-identified executable. Takes its executable, a new directory and exact source revision; returns the manifest checksum. */
export function pack(binary, directory, revision) {
    if (!/^[a-f0-9]{40}$/.test(revision)) throw new Error('Exact source revision required');
    const arch = architecture(binary);
    const versions = execFileSync('readelf', ['--version-info', binary], { encoding: 'utf8' });
    const glibc = [...versions.matchAll(/GLIBC_(\d+)\.(\d+)/g)].map(match => [Number(match[1]), Number(match[2])]).sort((a, b) => b[0] - a[0] || b[1] - a[1])[0];
    if (!glibc) throw new Error('Executable libc requirements unavailable');
    fs.mkdirSync(directory, { mode: 0o700 });
    const files = {};
    const license = JSON.parse(fs.readFileSync(path.join(root, 'licenses/manifest.json')));
    const sources = Object.keys(license.package).filter(file => file.startsWith('vendor/')).map(file => [`source/${file}`, path.join(root, file)]);
    for (const [name, source] of [['omatainer', binary], ['update.mjs', fileURLToPath(import.meta.url)], ['LICENSE', path.join(root, 'LICENSE')], ['licenses.json', path.join(root, 'licenses/manifest.json')], ['notices.json', path.join(root, 'licenses/notices.json')], ...sources]) {
        const target = path.join(directory, name);
        fs.mkdirSync(path.dirname(target), { recursive: true });
        fs.copyFileSync(source, target, fs.constants.COPYFILE_EXCL);
        fs.chmodSync(target, name === 'omatainer' ? 0o755 : 0o644);
        const fd = fs.openSync(target, 'r'); try { fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
        files[name] = { sha256: digest(target), bytes: fs.statSync(target).size };
    }
    const version = fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8').match(/^version = "([^"]+)"/m)?.[1];
    const schema = (file, name) => {
        const value = fs.readFileSync(path.join(root, file), 'utf8').match(new RegExp(`const ${name}: u32 = (\\d+);`))?.[1];
        if (!value) throw new Error(`Missing migration schema: ${file}`);
        return Number(value);
    };
    const manifest = { schema: 1, application: 'omatainer', version: `${version}+${revision.slice(0, 12)}`, revision,
        platform: { os: 'linux', architecture: arch, glibc: glibc.join('.'), installer: 'Node.js 22 or newer' },
        migration: { preferences: schema('src/preferences/mod.rs', 'VERSION'), project: schema('src/engine/project/model.rs', 'STATE_VERSION'), policy: 'Installation preserves preferences, projects, libraries and plugins. Opening documents may migrate their format; retain document backups when rolling back.', automatic: false }, files };
    write(path.join(directory, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n', 0o644);
    syncDirectory(directory);
    return digest(path.join(directory, 'manifest.json'));
}

/** Verify a trusted package checksum and all files. Takes a directory and independently obtained manifest SHA-256; returns its validated manifest. */
export function verify(directory, expected) {
    if (fs.lstatSync(path.join(directory, 'manifest.json')).size > 2 * 1024 * 1024 || !/^[a-f0-9]{64}$/.test(expected) || digest(path.join(directory, 'manifest.json')) !== expected) throw new Error('Package manifest checksum differs from the expected release checksum');
    const manifest = JSON.parse(fs.readFileSync(path.join(directory, 'manifest.json'), 'utf8'));
    if (manifest.schema !== 1 || manifest.application !== 'omatainer' || !/^[a-f0-9]{40}$/.test(manifest.revision) || manifest.platform.os !== process.platform || manifest.platform.architecture !== process.arch || !manifest.migration || manifest.migration.automatic !== false) throw new Error('Unsupported package platform or migration policy');
    for (const [name, record] of Object.entries(manifest.files)) {
        if (name.startsWith('/') || name.split('/').some(part => !part || part === '.' || part === '..') || !/^[a-zA-Z0-9_.\/-]+$/.test(name)) throw new Error('Unsafe package file name');
        const file = path.join(directory, name);
        if (!Number.isSafeInteger(record.bytes) || record.bytes < 1 || record.bytes > 512 * 1024 * 1024 || fs.lstatSync(file).size !== record.bytes || digest(file) !== record.sha256) throw new Error(`Package file failed verification: ${name}`);
        if (!fs.realpathSync(file).startsWith(fs.realpathSync(directory) + path.sep)) throw new Error('Package file escapes its directory');
    }
    const license = JSON.parse(fs.readFileSync(path.join(directory, 'licenses.json')));
    const expectedFiles = ['LICENSE', 'licenses.json', 'notices.json', 'omatainer', 'update.mjs', ...Object.keys(license.package).filter(file => file.startsWith('vendor/')).map(file => `source/${file}`)];
    if (JSON.stringify(Object.keys(manifest.files).sort()) !== JSON.stringify(expectedFiles.sort())) throw new Error('Package file inventory differs from the supported release shape');
    if (architecture(path.join(directory, 'omatainer')) !== process.arch) throw new Error('Executable architecture differs from manifest');
    const current = process.report.getReport().header.glibcVersionRuntime;
    const version = value => { if (!/^\d+\.\d+$/.test(value || '')) throw new Error('glibc version unavailable'); return value.split('.').map(Number); };
    const required = version(manifest.platform.glibc), available = version(current);
    if (available[0] < required[0] || available[0] === required[0] && available[1] < required[1]) throw new Error(`Release requires glibc ${manifest.platform.glibc}`);
    return manifest;
}

/** Defer installation until the app closes. Takes the install path; refuses an instance owned by this user using that executable. */
function stopped(destination) {
    for (const pid of fs.readdirSync('/proc').filter(name => /^\d+$/.test(name))) {
        try {
            if (fs.statSync(`/proc/${pid}`).uid !== process.getuid()) continue;
            const executable = fs.readlinkSync(`/proc/${pid}/exe`).replace(/ \(deleted\)$/, '');
            if (executable === destination || executable === destination + '.previous') throw new Error(`Close Omatainer before updating or rolling back (PID ${pid}); active performances are deferred`);
        } catch (error) { if (!['ENOENT', 'EACCES', 'EPERM'].includes(error.code)) throw error; }
    }
}

/** Validate the current install inode. Takes a path; returns its regular owned file or null when absent. */
function currentFile(destination) {
    let stat;
    try { stat = fs.lstatSync(destination); } catch (error) { if (error.code === 'ENOENT') return null; throw error; }
    if (!stat.isFile() || stat.isSymbolicLink() || stat.uid !== process.getuid()) throw new Error('Install destination must be a regular executable owned by this user');
    return stat;
}

/** Publish one executable atomically under the caller's install lock. Takes source, destination, expected digest and optional fault hook; returns a durable rollback receipt. */
export function publish(source, destination, expected, fault = () => {}) {
    destination = path.resolve(destination);
    if (digest(source) !== expected || architecture(source) !== process.arch) throw new Error('Update source failed verification');
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    currentFile(destination);
    stopped(destination);
    const journal = destination + '.updates';
    fs.mkdirSync(journal, { recursive: true, mode: 0o700 });
    const journalStat = fs.lstatSync(journal);
    if (!journalStat.isDirectory() || journalStat.isSymbolicLink() || journalStat.uid !== process.getuid()) throw new Error('Unsafe update journal');
    const staged = path.join(path.dirname(destination), `.${path.basename(destination)}.update-${process.pid}-${crypto.randomUUID()}`);
    let committed = false, before;
    try {
        stopped(destination);
        before = currentFile(destination) ? digest(destination) : null;
        if (before) {
            const backup = path.join(journal, before);
            if (!fs.existsSync(backup)) { fs.copyFileSync(destination, backup, fs.constants.COPYFILE_EXCL); fs.chmodSync(backup, 0o755); }
            if (digest(backup) !== before) throw new Error('Rollback executable failed verification');
            const fd = fs.openSync(backup, 'r'); try { fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
        }
        fs.copyFileSync(source, staged, fs.constants.COPYFILE_EXCL);
        fs.chmodSync(staged, 0o755);
        if (digest(staged) !== expected) throw new Error('Staged executable differs from the package');
        const fd = fs.openSync(staged, 'r'); try { fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
        const receipt = { schema: 1, before, after: expected, installedAt: new Date().toISOString() };
        write(path.join(journal, `${expected}-${crypto.randomUUID()}.json`), JSON.stringify(receipt) + '\n');
        syncDirectory(journal);
        fault('before_publish');
        stopped(destination);
        fs.renameSync(staged, destination);
        committed = true;
        fault('after_publish');
        syncDirectory(path.dirname(destination));
        return receipt;
    } catch (error) {
        if (committed) {
            if (before) {
                fs.copyFileSync(path.join(journal, before), staged, fs.constants.COPYFILE_EXCL);
                fs.chmodSync(staged, 0o755);
                const fd = fs.openSync(staged, 'r'); try { fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
                fs.renameSync(staged, destination);
            }
            else fs.unlinkSync(destination);
            syncDirectory(path.dirname(destination));
        }
        throw error;
    } finally {
        if (fs.existsSync(staged)) fs.unlinkSync(staged);
    }
}

/** Hold an OS install lock for one command. Takes destination and child arguments; returns its output, releasing the lock even after a crash. */
export function locked(destination, args) {
    destination = path.resolve(destination);
    const journal = destination + '.updates';
    fs.mkdirSync(journal, { recursive: true, mode: 0o700 });
    const stat = fs.lstatSync(journal);
    if (!stat.isDirectory() || stat.isSymbolicLink() || stat.uid !== process.getuid()) throw new Error('Unsafe update journal');
    const lock = path.join(journal, 'lock');
    currentFile(lock);
    return execFileSync('flock', ['-n', '-E', '75', lock, process.execPath, fileURLToPath(import.meta.url), ...args], { encoding: 'utf8' });
}

/** Restore a verified prior executable. Takes an install path and optional fault hook; returns the atomic rollback receipt. */
export function rollback(destination, fault) {
    destination = path.resolve(destination);
    const current = digest(destination), journal = destination + '.updates';
    const receipts = fs.readdirSync(journal).filter(name => name.endsWith('.json')).map(name => JSON.parse(fs.readFileSync(path.join(journal, name), 'utf8')))
        .filter(receipt => receipt.schema === 1 && receipt.after === current && /^[a-f0-9]{64}$/.test(receipt.before || '')).sort((a, b) => b.installedAt.localeCompare(a.installedAt));
    if (!receipts.length) throw new Error('No verified previous release for this executable');
    return publish(path.join(journal, receipts[0].before), destination, receipts[0].before, fault);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    const [action, first, second, third] = process.argv.slice(2);
    const destination = third || path.join(os.homedir(), '.local/bin/omatainer');
    if (Number(process.versions.node.split('.')[0]) < 22) throw new Error('Node.js 22 or newer required');
    if (action === 'build') {
        if (!first) throw new Error('Usage: release-package.mjs build NEW_DIRECTORY');
        const clean = () => {
            if (execFileSync('git', ['status', '--porcelain'], { cwd: root, encoding: 'utf8' }).trim()) throw new Error('Commit the reviewed source before packaging');
            return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim();
        };
        const revision = clean(), target = path.resolve(process.env.CARGO_TARGET_DIR || path.join(root, 'target'));
        const temporary = path.join(root, 'target/release-packages/tmp');
        if (!target.startsWith('/home/')) throw new Error('Release build outputs must stay on /home');
        fs.mkdirSync(temporary, { recursive: true });
        execFileSync(process.execPath, [path.join(root, 'scripts/refresh-source-inventory.mjs'), 'check'], { cwd: root, stdio: 'inherit' });
        execFileSync('cargo', ['rustc', '--locked', '--release', '--', '-C', `metadata=${revision}`], { cwd: root,
            env: { ...process.env, TMPDIR: temporary, CARGO_TARGET_DIR: target, CARGO_INCREMENTAL: '0' }, stdio: 'inherit' });
        if (clean() !== revision) throw new Error('Source changed during the release build');
        const binary = path.join(target, 'release/omatainer');
        const info = JSON.parse(execFileSync(binary, ['benchmark-build-info'], { encoding: 'utf8' }));
        if (info.debug_assertions || info.os !== 'linux' || info.arch !== (process.arch === 'arm64' ? 'aarch64' : 'x86_64')) throw new Error('Release executable reports an unexpected build profile or platform');
        console.log(pack(binary, path.resolve(first), revision));
    } else if (action === 'verify' || action === 'install' || action === '_install') {
        if (!first || !second) throw new Error('Usage: release-package.mjs verify|install PACKAGE EXPECTED_MANIFEST_SHA256 [DESTINATION]');
        const manifest = verify(path.resolve(first), second);
        if (action === 'install') process.stdout.write(locked(destination, ['_install', first, second, destination]));
        else console.log(JSON.stringify(action === 'verify' ? manifest : publish(path.join(path.resolve(first), 'omatainer'), destination, manifest.files.omatainer.sha256)));
    } else if (action === 'rollback') process.stdout.write(locked(first || destination, ['_rollback', first || destination]));
    else if (action === '_rollback') console.log(JSON.stringify(rollback(first || destination)));
    else throw new Error('Usage: release-package.mjs build|verify|install|rollback');
}
