import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { load, render, verifyFixtureNames } from './capability-matrix.mjs';
import { order } from './app-checklist.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const directory = path.join(root, 'target/app-backlog/matrix-tests');
fs.mkdirSync(directory, { recursive: true });
const fixture = fs.mkdtempSync(path.join(directory, 'run-'));
after(() => fs.rmSync(fixture, { recursive: true }));
fs.cpSync(path.join(root, 'docs/backlog'), path.join(fixture, 'docs/backlog'), { recursive: true });
const read = name => JSON.parse(fs.readFileSync(path.join(fixture, 'docs/backlog', name)));
const write = (name, value) => fs.writeFileSync(path.join(fixture, 'docs/backlog', name), JSON.stringify(value));
for (const record of Object.values(read('capability-status.json').issues)) {
    for (const route of record.paths) {
        const target = path.join(fixture, route.file);
        fs.mkdirSync(path.dirname(target), { recursive: true }); fs.copyFileSync(path.join(root, route.file), target);
    }
}

test('all remaining issues and excluded releases stay visible without acceptance inflation', () => {
    const text = render(fixture);
    for (let number = 129; number <= 357; number++) assert.ok(text.includes(`| [#${number}](https://github.com/michaelmonetized/omatainer/issues/${number})`));
    for (const number of read('release-scope.json').excluded_issues) assert.ok(text.split('\n').find(line => line.startsWith(`| [#${number}]`)).endsWith('| excluded from release |'));
    assert.ok(!text.includes('| accepted |'));
});

test('missing, duplicate and reordered inventories refuse generation', () => {
    const rows = read('remaining-issues.json');
    for (const changed of [rows.slice(0, -1), [rows[1], rows[0], ...rows.slice(2)], [rows[0], ...rows.slice(0, -1)]]) {
        write('remaining-issues.json', changed); assert.throws(() => load(fixture), /inventory/);
    }
    write('remaining-issues.json', rows);
});

test('excluded providers, unavailable routes and unsupported acceptance claims are refused', () => {
    const registry = read('capability-status.json');
    const changed = structuredClone(registry); changed.issues['158'] = changed.issues['131'];
    write('capability-status.json', changed); assert.throws(() => load(fixture), /excluded/);
    for (const file of ['src/nonexistent.rs', '../../outside.rs']) {
        const changed = structuredClone(registry); changed.issues['131'].paths[0].file = file;
        write('capability-status.json', changed); assert.throws(() => load(fixture), /source route/);
    }
    const accepted = structuredClone(registry); accepted.issues['131'].status = 'accepted';
    write('capability-status.json', accepted); assert.throws(() => load(fixture), /complete evidence/);
    accepted.issues['131'].acceptance = 'all criteria passed';
    accepted.issues['131'].evidence = ['docs/validation/missing.md'];
    write('capability-status.json', accepted); assert.throws(() => load(fixture), /acceptance evidence/);
    write('capability-status.json', registry);
});

test('missing and misspelled Rust selectors cannot masquerade as executed tests', () => {
    const status = { issues: { 131: read('capability-status.json').issues['131'] } };
    const names = status.issues[131].fixtures.map(fixture => fixture + '::actual_case');
    verifyFixtureNames(status, names);
    for (const changed of [[], names.slice(0, -1), names.map(name => name.replace('deck_load_lock', 'misspelled'))]) assert.throws(() => verifyFixtureNames(status, changed), /no compiled tests/);
});

test('the single app checklist preserves every ticket and rejects missing or circular dependencies', () => {
    const registry = JSON.parse(fs.readFileSync(path.join(root, 'docs/backlog/app-checklist.json')));
    const rows = order(registry), positions = new Map(rows.map((row, index) => [row.issue, index]));
    assert.equal(rows.length, 356);
    for (const row of rows) for (const dependency of row.dependencies) assert.ok(positions.get(dependency) < positions.get(row.issue));
    const missing = structuredClone(registry); missing.issues.pop(); assert.throws(() => order(missing), /every issue/);
    const circular = structuredClone(registry); circular.issues[0].dependencies = [3]; circular.issues[1].dependencies = [2]; assert.throws(() => order(circular), /Circular/);
    const unavailable = structuredClone(registry); unavailable.issues[0].dependencies = [9999]; assert.throws(() => order(unavailable), /Missing dependency/);
});
