import {cp,mkdir,copyFile,writeFile,chmod,rm,rename,stat} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';
import {signAudioSidecar} from './sign_audio_sidecar.mjs';
import {findNodeLicense} from './audio_node_license.mjs';
const root=fileURLToPath(new URL('../',import.meta.url));
execFileSync(process.platform==='win32'?'python':'python3',['scripts/import_release.py','--check'],{cwd:root,stdio:'inherit'});
const bundle=join(root,'audio-sidecar-bundle');await mkdir(bundle,{recursive:true});
const nodePath=join(bundle,process.platform==='win32'?'node.exe':'node');
const candidate=join(bundle,process.platform==='win32'?'node-next.exe':'node-next');
// Use the executable on disk, including in environments that virtualize Node's fs reads.
const nodeSource=process.env.TETORICA_AUDIO_NODE??process.execPath;
if(process.platform==='win32')await copyFile(nodeSource,candidate);
else execFileSync('cp',[nodeSource,candidate]);
await chmod(candidate,0o755);
let replaced=false;
try{
  // A standalone official Node is much larger than a shared-lib launcher. Do not
  // execute a copied launcher: its loader can depend on the original installation.
  if((await stat(candidate)).size<1024*1024)throw Error('Node is a shared-library launcher');
  if(process.platform==='darwin'&&execFileSync('otool',['-L',candidate],{encoding:'utf8'}).includes('@rpath/libnode'))throw Error('Node requires a separate libnode');
  await rename(candidate,nodePath);replaced=true;
}
catch{
  await rm(candidate,{force:true});
  try{if((await stat(nodePath)).size<1024*1024)throw Error('Missing runtime');console.log('Retained the working bundled Node runtime');}
  catch{throw Error('The copied Node cannot run. Set TETORICA_AUDIO_NODE to a standalone Node executable.');}
}
await cp(join(root,'audio-sidecar'),bundle,{recursive:true});
await rm(join(bundle,'node_modules'),{recursive:true,force:true});
for(const name of ['audify','bindings','file-uri-to-path','ws'])await cp(join(root,'node_modules',name),join(bundle,'node_modules',name),{recursive:true});
if(process.env.TETORICA_AUDIFY_RELEASE){
  const release=join(bundle,'node_modules/audify/build/Release');await rm(release,{recursive:true,force:true});
  await cp(process.env.TETORICA_AUDIFY_RELEASE,release,{recursive:true});
}
if(replaced)await cp(await findNodeLicense(nodeSource),join(bundle,'NODE_LICENSE'));
await cp(join(root,'dist/LICENSE'),join(bundle,'TETORICA_LICENSE'));
await writeFile(join(bundle,'package.json'),JSON.stringify({private:true,type:'module'}));
if(process.platform==='darwin')await signAudioSidecar(bundle);
console.log(`Prepared Audify sidecar for ${process.platform}/${process.env.TETORICA_AUDIO_ARCH??process.arch}`);
