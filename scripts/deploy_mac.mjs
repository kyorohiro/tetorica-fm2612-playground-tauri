import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {execFileSync} from 'node:child_process';
import {macAudioAssets} from './mac_audio_assets.mjs';
const root=fileURLToPath(new URL('../',import.meta.url));
const targets=process.argv.slice(2);
for(const target of targets.length?targets:['x86_64-apple-darwin','aarch64-apple-darwin']){
  const assets=await macAudioAssets(target);
  execFileSync(join(root,'node_modules/.bin/tauri'),['build','--target',target,'--bundles','app,dmg'],{
    cwd:root,stdio:'inherit',env:{...process.env,...assets},
  });
}
