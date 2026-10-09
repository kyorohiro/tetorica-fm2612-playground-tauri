import {mkdir,readFile,writeFile,access} from 'node:fs/promises';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
const root=fileURLToPath(new URL('../',import.meta.url));
const hashes={
  arm64:{node:'be87e21bd235a451fad02c89e5bf7cb17e206e4cd89dd5664f20d19e7dfde6f9',audify:'a0943546829becd4638cd7ae6125308b37bda806742b7d4bd723ae8bd7c6648c'},
  x64:{node:'c266da5a9075a56e1aa02460ce8df96fca9e796c388abe94a8df4949945df6b6',audify:'ce341df9e044f9a99c122b1fd6416e1261f627dfb263bdd1afa91f98b21655f4'},
};
async function archive(url,file,hash){
  try{await access(file);}catch{const response=await fetch(url);if(!response.ok)throw Error(`Download failed (${response.status}): ${url}`);await writeFile(file,new Uint8Array(await response.arrayBuffer()));}
  if(createHash('sha256').update(await readFile(file)).digest('hex')!==hash)throw Error(`SHA256 mismatch: ${file}`);
}
export async function macAudioAssets(target){
  const arch=target==='aarch64-apple-darwin'?'arm64':target==='x86_64-apple-darwin'?'x64':null;
  if(!arch)throw Error(`Unsupported macOS target: ${target}`);
  const cache=join(root,'audio-mac-cache');await mkdir(cache,{recursive:true});
  const nodeName=`node-v25.2.1-darwin-${arch}`,nodeArchive=join(cache,nodeName+'.tar.gz');
  const audifyName=`audify-v1.10.1-napi-v10-darwin-${arch}`,audifyArchive=join(cache,audifyName+'.tar.gz');
  await archive(`https://nodejs.org/dist/v25.2.1/${nodeName}.tar.gz`,nodeArchive,hashes[arch].node);
  await archive(`https://github.com/almoghamdani/audify/releases/download/v1.10.1/${audifyName}.tar.gz`,audifyArchive,hashes[arch].audify);
  const native=join(cache,audifyName);await mkdir(native,{recursive:true});
  execFileSync('tar',['-xzf',nodeArchive,'-C',cache]);execFileSync('tar',['-xzf',audifyArchive,'-C',native]);
  return {TETORICA_AUDIO_ARCH:arch,TETORICA_AUDIO_NODE:join(cache,nodeName,'bin/node'),
    TETORICA_AUDIO_NODE_LICENSE:join(cache,nodeName,'LICENSE'),TETORICA_AUDIFY_RELEASE:join(native,'build/Release')};
}
if(process.argv[1]===fileURLToPath(import.meta.url))console.log(JSON.stringify(await macAudioAssets(process.argv[2])));
