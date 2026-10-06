import path from 'node:path';

/** Verify retained dependency records. Takes Cargo metadata, lock text, retained graph and root; refuses any changed version, feature, license, registry checksum or vendor identity. */
export function verifyGraph(metadata, lock, retained, root) {
    const checksums = new Map(lock.split('[[package]]').slice(1).map(block => {
        const field = key => block.match(new RegExp(`^${key} = "([^"\\n]+)"$`, 'm'))?.[1];
        return [field('name') + '@' + field('version'), field('checksum') || null];
    }));
    const nodes = new Map(metadata.resolve.nodes.map(node => [node.id, node]));
    const known = new Map(retained.map(row => [row.name + '@' + row.version, row]));
    const actual = metadata.packages.filter(pkg => pkg.id !== metadata.resolve.root && nodes.has(pkg.id)).map(pkg => {
        const key = pkg.name + '@' + pkg.version, old = known.get(key);
        if (!checksums.has(key) || !old) throw new Error(`Dependency has no retained complete records: ${key}`);
        const row = { name: pkg.name, version: pkg.version, license: pkg.license, checksum: checksums.get(key), features: [...nodes.get(pkg.id).features].sort() };
        if (pkg.source === null) {
            if (!old.vendored_source || path.dirname(pkg.manifest_path) !== path.resolve(root, old.vendored_source.path)) throw new Error(`Unretained local dependency: ${key}`);
            row.vendored_source = old.vendored_source;
        } else if (pkg.source !== 'registry+https://github.com/rust-lang/crates.io-index' || !row.checksum || old.vendored_source) throw new Error(`Dependency registry identity changed: ${key}`);
        return row;
    });
    const canonical = value => Array.isArray(value) ? value.map(canonical) : value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
    const order = rows => rows.sort((a,b) => a.name.localeCompare(b.name, 'en') || a.version.localeCompare(b.version, 'en'));
    if (JSON.stringify(canonical(order(actual))) !== JSON.stringify(canonical(order([...retained])))) throw new Error('Resolved dependency graph changed; supply complete refreshed legal records before packaging');
    return actual.length;
}
