import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
const root=process.cwd();

/** Prepare a private source inventory fixture. Takes a workflow; returns after validating it and removing only its private copy. */
function fixture(workflow){
    const directory=fs.mkdtempSync(path.join(os.tmpdir(),'omatainer-license-inventory-'));
    try {
        const manifest=JSON.parse(fs.readFileSync(path.join(root,'licenses/manifest.json')));
        const files=new Set([...Object.keys(manifest.source_files),'licenses/manifest.json','licenses/notices.json','licenses/assets.json','licenses/supplements.json']);
        for(const file of files){const target=path.join(directory,file);fs.mkdirSync(path.dirname(target),{recursive:true});fs.copyFileSync(path.join(root,file),target,fs.constants.COPYFILE_EXCL);}
        fs.mkdirSync(path.join(directory,'target/validation/combined-completion/tmp'),{recursive:true});
        const run=mode=>execFileSync(process.execPath,['scripts/license-inventory.mjs',mode],{cwd:directory,encoding:'utf8',stdio:'pipe',env:{...process.env,TMPDIR:path.join(directory,'target/validation/combined-completion/tmp')}});
        workflow(directory,run);
    } finally {fs.rmSync(directory,{recursive:true,force:true});}
}
test('license inventory retains checklist files and detects changed checklist bytes',()=>fixture((directory,run)=>{
    assert.match(run('check'),/Pinned dependency, retained notice and source inventory checks passed/);
    const file=path.join(directory,'docs/backlog/app-checklist.md');fs.appendFileSync(file,'\nPrivate fixture change.\n');
    assert.throws(()=>run('check'),error=>error.status===1&&/Source changed: docs\/backlog\/app-checklist.md/.test(error.stderr));
    run('refresh');assert.match(run('check'),/checks passed/);
    const manifest=JSON.parse(fs.readFileSync(path.join(directory,'licenses/manifest.json')));for(const file of ['app-checklist.json','app-checklist.md','remaining-checklist.md'])assert.ok(manifest.source_files['docs/backlog/'+file]);
}));
test('license inventory refuses an unrecorded backlog input and regenerates its exact source set',()=>fixture((directory,run)=>{
    const file=path.join(directory,'docs/backlog/private-new-input.json');fs.writeFileSync(file,'{"fixture":true}\n',{flag:'wx'});
    assert.throws(()=>run('check'),error=>error.status===1&&/Source\/integration inventory changed/.test(error.stderr));
    run('refresh');assert.match(run('check'),/checks passed/);
    fs.unlinkSync(file);assert.throws(()=>run('check'),error=>error.status===1&&/Source\/integration inventory changed/.test(error.stderr));
    run('refresh');assert.match(run('check'),/checks passed/);
}));
