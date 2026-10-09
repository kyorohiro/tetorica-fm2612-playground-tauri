import {access} from 'node:fs/promises';
import {dirname,join} from 'node:path';

export function nodeLicenseCandidates(executable){
  const directory=dirname(executable);
  // Windows distributions put node.exe and LICENSE beside each other;
  // Unix distributions put node inside bin/ and LICENSE one directory above it.
  return [join(directory,'LICENSE'),join(directory,'../LICENSE'),join(directory,'../share/doc/node/LICENSE')];
}
export async function findNodeLicense(executable,override=process.env.TETORICA_AUDIO_NODE_LICENSE){
  const candidates=override?[override]:nodeLicenseCandidates(executable);
  for(const file of candidates){try{await access(file);return file;}catch{}}
  throw Error('Node LICENSE was not found. Set TETORICA_AUDIO_NODE_LICENSE to the distribution license.');
}
