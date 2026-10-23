#!/usr/bin/env node
// Actual Rust Shared/Renderer mixed racks. No CPAL stream or ignored tests.
import {spawnSync} from 'node:child_process';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
const repository=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const args=process.argv.slice(2),flags=new Set(['--long','--release','--help']),values=new Set(['--out','--seconds','--rounds','--compare']);
const options=new Map();let argumentError;
for(let i=0;i<args.length;i++){
  const [name,...inline]=args[i].split('=');
  if(flags.has(name)){if(inline.length)argumentError=`${name} takes no value`;options.set(name,true);continue;}
  if(!values.has(name)){argumentError=`Unknown option: ${args[i]}`;continue;}
  const value=inline.length?inline.join('='):args[++i];
  if(value===undefined||value.startsWith('--'))argumentError=`${name} requires a value`;else options.set(name,value);
}
if(options.has('--help')){
  console.log('Usage: node scripts/check_native_extra_stability.mjs [--out=report.json] [--long] [--seconds=30] [--rounds=3] [--release] [--compare=previous.json]');
  console.log('Default: three mixed racks, two rounds, two 0.125-second runs per round; fixed/variable exact PCM, owner/Stop/disposal and Native budget checks.');
  console.log('--long defaults to 30 simulated seconds per run and three rounds. Solo/raw diagnostic windows stay at 4096 frames. Timings have no speed threshold.');
  process.exit(argumentError?1:0);
}
const out=resolve(options.get('--out')??resolve(repository,'native-validation/extra-stability.json'));
await mkdir(dirname(out),{recursive:true});
let report={schemaVersion:1,passed:false,deviceOpened:false,scenes:[],soloSources:[]};const start=Date.now();
try{
  if(argumentError)throw Error(argumentError);
  const long=options.has('--long'),seconds=Number(options.get('--seconds')??(long?30:0.125)),rounds=Number(options.get('--rounds')??(long?3:2));
  if(!Number.isFinite(seconds)||seconds<0.09||seconds>3600)throw Error('--seconds must be between 0.09 and 3600');
  if(!Number.isInteger(rounds)||rounds<1||rounds>1000)throw Error('--rounds must be an integer between 1 and 1000');
  const cargo=['test','--locked','--offline','--manifest-path','src-tauri/Cargo.toml',...(options.has('--release')?['--release']:[]),'native_extra_stability_','--','--test-threads=1','--nocapture'];
  await writeFile(out,JSON.stringify({...report,runner:{state:'running',command:['cargo',...cargo]}},null,2)+'\n');
  const result=spawnSync('cargo',cargo,{cwd:repository,env:{...process.env,TETORICA_NATIVE_EXTRA_STABILITY_PROFILE:long?'long':'bounded',TETORICA_NATIVE_EXTRA_STABILITY_SECONDS:String(seconds),TETORICA_NATIVE_EXTRA_STABILITY_ROUNDS:String(rounds),TETORICA_NATIVE_EXTRA_STABILITY_REPORT:out},stdio:'inherit'});
  report=JSON.parse(await readFile(out,'utf8'));
  report.runner={state:'complete',platform:process.platform,arch:process.arch,command:['cargo',...cargo],exitCode:result.status,signal:result.signal,wallSeconds:(Date.now()-start)/1000,...(result.error?{error:result.error.message}:{})};
  report.passed=result.status===0&&report.passed===true&&report.scenes?.length===3&&report.scenes.every(scene=>scene.passed===true)&&report.soloSources?.length===7&&report.soloSources.every(source=>source.passed===true)&&report.admission?.passed===true&&report.slots?.passed===true&&report.deviceOpened===false;
  if(options.has('--compare')){
    const path=resolve(options.get('--compare')),baseline=JSON.parse(await readFile(path,'utf8'));
    report.comparison={baseline:path,comparableConfiguration:baseline.profile===report.profile&&baseline.sampleRate===report.sampleRate&&baseline.secondsPerRun===report.secondsPerRun&&baseline.rounds===report.rounds&&baseline.timingBuild===report.timingBuild,note:'Diagnostic only: build, compiler, host load and architecture affect timing.',scenes:report.scenes.map(scene=>{const previous=baseline.scenes?.find(old=>old.scene===scene.scene&&old.passed);return {scene:scene.scene,renderSeconds:scene.delivered?.renderSeconds,baselineRenderSeconds:previous?.delivered?.renderSeconds??null,renderTimeRatio:previous?.delivered?.renderSeconds>0?scene.delivered.renderSeconds/previous.delivered.renderSeconds:null};})};
  }
}catch(error){report.passed=false;report.runner={...report.runner,state:'failed',error:String(error.stack??error),wallSeconds:(Date.now()-start)/1000};}
await writeFile(out,JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({passed:report.passed,scenes:report.scenes?.length??0,soloSources:report.soloSources?.length??0,deviceOpened:false,report:out}));
if(!report.passed)process.exitCode=1;
