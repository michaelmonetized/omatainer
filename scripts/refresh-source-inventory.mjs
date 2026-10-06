#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { verifyGraph } from './retained-dependency-graph.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const file = path.join(root, 'licenses/manifest.json');
const manifest = JSON.parse(fs.readFileSync(file));
const sha = source => crypto.createHash('sha256').update(fs.readFileSync(path.join(root, source))).digest('hex');
if (process.argv.includes('--verify-unchanged-dependencies')) {
    const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--offline', '--format-version', '1', '--filter-platform', manifest.target], { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 }));
    verifyGraph(metadata, fs.readFileSync(path.join(root, 'Cargo.lock'), 'utf8'), manifest.cargo, root);
} else for (const file of ['Cargo.lock', 'Cargo.toml']) if (manifest.source_files[file] !== sha(file)) throw new Error('Dependency configuration changed; refresh complete dependency license records first');
for (const row of manifest.cargo) for (const [file, expected] of Object.entries(row.vendored_source?.files || {})) {
    if (sha(file) !== expected) throw new Error('Vendored dependency changed; refresh its complete license/source records first');
}
const walk = directory => fs.readdirSync(path.join(root, directory), { withFileTypes: true }).flatMap(entry => {
    const file = path.join(directory, entry.name);
    if (entry.isSymbolicLink()) throw new Error(`Unexpected source symlink: ${file}`);
    return entry.isDirectory() ? walk(file) : entry.isFile() ? [file] : [];
});
const files = new Set(Object.keys(manifest.source_files).filter(file => fs.existsSync(path.join(root, file))));
for (const directory of ['src', 'tests', 'benchmarks', 'vendor', 'locales', 'scripts', 'docs/backlog']) for (const file of walk(directory)) files.add(file);
manifest.source_files = Object.fromEntries([...files].sort().map(file => [file, sha(file)]));
const canonical = value => Array.isArray(value) ? value.map(canonical) : value && typeof value === 'object'
    ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
if (process.argv[2] === 'generate') fs.writeFileSync(file, JSON.stringify(canonical(manifest), null, 2) + '\n');
else if (process.argv[2] !== 'check' || JSON.stringify(JSON.parse(fs.readFileSync(file)).source_files) !== JSON.stringify(manifest.source_files)) throw new Error('Source inventory is stale; run node scripts/refresh-source-inventory.mjs generate');
console.log(`Source inventory ${process.argv[2]}: ${files.size} files; dependency records preserved`);
