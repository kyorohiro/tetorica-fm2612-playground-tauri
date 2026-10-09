import {readFile,writeFile,stat} from 'node:fs/promises';
import {createRequire} from 'node:module';
import {createHash} from 'node:crypto';
import {join} from 'node:path';
import {execFileSync} from 'node:child_process';

// Audify 1.10.1's WASAPI enumerator is a ComPtr member: its implicit
// destructor runs AFTER the destructor body calls CoUninitialize().
// Release it while COM is still alive instead of calling Release after shutdown.
export function patchWasapi(source){
  const newline=source.includes('\r\n')?'\r\n':'\n';
  const normalized=source.replaceAll('\r\n','\n');
  const start=normalized.indexOf('RtApiWasapi::~RtApiWasapi()');
  const end=normalized.indexOf('//-----------------------------------------------------------------------------',start);
  if(start<0||end<0)throw Error('Unsupported Audify WASAPI destructor');
  const body=normalized.slice(start,end);
  const old='  // If this object previously called CoInitialize()\n';
  const replacement='  // Release COM interfaces before CoUninitialize (Tetorica Windows fix).\n  deviceEnumerator_.Reset();\n\n'+old;
  if(body.includes(replacement))return source;
  if(body.split(old).length!==2||!body.includes('    CoUninitialize();'))throw Error('Unsupported Audify WASAPI cleanup');
  return (normalized.slice(0,start)+body.replace(old,replacement)+normalized.slice(end)).replaceAll('\n',newline);
}

export async function prepareWindowsAudify(root){
  const audify=join(root,'node_modules/audify');
  const pkg=JSON.parse(await readFile(join(audify,'package.json'),'utf8'));
  if(pkg.version!=='1.10.1')throw Error('Review the Windows WASAPI patch before updating Audify');
  const sourcePath=join(audify,'vendor/rtaudio/RtAudio.cpp');
  const original=await readFile(sourcePath,'utf8');
  const patched=patchWasapi(original);
  const key=createHash('sha256').update(patched).update(process.version).update(process.arch).digest('hex');
  const marker=join(audify,'build/tetorica-wasapi-patch.txt');
  const binary=join(audify,'build/Release/audify.node');
  try{
    if(original===patched&&(await readFile(marker,'utf8'))===key&&(await stat(binary)).size>0)return;
  }catch{/* A fresh npm install needs the patched native build. */}
  await writeFile(sourcePath,patched);
  const require=createRequire(join(root,'package.json'));
  console.log('Rebuilding Audify with WASAPI COM cleanup fix');
  execFileSync(process.execPath,[require.resolve('cmake-js/bin/cmake-js'),'rebuild','--directory',audify],{cwd:root,stdio:'inherit'});
  await stat(binary);
  await writeFile(marker,key);
}
