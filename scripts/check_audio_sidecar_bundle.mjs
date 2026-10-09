import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {execFileSync} from 'node:child_process';
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
