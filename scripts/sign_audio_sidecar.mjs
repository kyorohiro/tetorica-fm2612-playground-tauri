import {readdir,open} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';

export async function nativeBinaries(root){
  const files=[];
  async function walk(directory){for(const entry of await readdir(directory,{withFileTypes:true})){
    const path=join(directory,entry.name);
    if(entry.isDirectory())await walk(path);
    else if(entry.isFile()){
      const handle=await open(path,'r');const magic=Buffer.alloc(4);
      try{await handle.read(magic,0,4,0);}finally{await handle.close();}
      if(['feedface','cefaedfe','feedfacf','cffaedfe','cafebabe','bebafeca','cafebabf','bfbafeca'].includes(magic.toString('hex')))files.push(path);
    }
  }}await walk(root);return files.sort();
}
export async function signAudioSidecar(root,{identity=process.env.APPLE_SIGNING_IDENTITY,arch=process.env.TETORICA_AUDIO_ARCH??process.arch,run=execFileSync}={}){
  const binaries=await nativeBinaries(root);
  const expected=arch==='x64'?'x86_64':arch;
  for(const file of binaries){
    const architectures=run('lipo',['-archs',file],{encoding:'utf8'}).trim().split(/\s+/);
    if(!architectures.includes(expected))throw Error(`Audio binary has ${architectures.join(', ')}, expected ${expected}: ${file}`);
  }
  const developerId=Boolean(identity&&identity!=='-');
  const entitlements=fileURLToPath(new URL('../src-tauri/AudioNode.entitlements.plist',import.meta.url));
  // Sign each nested Mach-O before Tauri signs/seals the containing .app.
  for(const file of binaries.sort((a,b)=>Number(a===join(root,'node'))-Number(b===join(root,'node')))){
    // Unsigned CI/dev builds must not retain a Developer ID Node signature while
    // replacing its addons with ad-hoc binaries (macOS rejects mismatched Teams).
    const args=['--force','--sign',developerId?identity:'-'];
    if(developerId)args.push('--timestamp','--options','runtime');
    if(file===join(root,'node'))args.push('--entitlements',entitlements);
    args.push(file);run('codesign',args,{stdio:'inherit'});
    run('codesign',['--verify','--strict',file],{stdio:'inherit'});
  }
  return binaries;
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
  await signAudioSidecar(resolve(process.argv[2]??'audio-sidecar-bundle'));
}
