#!/usr/bin/env node
// Offline exact PCM comparison: real Browser worklet/WASM vs pinned Native DSP.
// The AudioWorkletProcessor host alone is stubbed; no audio device is opened.
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFile,writeFile,mkdtemp,rm,mkdir} from 'node:fs/promises';
import {tmpdir,endianness} from 'node:os';
import {dirname,resolve,relative} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';

const repository=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const options={};
for(let i=2;i<process.argv.length;i++){
  const arg=process.argv[i];
  if(arg==='--help'){console.log('node scripts/check_native_wonderswan.mjs [--source DIR] [--fixture FILE] [--out FILE] [--sanitize]');process.exit(0);}
  if(arg==='--sanitize'){options.sanitize=true;continue;}
  assert(['--source','--fixture','--out'].includes(arg),`Unknown argument: ${arg}`);
  assert(process.argv[i+1]&&!process.argv[i+1].startsWith('--'),`${arg} requires a path`);
  options[arg.slice(2)]=resolve(process.argv[++i]);
}
assert.equal(endianness(),'LE','Fixture float/state encoding is little endian');
const source=options.source??resolve(repository,'../..');
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const sdk=resolve(repository,'src-tauri/vendor/tetorica-soundchip-native');
const native=resolve(repository,'src-tauri/src');
let temporary;
try{
  const sources=[resolve(native,'native_genesis_wonderswan.cpp'),resolve(native,'native_genesis_wonderswan_fixture.cpp'),resolve(sdk,'vendor/mame-wonderswan/wonderswan.cpp')];
  const inputs=[...sources,resolve(native,'native_genesis_wonderswan.h'),resolve(sdk,'vendor/mame-wonderswan/wonderswan.h'),resolve(repository,'src-tauri/soundchip-sdk.json')];
  const nativeSources=await Promise.all(inputs.map(async path=>({path:relative(repository,path),sha256:hash(await readFile(path))})));
  let executable=options.fixture;
  if(!executable){
    temporary=await mkdtemp(resolve(tmpdir(),'tetorica-native-wonderswan-'));
    executable=resolve(temporary,'fixture');
    const flags=options.sanitize?['-O1','-g','-fsanitize=address,undefined','-fno-omit-frame-pointer']:['-O2'];
    execFileSync(process.env.CXX??'clang++',['-std=c++17',...flags,'-ffp-contract=off','-fwrapv','-fno-strict-aliasing','-I'+resolve(sdk,'vendor/mame-wonderswan'),...sources,'-o',executable],{stdio:'pipe'});
  }
  // Fixture emits self-test JSON to stderr, including the allocator scope.
  const selfRun=await import('node:child_process').then(({spawnSync})=>spawnSync(executable,['--self-test'],{encoding:'utf8'}));
  assert.equal(selfRun.status,0,selfRun.stderr);const selfTest=JSON.parse(selfRun.stderr);assert.equal(selfTest.passed,true);
  const {WonderSwan,WONDERSWAN_CLOCK}=await import(pathToFileURL(resolve(source,'web/wonderswan.js')));
  const {WonderSwanVoiceInspector}=await import(pathToFileURL(resolve(source,'web/playground_wonderswan_inspector.js')));
  const {ChipClockResampler,validateChipClock}=await import(pathToFileURL(resolve(source,'web/playground_chip_clock.js')));
  const {createProcessorPerformance}=await import(pathToFileURL(resolve(source,'web/playground_performance_processor.js')));
  const {receiveChipInspector}=await import(pathToFileURL(resolve(source,'web/playground_chip_inspector_state.js')));
  const {default:factory}=await import(pathToFileURL(resolve(source,'docs/generated/wonderswan_wasm.js')));
  const binary=await readFile(resolve(source,'docs/generated/wonderswan_wasm.wasm'));
  const workletBytes=await readFile(resolve(source,'web/playground_wonderswan_worklet.js'));
  const workletSource=workletBytes.toString().replace(/^import .*;$/gm,'');
  const report={schemaVersion:1,status:'failed',deviceOpened:false,audioContextCreated:false,physicalAudioTested:false,
    method:'Actual Browser WonderSwan Processor source, performance scheduler, inspector, clock resampler and real WASM; only host AudioWorkletProcessor/MessagePort stubbed. Native uses unchanged pinned SDK core plus application adapter.',
    wasmSha256:hash(binary),workletSha256:hash(workletBytes),nativeSources,selfTest,sanitizers:options.sanitize?['address','undefined']:[],
    sourcePin:JSON.parse(await readFile(resolve(repository,'src-tauri/soundchip-sdk.json'),'utf8')),
    limits:['Offline PCM/API checks do not measure physical audio, deadlines or hardware accuracy.','Browser worklet has no per-source mute command; Native mute is checked independently against phase/zero/register controls and raw WASM.','Allocation audit intercepts C++ new/delete only, not direct malloc/free.'],rates:[44100,48000,96000],cases:[],rawCases:[],stereoFrames:0,rawStereoFrames:0,maximumDifference:0};
  const ev=(frame,op,address=0,value=0,mask=255)=>({frame,op,address,value,mask});
  function voice(ch,byteOffset=0){
    const events=[];
    for(let i=0;i<16;i++)events.push(ev(0,'memory',ch*16+i,((i*2+byteOffset)&15)|(((i*2+1+byteOffset)&15)<<4)));
    const frequency=1737+ch*69;
    events.push(ev(0,'write',0x80+ch*2,frequency&255),ev(0,'write',0x81+ch*2,frequency>>8),ev(0,'write',0x88+ch,[0xf1,0x2e,0xb5,0x79][ch]));
    return events;
  }
  const base=[ev(0,'write',0x91,8),...Array.from({length:4},(_,ch)=>voice(ch,ch*3)).flat(),ev(0,'write',0x90,15)];
  const cases=[
    {name:'cold-silence',frames:1024,events:[],silent:true},
    {name:'four-waves-live-RAM-pan-base-output',events:[...base,ev(113,'memory',3,255),ev(997,'write',0x88,0x0f),ev(1701,'write',0x8f,255),ev(2301,'write',0x8f,0),ev(3103,'write',0x91,0),ev(3901,'write',0x91,8)]},
    {name:'masked-gates-and-mode-bits',events:[...base,ev(503,'write',0x90,0,2),ev(901,'write',0x90,64,64),ev(1503,'write',0x90,0,8),ev(2201,'write',0x90,2,2),ev(3001,'off',0),ev(4001,'stop')]},
    {name:'signed-sweep',events:[...base,ev(0,'write',0x90,0x44),ev(0,'write',0x8c,7),ev(0,'write',0x8d,0),ev(701,'write',0x8c,0xff),ev(1901,'write',0x8c,0x81),ev(3001,'write',0x8d,1)]},
    {name:'PCM-stereo-half-full-volume',events:[...base,ev(0,'write',0x90,0x22),ev(0,'write',0x94,15),ev(0,'write',0x89,0xe3),ev(509,'write',0x94,5),ev(997,'write',0x89,0x20),ev(1701,'write',0x94,8),ev(2601,'write',0x94,2),ev(3701,'write',0x90,0,32)]},
    {name:'clock-min-max-fractional-reset',events:[...base,ev(509,'clock',0,1536000),ev(997,'clock',0,3579545),ev(1701,'clock',0,100000),ev(2101,'clock',0,100000000),ev(2301,'clock',0,3072000),ev(2601,'reset'),...base.map(e=>({...e,frame:2701})),ev(3103,'clock',0,2000001),ev(3901,'off',3)]},
    {name:'stop-retains-patch-RAM-clock',events:[...base,ev(509,'clock',0,4000001),ev(997,'stop'),ev(1701,'write',0x90,15,15),ev(3103,'off',1),ev(3901,'stop')]},
    {name:'cold-constant-wave-reset-gate-fence',events:[...base,...Array.from({length:64},(_,i)=>ev(0,'memory',i,0x88)),ev(31,'off',0),ev(101,'write',0x90,1,1),ev(997,'reset'),...base.map(e=>({...e,frame:1101})),...Array.from({length:64},(_,i)=>ev(1101,'memory',i,0x88))],silent:true},
  ];
  for(let type=0;type<8;type++)cases.push({name:'noise-'+type,events:[...base,ev(0,'write',0x8e,0x18|type),ev(0,'write',0x90,0x88),ev(701,'write',0x8e,0x10|type),ev(1901,'write',0x90,8),ev(2601,'write',0x90,0x88),ev(3901,'off',3)]});
  let seed=0x658762;const random=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed;};
  const fuzz=[...base];for(let i=0;i<240;i++){const frame=i*19+1;if(i%4===0)fuzz.push(ev(frame,'memory',random()%16384,random()&255));else fuzz.push(ev(frame,'write',0x80+(random()%32),random()&255));}
  cases.push({name:'deterministic-raw-registers-RAM',events:fuzz});
  async function processorFor(rate,clock){
    let Processor,resolveReady,frame=0;
    const ready=new Promise(resolve=>resolveReady=resolve);
    const port={postMessage:data=>{if(data.ready||data.error)resolveReady(data);}};
    vm.runInNewContext(workletSource,{WonderSwan,WONDERSWAN_CLOCK,factory,WonderSwanVoiceInspector,ChipClockResampler,validateChipClock,receiveChipInspector,sampleRate:rate,
      createProcessorPerformance:options=>createProcessorPerformance({...options,rate,nowFrame:()=>frame}),
      AudioWorkletProcessor:class{constructor(){this.port=port;}},registerProcessor:(_name,Type)=>Processor=Type});
    const p=new Processor({processorOptions:{clock,wasmBinary:binary}});
    assert.equal((await ready).ready,true);
    return {p,advance:n=>frame+=n};
  }
  function nativeRender(test,rate,clock,mode){
    const input=test.events.map(e=>`${e.frame} ${e.op} ${e.address} ${e.value} ${e.mask}`).join('\n')+'\n';
    const bytes=execFileSync(executable,[String(rate),String(test.frames),String(clock),mode],{input,maxBuffer:test.frames*8+32768,timeout:30000});
    assert.equal(bytes.length,test.frames*8+16428);
    return {pcm:new Float32Array(bytes.buffer,bytes.byteOffset,test.frames*2),state:bytes.subarray(test.frames*8)};
  }
  function command(p,e){
    if(e.op==='write')p.receive({method:'writeRegister',args:[e.address,e.value],controlMask:e.mask});
    else if(e.op==='memory')p.receive({method:'writeMemory',args:[e.address,e.value]});
    else if(e.op==='clock')p.receive({method:'clock',args:[e.value]});
    else if(e.op==='reset')p.receive({method:'reset'});
    else if(e.op==='stop')p.receive({method:'stopVoices'});
    else if(e.op==='off')p.receive({method:'keyOff',args:[e.address]});
    else throw Error('Unknown operation '+e.op);
  }
  for(const rate of report.rates)for(const clock of [3072000,3579545])for(const testBase of cases){
    const test={frames:5120,...testBase,events:testBase.events.slice().sort((a,b)=>a.frame-b.frame)};
    const nativeResult=nativeRender(test,rate,clock,'speaker');
    for(const layout of [[128],[1,7,113,257]]){
      const {p,advance}=await processorFor(rate,clock);
      let frame=0,next=0,block=0,max=0,peak=0;
      try{
        while(frame<test.frames){
          while(next<test.events.length&&test.events[next].frame===frame)command(p,test.events[next++]);
          const length=Math.min(layout[block++%layout.length],test.frames-frame,(test.events[next]?.frame??test.frames)-frame);
          assert(length>0);const left=new Float32Array(length),right=new Float32Array(length);
          p.process([] ,[[left,right]]);advance(length);
          for(let i=0;i<length;i++){
            const l=nativeResult.pcm[(frame+i)*2],r=nativeResult.pcm[(frame+i)*2+1];
            assert(Number.isFinite(l)&&Number.isFinite(r));
            max=Math.max(max,Math.abs(l-left[i]),Math.abs(r-right[i]));peak=Math.max(peak,Math.abs(l),Math.abs(r));
          }
          frame+=length;
        }
        assert.equal(max,0,`${test.name}@${rate}/${clock} layout${layout}: exact actual-worklet speaker PCM`);
        assert.deepEqual(Array.from(nativeResult.state.subarray(0,32)),Array.from(p.inspector.registers),'applied register mirror');
        assert.deepEqual(Array.from(nativeResult.state.subarray(32,16416)),Array.from(p.inspector.memory),'applied RAM mirror');
        assert.equal(nativeResult.state.readUInt32LE(16416),p.clock,'applied clock');
        if(test.silent)assert.equal(peak,0,`${test.name}: constant/cold control must remain silent`);
        else assert(peak>0,`${test.name}: positive PCM control`);
        report.cases.push({name:test.name,sampleRate:rate,initialClock:clock,layout,frames:test.frames,maximumDifference:max,peak});
        report.stereoFrames+=test.frames;
      }finally{p.dispose();}
    }
  }
  // Separate raw C ABI vs real WASM: raw DSP stays uncentered and unscaled.
  for(const rate of report.rates)for(const clock of [100001,3072000,3579545]){
    const test={name:'raw-four-waves-mute-noise-sweep-PCM',frames:8192,events:[...base,
      ev(701,'mute',0,5),ev(997,'mute',0,0),ev(1701,'write',0x8e,0x1e),ev(1701,'write',0x90,0x88),
      ev(2601,'write',0x90,0x44),ev(2601,'write',0x8c,0xff),ev(2601,'write',0x8d,0),
      ev(3901,'write',0x90,0x22),ev(3901,'write',0x94,15),ev(3901,'write',0x89,0xe3),
      ev(4901,'write',0x89,0x20),ev(6001,'reset'),...base.map(e=>({...e,frame:6101}))].sort((a,b)=>a.frame-b.frame)};
    const {pcm}=nativeRender(test,rate,clock,'raw');
    const chip=await WonderSwan.create({moduleFactory:factory,moduleOptions:{wasmBinary:binary},clock,sampleRate:rate});
    let frame=0,next=0,max=0,mute=0;
    try{
      while(frame<test.frames){
        while(next<test.events.length&&test.events[next].frame===frame){const e=test.events[next++];
          if(e.op==='write')chip.writeRegister(e.address,e.value);
          else if(e.op==='memory')chip.writeMemory(e.address,e.value);
          else if(e.op==='mute'){mute=e.value;for(let ch=0;ch<4;ch++)chip.setChannelMuted(ch,!!(mute&(1<<ch)));}
          else if(e.op==='reset'){chip.reset();for(let ch=0;ch<4;ch++)chip.setChannelMuted(ch,!!(mute&(1<<ch)));}
        }
        const count=Math.min(257,test.frames-frame,(test.events[next]?.frame??test.frames)-frame),view=chip.generateStereoView(count);
        for(let i=0;i<count;i++)max=Math.max(max,Math.abs(pcm[(frame+i)*2]-view.left[i]),Math.abs(pcm[(frame+i)*2+1]-view.right[i]));
        frame+=count;
      }
      assert.equal(max,0,`raw WASM ${rate}/${clock}`);
      report.rawCases.push({name:test.name,sampleRate:rate,initialClock:clock,frames:test.frames,maximumDifference:0});report.rawStereoFrames+=test.frames;
    }finally{chip.dispose();}
  }
  report.status='passed';
  if(options.out){await mkdir(dirname(options.out),{recursive:true});await writeFile(options.out,JSON.stringify(report,null,2)+'\n');}
  console.log(JSON.stringify({status:report.status,cases:report.cases.length,stereoFrames:report.stereoFrames,rawCases:report.rawCases.length,rawStereoFrames:report.rawStereoFrames,maximumDifference:0,selfTest,deviceOpened:false}));
}finally{if(temporary)await rm(temporary,{recursive:true,force:true});}
