import {fileURLToPath} from 'node:url';
import {join,toNamespacedPath} from 'node:path';
import {execFileSync} from 'node:child_process';
import {cp,mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {nodeResourcePath} from './node_resource_path.mjs';
const root=fileURLToPath(new URL('../audio-sidecar-bundle/',import.meta.url));
// Loading the addon checks architecture, ABI and shared-library dependencies.
// No device is opened and no audio hardware is required on the CI runner.
execFileSync(join(root,process.platform==='win32'?'node.exe':'node'),
  ['--expose-gc','--eval',`(async()=>{
    const {RtAudio,RtAudioApi}=require('audify');
    if(typeof RtAudio!=='function')throw Error('Audify is unavailable');
    if(process.platform==='win32'){
      for(let cycle=0;cycle<3;cycle++){
        let audio=new RtAudio(RtAudioApi.WINDOWS_WASAPI);
        if(!String(audio.getApi()).toLowerCase().includes('wasapi'))throw Error('WASAPI is unavailable');
        audio.getDevices();audio=null;
        global.gc();await new Promise(resolve=>setImmediate(resolve));
      }
      console.log('Bundled Windows WASAPI enumeration and garbage collection work');
    }
    console.log('Bundled Node/Audify loads successfully');
  })().catch(error=>{console.error(error);process.exitCode=1;});`],
  {cwd:root,stdio:'inherit',timeout:30000});
console.log('Bundled Node/Audify exited normally');
// Use an isolated resource directory, including a space as in Windows installs.
// Testing require('audify') alone misses ESM imports and missing server resources.
const isolated=await mkdtemp(join(tmpdir(),'Tetorica audio resources '));
try{
  await cp(root,isolated,{recursive:true});
  const requests=[{id:1,op:'stop'},{id:2,op:'status'}];
  if(process.platform==='win32')requests.push({id:3,op:'devices'});
  requests.push({id:4,op:'stop'});
  // Start with the same verbatim resource path Rust canonicalize returns,
  // then apply the native launcher's conversion to executable AND cwd.
  const resourceRoot=nodeResourcePath(process.platform==='win32'?toNamespacedPath(isolated):isolated);
  const stdout=execFileSync(join(resourceRoot,process.platform==='win32'?'node.exe':'node'),['server.mjs'],{
    cwd:resourceRoot,encoding:'utf8',timeout:30000,
    input:requests.map(request=>JSON.stringify(request)).join('\n')+'\n',
    env:{...process.env,NODE_OPTIONS:'',NODE_PATH:''},
  });
  const replies=stdout.split(/\r?\n/).flatMap(line=>{try{return [JSON.parse(line)];}catch{return [];}});
  for(const request of requests){
    const reply=replies.find(reply=>reply.id===request.id);
    if(!reply||reply.error)throw Error(`Bundled server ${request.op} failed: ${reply?.error??stdout}`);
    if(request.op==='status'&&reply.result?.active!==false)throw Error('Bundled server should start idle');
    if(request.op==='devices'&&!Array.isArray(reply.result))throw Error('Invalid device reply');
  }
  console.log('Isolated bundled audio server starts, responds and stops successfully');
}finally{await rm(isolated,{recursive:true,force:true});}
