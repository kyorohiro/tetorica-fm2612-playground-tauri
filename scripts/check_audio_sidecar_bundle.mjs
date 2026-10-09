import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {execFileSync} from 'node:child_process';
const root=fileURLToPath(new URL('../audio-sidecar-bundle/',import.meta.url));
// Loading the addon checks architecture, ABI and shared-library dependencies.
// No device is opened and no audio hardware is required on the CI runner.
execFileSync(join(root,process.platform==='win32'?'node.exe':'node'),
  ['--eval',"const {RtAudio}=require('audify');if(typeof RtAudio!=='function')throw Error('Audify is unavailable');console.log('Bundled Node/Audify loads successfully');"],
  {cwd:root,stdio:'inherit',timeout:30000});
