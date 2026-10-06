#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

/** Validate the app backlog. Takes a registry; returns dependency-ordered work or refuses missing and circular dependencies. */
export function order(registry) {
    if (registry.schema !== 1 || !Array.isArray(registry.issues)) throw new Error('Unsupported app checklist');
    const rows = new Map(registry.issues.map(row => [row.issue, row]));
    if (rows.size !== 356 || registry.issues.some((row, index) => row.issue !== index + 2)) throw new Error('Checklist must contain every issue from 2 through 357 once, in order');
    const visited = new Set(), visiting = new Set(), result = [];
    const visit = issue => {
        if (visited.has(issue)) return;
        if (visiting.has(issue)) throw new Error(`Circular dependency at #${issue}`);
        const row = rows.get(issue);
        if (!row) throw new Error(`Missing dependency #${issue}`);
        if (!['existing stack', 'planned', 'partial', 'implemented', 'accepted', 'outside release'].includes(row.state)) throw new Error(`Invalid state at #${issue}`);
        if (row.state === 'accepted' && !row.evidence.length) throw new Error(`Acceptance evidence missing at #${issue}`);
        for (const evidence of row.evidence) {
            const target = path.resolve(root, evidence);
            if (!target.startsWith(root + path.sep) || !fs.existsSync(target)) throw new Error(`Unavailable evidence at #${issue}`);
        }
        visiting.add(issue);
        for (const dependency of row.dependencies) visit(dependency);
        visiting.delete(issue);
        visited.add(issue);
        result.push(row);
    };
    for (const row of registry.issues) visit(row.issue);
    return result;
}

/** Render one checklist. Takes a validated registry; returns Markdown with software state and linked acceptance evidence. */
export function render(registry) {
    const rows = order(registry);
    const counts = Object.fromEntries(['existing stack', 'implemented', 'partial', 'planned', 'accepted', 'outside release'].map(state => [state, rows.filter(row => row.state === state).length]));
    const lines = ['# App completion checklist', '',
        `Reviewed ${registry.reviewed}. Source base: \`${registry.base}\`. Tracking: [#1](https://github.com/michaelmonetized/omatainer/issues/1).`, '',
        registry.hardwareQualification, '',
        'Each issue appears once, after its dependencies. The Code column checks off implemented code, including the existing stack, and does not claim complete acceptance. Accepted requires complete recorded acceptance. The eight provider integrations previously excluded by Michael stay outside this release. No open issue is automatically closed by this checklist.', '',
        Object.entries(counts).map(([state, count]) => `${count} ${state}`).join(' · '), '',
        `- [${registry.waveform?.state === 'implemented' ? 'x' : ' '}] High-resolution stereo source peaks with measured rainbow frequency bands, native-rate validation, project reopen and pixel-scaled rendering. [Receipt](../validation/rainbow-waveforms.md)`, '',
        '| Code | Issue | Capability | State | Depends on | Evidence / existing PRs |',
        '| --- | --- | --- | --- | --- | --- |'];
    for (const row of rows) {
        const links = [...row.evidence.map(file => `[receipt](../../${file})`), ...row.prs.map(url => `[#${url.split('/').at(-1)}](${url})`)];
        lines.push(`| ${['accepted', 'implemented', 'existing stack'].includes(row.state) ? '[x]' : '[ ]'} | [#${row.issue}](${row.url}) | ${row.title.replaceAll('|', '\\|')} | ${row.state} | ${row.dependencies.map(issue => `#${issue}`).join(', ')} | ${links.join(', ')} |`);
    }
    return lines.join('\n') + '\n';
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    const registry = JSON.parse(fs.readFileSync(path.join(root, 'docs/backlog/app-checklist.json'), 'utf8'));
    const result = render(registry), destination = path.join(root, 'docs/backlog/app-checklist.md');
    const action = process.argv[2] || 'check';
    if (action === 'generate') fs.writeFileSync(destination, result);
    else if (action !== 'check' || fs.readFileSync(destination, 'utf8') !== result) throw new Error('App checklist is stale; run node scripts/app-checklist.mjs generate');
    console.log(`App checklist ${action}: ${registry.issues.length} issues, dependency order verified`);
}
