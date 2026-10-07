#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {execFileSync} from 'node:child_process';

const root=process.cwd();
const read=p=>JSON.parse(fs.readFileSync(p,'utf8'));
const sha=b=>crypto.createHash('sha256').update(b).digest('hex');
const stable=v=>JSON.stringify(v,(_,x)=>x && typeof x==='object' && !Array.isArray(x)?Object.fromEntries(Object.keys(x).sort().map(k=>[k,x[k]])):x);
const write=(p,v)=>fs.writeFileSync(p,JSON.stringify(JSON.parse(stable(v)),null,2)+'\n');
const regular=p=>{if(!fs.lstatSync(p).isFile())throw Error(`Expected a regular provenance file: ${p}`);return fs.readFileSync(p);};
const walk=p=>fs.existsSync(p)?fs.readdirSync(p,{withFileTypes:true}).flatMap(e=>e.isDirectory()?walk(path.join(p,e.name)):[path.join(p,e.name)]):[];
const manifest=read('licenses/manifest.json');
const policy=read('licenses/assets.json');
const notes=read('licenses/notices.json');
const supplements=read('licenses/supplements.json');
const mode=process.argv[2];
if(!['refresh','check'].includes(mode))throw Error('usage: node scripts/license-inventory.mjs refresh|check');
const toolchain=Object.fromEntries(execFileSync('rustc',['-Vv'],{encoding:'utf8'}).trim().split('\n').filter(l=>l.includes(': ')).map(l=>[l.slice(0,l.indexOf(': ')),l.slice(l.indexOf(': ')+2)]));
if(stable(toolchain)!==stable(manifest.toolchain))throw Error('Rust toolchain changed; review its runtime notices before refreshing this inventory');
const meta=JSON.parse(execFileSync('cargo',['metadata','--locked','--offline','--format-version','1','--filter-platform',manifest.target],{encoding:'utf8',maxBuffer:32*1024*1024,env:{...process.env,TMPDIR:path.join(root,'target/validation/combined-completion/tmp')}}));
const checksums=new Map(fs.readFileSync('Cargo.lock','utf8').split('[[package]]').slice(1).map(block=>{const name=block.match(/^name = "([^"]+)"/m)?.[1],version=block.match(/^version = "([^"]+)"/m)?.[1],checksum=block.match(/^checksum = "([^"]+)"/m)?.[1]??null;if(!name||!version)throw Error('Unsupported Cargo lock record');return [`${name}@${version}`,checksum];}));
const nodes=new Map(meta.resolve.nodes.map(n=>[n.id,n]));
const previous=new Map(manifest.entries.map(e=>[e.id,e]));
const entries=manifest.entries.filter(e=>e.category!=='rust-component');
const cargo=[];
for(const p of meta.packages.filter(p=>p.name!=='omatainer'&&nodes.has(p.id)).sort((a,b)=>a.name.localeCompare(b.name,'en')||a.version.localeCompare(b.version,'en'))){
  const key=`${p.name}@${p.version}`;
  const row={name:p.name,version:p.version,license:p.license,checksum:checksums.get(key),features:[...nodes.get(p.id).features].sort()};
  if(row.checksum===undefined)throw Error(`Dependency missing from lock: ${key}`);
  const directory=path.dirname(p.manifest_path);
  const archive=`https://static.crates.io/crates/${p.name}/${p.name}-${p.version}.crate`;
  let sources=[{location:archive,sha256:row.checksum}];
  let delivery='resolved Linux build/runtime dependency; not a claim that every module is linked';
  if(p.source===null){
    const relative=path.relative(root,directory);
    if(!relative.startsWith('vendor/'))throw Error(`Unreviewed local dependency: ${directory}`);
    const upstream=read(path.join(directory,'UPSTREAM.json'));
    if(upstream.name!==p.name||upstream.version!==p.version||upstream.archive!==archive||!/^[a-f0-9]{64}$/.test(upstream.sha256))throw Error(`Invalid vendor identity: ${key}`);
    const files=Object.fromEntries(walk(directory).map(f=>[path.relative(root,f),sha(regular(f))]));
    const expected=new Set([...Object.keys(upstream.files),'LICENSE','UPSTREAM.json','PATCHES.md']);
    if(stable([...expected].sort())!==stable(walk(directory).map(f=>path.relative(directory,f)).sort()))throw Error(`Unreviewed vendor file: ${key}`);
    for(const [f,h]of Object.entries(upstream.files))if(!upstream.patches.includes(f)&&sha(regular(path.join(directory,f)))!==h)throw Error(`Unreviewed vendor modification: ${key}/${f}`);
    for(const f of Object.keys(files))policy.package[f]=`.local/share/omatainer/source/${f}`;
    row.vendored_source={path:relative,upstream_archive_sha256:upstream.sha256,files};
    sources=[{location:archive,sha256:upstream.sha256},...Object.entries(files).sort(([a],[b])=>a.localeCompare(b,'en')).map(([f,h])=>({location:policy.package[f],sha256:h}))];
    delivery+='; modified vendored component, exact covered source retained in package';
  }else if(!p.source.startsWith('registry+'))throw Error(`Unreviewed non-registry dependency: ${key}`);
  cargo.push(row);
  const existing=previous.get(`crate:${key}`);
  const refs=walk(directory).filter(f=>/LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT|UNLICENSE/.test(path.basename(f).toUpperCase())||(p.name==='epaint_default_fonts'&&f.endsWith('.txt'))).sort().map(f=>{
    const bytes=regular(f);if(bytes.length>1_000_000)throw Error(`Oversized license notice: ${f}`);const text=bytes.toString('utf8');if(!Buffer.from(text).equals(bytes))throw Error(`Non-UTF-8 notice: ${f}`);const hash=sha(bytes);notes[hash]=text;return {location:p.source===null?path.relative(root,f):`${archive}#${path.relative(directory,f)}`,sha256:hash};
  });
  if(!refs.length){for(const record of supplements[p.name]??[]){const hash=sha(Buffer.from(record.text));notes[hash]=record.text;refs.push({location:record.url,sha256:hash});}if(!refs.length){if(!existing)throw Error(`Missing pinned license notice: ${key}`);refs.push(...existing.notices);}}
  const allowed=['MIT','Apache-2.0','BSD-2-Clause','BSD-3-Clause','ISC','Zlib','MPL-2.0','Unicode-3.0','BSL-1.0','CC0-1.0','Unlicense','0BSD','CDLA-Permissive-2.0'];
  if(!existing&&!allowed.some(x=>p.license===x||p.license?.split(/ OR | AND /).every(y=>allowed.includes(y))))throw Error(`Review license rights: ${key} ${p.license}`);
  entries.push({id:`crate:${key}`,name:`${p.name} ${p.version}`,category:'rust-component',delivery,license:p.license,commercial_use:existing?.commercial_use??'The recorded open-source license permits commercial use subject to its terms.',redistribution:existing?.redistribution??'Retain the supplied copyright, license and required notices. OR denotes a choice of terms; AND requires all applicable terms. See the exact notices, including any third-party portions, below.',sources,notices:refs,members:[]});
}
cargo.sort((a,b)=>a.name<b.name?-1:a.name>b.name?1:a.version<b.version?-1:a.version>b.version?1:0);
const packageFiles=['contrib','plugin','vendor'].flatMap(walk).sort();
if(stable(packageFiles)!==stable(Object.keys(policy.package).sort()))throw Error('Unmanifested integration or vendor assets');
const source=new Set([...Object.keys(policy.package),'LICENSE','Cargo.toml','Cargo.lock','licenses/assets.json','licenses/supplements.json','benchmarks/policy.json','README.md','CONTRACT.md','docs/manual.md','docs/capability-matrix.md',...walk('docs/backlog'),...walk('scripts').filter(p=>/\.(py|sh|mjs)$/.test(p)),...['src','tests','benchmarks','vendor','locales'].flatMap(walk)]);
if(fs.existsSync('build.rs'))source.add('build.rs');
const used=new Set(entries.flatMap(e=>e.notices.map(n=>n.sha256)));
const retained=Object.fromEntries([...used].sort().map(h=>{if(sha(Buffer.from(notes[h]??''))!==h)throw Error(`Missing or altered notice: ${h}`);return[h,notes[h]];}));
if(mode==='refresh'){
  write('licenses/assets.json',policy);
  manifest.cargo=cargo;manifest.entries=entries;manifest.package=policy.package;
  manifest.source_files=Object.fromEntries([...source].sort().map(p=>[p,sha(regular(p))]));
  write('licenses/notices.json',retained);write('licenses/manifest.json',manifest);
  console.log(`Recorded ${cargo.length} dependencies and ${source.size} source inputs`);
}else{
  if(stable(manifest.entries)!==stable(entries))throw Error('Retained license entries changed');
  if(stable(manifest.cargo)!==stable(cargo))throw Error('Resolved dependency inventory changed');
  if(stable(manifest.package)!==stable(policy.package)||stable(Object.keys(manifest.source_files).sort())!==stable([...source].sort()))throw Error('Source/integration inventory changed');
  for(const [p,h]of Object.entries(manifest.source_files))if(sha(regular(p))!==h)throw Error(`Source changed: ${p}`);
  if(stable(notes)!==stable(retained))throw Error('Unindexed license notices');
  console.log('Pinned dependency, retained notice and source inventory checks passed');
}
