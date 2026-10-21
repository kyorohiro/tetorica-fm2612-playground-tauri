/** Actual native/shared-WASM noise DSP comparison; no audio device or playback. */
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {execFileSync} from 'node:child_process';

const project=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const upstream=path.resolve(project,process.argv[2]??'../..');
const {NativeFXEngine}=await import(pathToFileURL(path.join(upstream,'web/native_fx_engine.js')));
const wasm=await WebAssembly.compile(await fs.readFile(path.join(upstream,'web/native_audio_effect.wasm')));
const temporary=await fs.mkdtemp(path.join(os.tmpdir(),'tetorica-native-noise-'));
const create=(slot,type,seed=123456)=>({operation:0,slot,argument:type,seed});
const parameter=(slot,argument,value,frames=0)=>({operation:1,slot,argument,value,frames});
const filter=(slot,argument)=>({operation:2,slot,argument});
const action=(slot,argument)=>({operation:3,slot,argument});
const reset={operation:4};
const at=(frame,...events)=>events.map(event=>({...event,frame}));
const types=['white','pink','brown','gray','clip'];
const scenarios=[
  ...types.map((name,type)=>({name,events:at(0,create(0,type),action(0,0))})),
  ...['lowpass','highpass','bandpass','notch','allpass','peaking','lowshelf','highshelf'].map((name,mode)=>({name:`filter-${name}`,events:at(0,create(0,0),parameter(0,2,800),parameter(0,3,.707),filter(0,mode),action(0,0))})),
  {name:'ramp-release-restart',events:[...at(0,create(0,1),parameter(0,0,.05),parameter(0,1,-1),parameter(0,4,.005),parameter(0,5,.01),action(0,0)),...at(128,parameter(0,0,.6,1000),parameter(0,1,1,1300),parameter(0,2,4800,1800),parameter(0,3,1.2,600)),...at(3072,action(0,1)),...at(5120,action(0,0)),...at(6144,action(0,1))],silentAfter:7200},
  {name:'multivoice-dispose-reset',events:[...at(0,...[0,1,2].flatMap(slot=>[create(slot,slot*2),parameter(slot,0,.1),parameter(slot,1,slot-1),action(slot,0)])),...at(2048,action(0,2)),...at(4096,create(0,3,0),action(0,0)),...at(8192,reset)],silentAfter:8192},
  {name:'seed-zero-slots',events:at(0,create(0,0,0),parameter(0,1,-1),action(0,0),create(31,0,0),parameter(31,1,1),action(31,0))},
  {name:'all-32-voices',events:at(0,...Array.from({length:32},(_,slot)=>[create(slot,slot%5,slot),parameter(slot,0,.01),action(slot,0)]).flat())},
  {name:'existing-stereo-input',input:true,events:at(0,create(0,0),parameter(0,0,.1),parameter(0,1,-.4),filter(0,1),action(0,0))},
];
try {
  const binary=path.join(temporary,'fixture');
  execFileSync('clang++',['-std=c++14','-O2','-Wall','-Wextra',path.join(project,'src-tauri/src/native_genesis_noise.cpp'),path.join(project,'src-tauri/src/native_genesis_noise_fixture.cpp'),'-o',binary],{stdio:'pipe'});
  let cases=0,maximum=0;
  for(const rate of [44100,48000,96000])for(const scenario of scenarios) {
    const frames=16384,events=scenario.events.map((event,index)=>({...event,index})).sort((a,b)=>a.frame-b.frame||a.index-b.index);
    const engine=new NativeFXEngine(wasm,rate),tokens=new Map();let tokenSequence=0;
    function apply(event){
      const slot=event.slot??0,base={op:'noise',slot,token:tokens.get(slot)};
      switch(event.operation){
        case 0:tokens.set(slot,++tokenSequence);engine.command({...base,action:'create',token:tokenSequence,type:event.argument,seed:event.seed});break;
        case 1:engine.command({...base,action:'parameter',param:event.argument,value:event.value,seconds:(event.frames??0)/rate});break;
        case 2:engine.command({...base,action:'filter',mode:event.argument});break;
        case 3:engine.command({...base,action:['start','stop','dispose'][event.argument]});break;
        case 4:engine.resetNoise();break;
      }
    }
    const input=Buffer.alloc(12+events.length*32+frames*8);input.writeUInt32LE(rate,0);input.writeUInt32LE(frames,4);input.writeUInt32LE(events.length,8);
    events.forEach((event,index)=>{const offset=12+index*32;input.writeUInt32LE(event.frame,offset);input.writeInt32LE(event.operation,offset+4);input.writeInt32LE(event.slot??0,offset+8);input.writeInt32LE(event.argument??0,offset+12);input.writeDoubleLE(event.value??0,offset+16);input.writeInt32LE(event.frames??0,offset+24);input.writeUInt32LE(event.seed??0,offset+28);});
    const expected=new Float32Array(frames*2);let cursor=0;
    for(let frame=0;frame<frames;){
      while(cursor<events.length&&events[cursor].frame===frame)apply(events[cursor++]);
      const count=Math.min(128,frames-frame,(events[cursor]?.frame??frames)-frame);
      const source=[new Float32Array(count),new Float32Array(count)],target=[new Float32Array(count),new Float32Array(count)];
      if(scenario.input)for(let i=0;i<count;i++){
        source[0][i]=Math.sin(2*Math.PI*330*(frame+i)/rate)*.03;source[1][i]=Math.cos(2*Math.PI*557*(frame+i)/rate)*.02;
        const offset=12+events.length*32+(frame+i)*8;input.writeFloatLE(source[0][i],offset);input.writeFloatLE(source[1][i],offset+4);
      }
      engine.process(source,target);
      for(let i=0;i<count;i++){expected[(frame+i)*2]=target[0][i];expected[(frame+i)*2+1]=target[1][i];}frame+=count;
    }
    const inputPath=path.join(temporary,'input.bin'),outputPath=path.join(temporary,'output.bin');await fs.writeFile(inputPath,input);execFileSync(binary,[inputPath,outputPath],{stdio:'pipe'});
    const output=await fs.readFile(outputPath);assert.equal(output.length,expected.byteLength);
    let difference=0,energy=0;
    for(let i=0;i<expected.length;i++){const value=output.readFloatLE(i*4);assert(Number.isFinite(value),`${scenario.name}@${rate} nonfinite`);difference=Math.max(difference,Math.abs(value-expected[i]));energy+=value*value;if(scenario.silentAfter!==undefined&&i>=scenario.silentAfter*2)assert.equal(value,0,`${scenario.name}@${rate} unreleased voice`);}
    assert(energy>0,`${scenario.name}@${rate} unexpectedly silent`);assert(difference<2e-6,`${scenario.name}@${rate} native/WASM difference ${difference}`);
    cases++;maximum=Math.max(maximum,difference);console.log(`${scenario.name}@${rate}: maximum difference ${difference}`);
  }
  console.log(`Native noise: ${cases} actual native/WASM cases passed; maximum difference ${maximum}; instance isolation checked in every case`);
} finally {await fs.rm(temporary,{recursive:true,force:true});}
