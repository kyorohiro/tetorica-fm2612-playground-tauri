#!/usr/bin/env node
// Actual Rust callback core + fixed SDK C++ against the Browser WASM synth and
// resampler. Offline only: never opens a device or creates an AudioContext.
import assert from 'node:assert/strict';
import {execFileSync,spawnSync} from 'node:child_process';
import {readFile,writeFile,mkdtemp,rm} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {tmpdir} from 'node:os';
import {resolve,dirname,relative} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
const repository=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const options={};
for(let i=2;i<process.argv.length;i++) {
  const arg=process.argv[i];
  if(arg==='--help'){console.log('node scripts/check_native_scsp.mjs [--source DIR] [--out FILE]');process.exit(0);}
  assert(['--source','--out'].includes(arg)&&process.argv[i+1],`Unknown/incomplete option ${arg}`);options[arg.slice(2)]=resolve(process.argv[++i]);
}
const source=options.source??resolve(repository,'../..'),native=resolve(repository,'src-tauri/src'),sdk=resolve(repository,'src-tauri/vendor/tetorica-soundchip-native');
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const coreNames=['scsp_instrument.cpp','scsp.cpp','scspdsp.cpp'];
const inputs=[...coreNames.map(n=>resolve(sdk,'vendor/mame-scsp',n)),...['scsp_instrument.h','scsp.h','scspdsp.h','standalone_support.h'].map(n=>resolve(sdk,'vendor/mame-scsp',n)),
  ...['native_genesis_scsp_core.rs','native_genesis_scsp_fixture.rs','native_genesis_scsp_alloc_probe.cpp'].map(n=>resolve(native,n)),
  resolve(repository,'src-tauri/build.rs'),resolve(repository,'src-tauri/soundchip-sdk.json'),
  ...['web/scsp.js','web/scspsynth.js','web/pcm_44100_resampler.js','docs/generated/scsp_wasm.js','docs/generated/scsp_wasm.wasm'].map(n=>resolve(source,n)),fileURLToPath(import.meta.url)];
