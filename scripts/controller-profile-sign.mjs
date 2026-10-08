#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import {createHash,createPrivateKey,createPublicKey,sign,verify} from 'node:crypto';
const [directory,keyPath,generationText,catalogVersion]=process.argv.slice(2);
if(!directory||!keyPath||!/^\d+$/.test(generationText??'')||!/^\d+\.\d+\.\d+$/.test(catalogVersion??''))throw Error('usage: controller-profile-sign DIRECTORY PRIVATE_KEY_FILE GENERATION CATALOG_VERSION');
const generation=Number(generationText);if(!Number.isSafeInteger(generation)||generation<1)throw Error('Invalid generation');
const metadata=fs.statSync(keyPath);if(!metadata.isFile()||metadata.mode&0o077)throw Error('Use an owner-only regular private signing key');
const key=createPrivateKey(fs.readFileSync(keyPath));
const publicKey=createPublicKey(key);const der=publicKey.export({type:'spki',format:'der'});
const pinned=fs.readFileSync('profiles/trust/ed25519-v1.pub','utf8').trim();if(der.subarray(-32).toString('hex')!==pinned)throw Error('Signing key does not match the pinned app trust root');
const names=fs.readdirSync(directory).filter(n=>n.endsWith('.json')&&n!=='catalog.json').sort();if(names.length<1||names.length>64)throw Error('Bounded profile files required');
const profiles=names.map(file=>{const p=path.join(directory,file);if(!fs.lstatSync(p).isFile())throw Error('Regular profile file required');const bytes=fs.readFileSync(p);const data=JSON.parse(bytes);if(bytes.length>65536||file!==`${data.id}-${data.version}.json`||data.schema!==1||data.preset_version!==6||data.provenance.current_physical_qualification!==false)throw Error('Incompatible profile asset');return{id:data.id,version:data.version,file,sha256:createHash('sha256').update(bytes).digest('hex'),bytes:bytes.length};});
const payload=JSON.stringify({schema:1,generation,version:catalogVersion,release:`controller-profiles-${catalogVersion}`,profiles});
const message=Buffer.concat([Buffer.from('Omatainer controller catalog schema 1\n'),Buffer.from(payload)]);const signature=sign(null,message,key);if(!verify(null,message,publicKey,signature))throw Error('Local signature verification failed');
const envelope={schema:1,signer:'ed25519-v1',payload,signature:signature.toString('hex')};
fs.writeFileSync(path.join(directory,'catalog.json'),JSON.stringify(envelope,null,2)+'\n',{flag:'wx'});
process.stdout.write(JSON.stringify({generation,version:catalogVersion,profiles:profiles.map(p=>({id:p.id,sha256:p.sha256,bytes:p.bytes})),signer:'ed25519-v1'})+'\n');
