import { test } from 'node:test';
import assert from 'node:assert/strict';
import { verifyGraph } from './retained-dependency-graph.mjs';

const root='/home/qualification/project', checksum='a'.repeat(64);
const lock=`version = 4\n[[package]]\nname = "omatainer"\nversion = "0.1.0"\n[[package]]\nname = "quick-xml"\nversion = "0.41.0"\nchecksum = "${checksum}"\n`;
const row={name:'quick-xml',version:'0.41.0',license:'MIT',checksum,features:['default']};
const metadata={resolve:{root:'root',nodes:[{id:'root',features:[],deps:[{pkg:'xml'}]},{id:'xml',features:['default']}]},packages:[{id:'root',name:'omatainer'},{id:'xml',name:'quick-xml',version:'0.41.0',license:'MIT',source:'registry+https://github.com/rust-lang/crates.io-index',manifest_path:'/home/cache/quick-xml/Cargo.toml'}]};

test('a root dependency edge may change only when the complete retained graph stays identical',()=>{
    assert.equal(verifyGraph(metadata,lock,[row],root),1);
    const indirect=structuredClone(metadata);indirect.resolve.nodes[0].deps=[];
    assert.equal(verifyGraph(indirect,lock,[row],root),1);
});
test('versions, features, licenses, registries, checksums and omitted components refuse retention',()=>{
    for(const mutate of [m=>m.packages[1].version='0.42.0',m=>m.packages[1].license='GPL-3.0',m=>m.packages[1].source='registry+https://example.com/index',m=>m.resolve.nodes[1].features.push('encoding'),m=>m.packages.pop()]) {
        const changed=structuredClone(metadata);mutate(changed);assert.throws(()=>verifyGraph(changed,lock,[row],root));
    }
    assert.throws(()=>verifyGraph(metadata,lock.replace(checksum,'b'.repeat(64)),[row],root));
});
test('local components require the exact retained vendor location and source record',()=>{
    const local=structuredClone(metadata);local.packages[1].source=null;local.packages[1].manifest_path=root+'/vendor/quick-xml/Cargo.toml';
    const retained={...row,checksum:null,vendored_source:{path:'vendor/quick-xml',files:{},upstream_archive_sha256:checksum}};
    assert.throws(()=>verifyGraph(local,lock,[row],root));
    assert.equal(verifyGraph(local,lock.replace(`checksum = "${checksum}"`,''),[retained],root),1);
    local.packages[1].manifest_path=root+'/other/quick-xml/Cargo.toml';assert.throws(()=>verifyGraph(local,lock,[retained],root));
});