const receipts=await Promise.all(inputs.map(async path=>({path:path.startsWith(repository)?`tauri/${relative(repository,path)}`:relative(source,path),sha256:hash(await readFile(path))})));
const temporary=await mkdtemp(resolve(tmpdir(),'tetorica-native-scsp-'));
try {
  const objects=[];
  for(const [i,path]of [...coreNames.map(n=>resolve(sdk,'vendor/mame-scsp',n)),resolve(native,'native_genesis_scsp_alloc_probe.cpp')].entries()) {
    const object=resolve(temporary,`${i}.o`);objects.push(object);
    execFileSync(process.env.CXX??'clang++',['-std=c++17','-O2','-fwrapv','-fno-strict-aliasing','-ffp-contract=off','-I'+resolve(sdk,'vendor/mame-scsp'),'-c',path,'-o',object],{stdio:'pipe'});
  }
  const library=resolve(temporary,'libscsp.a');execFileSync('ar',['rcs',library,...objects],{stdio:'pipe'});
  const linkage=['-L',temporary,'-l','static=scsp','-l',process.platform==='darwin'?'c++':'stdc++'];
  const fixture=resolve(temporary,'fixture'),test=resolve(temporary,'core-tests');
  execFileSync('rustc',['--edition=2021','-O',resolve(native,'native_genesis_scsp_fixture.rs'),...linkage,'-o',fixture],{stdio:'pipe'});
  execFileSync('rustc',['--edition=2021','--test',resolve(native,'native_genesis_scsp_core.rs'),...linkage,'-o',test],{stdio:'pipe'});
  const unitTests=execFileSync(test,[],{encoding:'utf8'}).trim();
  const {Scsp}=await import(pathToFileURL(resolve(source,'web/scsp.js')));
  const {ScspSynth,createScspInitSample}=await import(pathToFileURL(resolve(source,'web/scspsynth.js')));
  const {Pcm44100Resampler}=await import(pathToFileURL(resolve(source,'web/pcm_44100_resampler.js')));
  const {default:factory}=await import(pathToFileURL(resolve(source,'docs/generated/scsp_wasm.js')));
  const binary=await readFile(resolve(source,'docs/generated/scsp_wasm.wasm')),module=await factory({wasmBinary:binary});
  const scenarios=[
    {name:'cold-silence',seconds:.03,init(){},events:[],silent:true},
    {name:'PCM16-live-pitch-pan',events:[[113,'setFrequency',[0,880]],[997,'setMix',[0,{pan:15}]],[1701,'setMix',[0,{pan:31,totalLevel:8}]],[2303,'keyOff',[0]]]},
    {name:'PCM8-short-loop',init(s){const wave=s.loadPCM(new Int8Array([-128,0,127,0]),{rootNote:69,sourceSampleRate:22050,loopStartSamples:0});s.setPreset(31,{sample:wave,frequencyHz:880});s.keyOn(31);},events:[[501,'setFrequency',[31,440]],[1001,'setMix',[31,{pan:15}]],[1703,'keyOff',[31]]]},
    {name:'PCM16-one-shot',init(s){const wave=s.loadPCM(Int16Array.from({length:257},(_,i)=>i*197-26000),{rootNote:69});s.setPreset(7,{sample:wave,frequencyHz:440});s.keyOn(7);},events:[[701,'keyOn',[7]]],silentTail:4000},
    {name:'32-slot-clipping-and-isolated-retrigger',init(s,wave){for(let ch=0;ch<32;ch++){s.setPreset(ch,{sample:wave,frequencyHz:440+ch*3});s.keyOn(ch);}},events:[[503,'keyOff',[0]],[797,'keyOn',[31]],[997,'setMix',[13,{directLevel:0}]],[1501,'allNotesOff',[]]]},
    {name:'named-EG-release',init(s,wave){s.setPreset(0,{sample:wave,envelope:{attackRate:17,decay1Rate:14,decayLevel:10,decay2Rate:8,releaseRate:18,keyRateScaling:15}});s.keyOn(0);},seconds:.5,events:[[4001,'keyOff',[0]],[6001,'keyOn',[0]],[11001,'keyOff',[0]]]},
    {name:'Stop-retains-RAM-and-retrigger',events:[[509,'stopVoices',[]],[1101,'keyOn',[0]],[1701,'stopVoices',[]],[2301,'keyOn',[0]],[3001,'setFrequency',[0,220]],[3901,'stopVoices',[]]],silentTail:3901},
    {name:'reset-then-PCM8-upload',events:[[509,'reset',[{clearRam:true}]],[1101,'custom',s=>{const wave=s.loadPCM(new Int8Array([-100,50,100,-50]),{rootNote:69,loopStartSamples:0});s.setPreset(31,{sample:wave});s.keyOn(31);}],[3001,'keyOff',[31]]]},
    {name:'RAM-mutation-boundaries-and-masked-writes',events:[[31,'raw',(s,c)=>{c.loadRam(new Uint8Array([127,255]),0);c.loadRam(new Uint8Array([128,0]),524286);c.write(12,19,255);}],[701,'raw',(s,c)=>c.write(22,0x0f00,0x1f00)],[1101,'raw',(s,c)=>c.write(16,0x0800,0x7bff)]]},
    {name:'long-running-pitch-and-stop',seconds:10,events:[[48001,'setFrequency',[0,220]],[96001,'setFrequency',[0,880]],[144001,'setMix',[0,{totalLevel:16}]],[192001,'stopVoices',[]],[240001,'keyOn',[0]]]},
  ];
  function encode(rate,frames,events,blocks) {
    const chunks=[],words=(...values)=>{const data=Buffer.alloc(values.length*4);values.forEach((v,i)=>data.writeUInt32LE(v,i*4));chunks.push(data);};
    words(rate,frames,events.length,blocks.length);words(...blocks);
    for(const e of events){words(e.frame,e.kind,e.address??0,e.value??0,e.mask??65535,e.bytes?.length??0);if(e.bytes)chunks.push(e.bytes);}
    return Buffer.concat(chunks);
  }
  async function browserCase(scenario,rate,blocks) {
    let frame=0;const events=[],raw=await Scsp.create({moduleFactory:async()=>module});raw.prepare(16384);
    const core={sampleRate:44100,get nativeFrames(){return raw.nativeFrames;},prepare:n=>raw.prepare(n),
      reset(options={}){events.push({frame,kind:options.clearRam?4:3});raw.reset(options);},
      write(address,value,mask=65535){events.push({frame,kind:1,address,value,mask});raw.write(address,value,mask);},
      loadRam(bytes,address=0){events.push({frame,kind:2,address,bytes:Buffer.from(bytes)});raw.loadRam(bytes,address);},
      read:address=>raw.read(address),readRam:(...args)=>raw.readRam(...args),renderInto:(...args)=>raw.renderInto(...args),dispose:()=>raw.dispose()};
    const synth=new ScspSynth(core),resampler=new Pcm44100Resampler(core,rate),frames=Math.max(8192,Math.round(rate*(scenario.seconds??.12)));
    if(!scenario.silent){const {pcm,options}=createScspInitSample(),wave=synth.loadPCM(pcm,options);if(scenario.init)scenario.init(synth,wave);else{synth.setPreset(0,{sample:wave,frequencyHz:440});synth.keyOn(0);}}
    const actions=(scenario.events??[]).filter(e=>e[0]<frames),left=new Float32Array(frames),right=new Float32Array(frames);
    let action=0,block=0;
    while(frame<frames) {
      while(action<actions.length&&actions[action][0]===frame){const [,method,args]=actions[action++];if(method==='raw'||method==='custom')args(synth,core);else synth[method](...args);if(['reset','stopVoices'].includes(method))resampler.reset();}
      const end=Math.min(frames,frame+blocks[block++%blocks.length],actions[action]?.[0]??frames);
      resampler.render(left,right,end-frame,frame);frame=end;
    }
    const registers=Buffer.alloc(515*2);let at=0;for(const offset of [...Array.from({length:514},(_,i)=>i*2),0x408])registers.writeUInt16LE(raw.read(offset),at++*2);
    const ram=Buffer.from(raw.readRam(0,524288)),nativeFrames=raw.nativeFrames,outputFrames=resampler.outputFrames;
    synth.dispose();return {events,left,right,frames,registers,ram,nativeFrames,outputFrames};
  }
  const report={schemaVersion:1,status:'passed',deviceOpened:false,audioContextCreated:false,physicalAudioTested:false,
    method:'Actual Native Rust Core (same module compiled into Tauri) + pinned C ABI against real Browser ScspSynth/WASM/Pcm44100Resampler. Identical timestamped low-level operations, all stereo samples/registers/RAM and clock counts compared exactly.',
    sourcePin:JSON.parse(await readFile(resolve(repository,'src-tauri/soundchip-sdk.json'),'utf8')),inputs:receipts,unitTests,cases:[],stereoFrames:0,
    limits:['Native mode admission, RPC scheduling, leases, primary/extra rack and device recovery are not connected yet.','Offline equivalence does not verify hardware fidelity, physical output or callback deadlines.','Allocator probe counts Rust global alloc/dealloc and C++ new/delete; direct C malloc/free are not intercepted.']};
  for(const scenario of scenarios)for(const rate of [8000,44100,48000,96000,384000]) {
    // Long run is tested at realistic device rates, avoiding enormous fixtures.
    if(scenario.seconds===10&&![44100,48000,96000].includes(rate))continue;
    let original;
    for(const blocks of [[128],[1,7,63,257,3,1024,17]]) {
      const reference=await browserCase(scenario,rate,blocks),{frames,left,right,events}=reference;
      const run=spawnSync(fixture,[],{input:encode(rate,frames,events,blocks),maxBuffer:128*1024*1024});
      assert.equal(run.status,0,run.stderr?.toString());const bytes=run.stdout;
      assert.equal(bytes.length,frames*8+48+1030+524288);
      let peak=0;for(let i=0;i<frames;i++)for(let ch=0;ch<2;ch++){const value=bytes.readFloatLE(i*8+ch*4),expected=(ch?right:left)[i];assert.equal(value,expected,`${scenario.name} rate${rate} frame${i} ch${ch}`);peak=Math.max(peak,Math.abs(value));}
      const counters=Array.from({length:6},(_,i)=>Number(bytes.readBigUInt64LE(frames*8+i*8)));
      assert.deepEqual(counters,[reference.nativeFrames,reference.outputFrames,0,0,0,0],`${scenario.name} clocks/allocators`);
      assert.deepEqual(bytes.subarray(frames*8+48,frames*8+48+1030),reference.registers);
      assert.deepEqual(bytes.subarray(frames*8+48+1030),reference.ram);
      if(scenario.silent)assert.equal(peak,0);else assert(peak>0);
      if(scenario.silentTail!==undefined)for(let i=scenario.silentTail;i<frames;i++){assert.equal(left[i],0,`${scenario.name} left tail ${i}`);assert.equal(right[i],0,`${scenario.name} right tail ${i}`);}
      const pcmHash=hash(bytes.subarray(0,frames*8));if(original)assert.equal(pcmHash,original,'Partition changes PCM');else original=pcmHash;
      report.stereoFrames+=frames;report.cases.push({name:scenario.name,rate,blocks,frames,events:events.length,peak,pcmSha256:pcmHash,nativeFrames:counters[0],outputFrames:counters[1],rustAllocations:0,rustDeallocations:0,cppNew:0,cppDelete:0});
    }
    console.log(`${scenario.name} ${rate} Hz: exact PCM/registers/RAM, both partitions, no hot allocations`);
  }
  const current=await Promise.all(inputs.map(async path=>hash(await readFile(path))));
  assert.deepEqual(current,receipts.map(r=>r.sha256),'Inputs changed while measuring');
  if(options.out)await writeFile(options.out,JSON.stringify(report,null,2)+'\n');
  console.log(JSON.stringify({status:report.status,cases:report.cases.length,stereoFrames:report.stereoFrames,maximumDifference:0,unitTests}));
} finally {await rm(temporary,{recursive:true,force:true});}
