import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {execFileSync} from 'node:child_process';
const root=fileURLToPath(new URL('../audio-sidecar-bundle/',import.meta.url));
// Loading the addon checks architecture, ABI and shared-library dependencies.
// No device is opened and no audio hardware is required on the CI runner.
execFileSync(join(root,process.platform==='win32'?'node.exe':'node'),
  ['--eval',"const {RtAudio,RtAudioApi}=require('audify');if(typeof RtAudio!=='function')throw Error('Audify is unavailable');if(process.platform==='win32'){const audio=new RtAudio(RtAudioApi.WINDOWS_WASAPI);if(!String(audio.getApi()).toLowerCase().includes('wasapi'))throw Error('WASAPI is unavailable');audio.getDevices();console.log('Bundled Windows WASAPI enumeration works');}console.log('Bundled Node/Audify loads successfully');"],
  {cwd:root,stdio:'inherit',timeout:30000});
