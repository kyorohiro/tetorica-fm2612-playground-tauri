// Exercise the actual Rust native scheduler/media renderer and actual browser
// WASM engines with identical PCM and timestamps. No output device is opened.
import assert from 'node:assert/strict';
import {readFileSync,mkdtempSync,rmSync} from 'node:fs';
import {join,resolve,dirname} from 'node:path';
import {tmpdir} from 'node:os';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {spawnSync} from 'node:child_process';
const repo=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const source=resolve(process.argv[2]??join(repo,'../..'));
const directory=mkdtempSync(join(tmpdir(),'tetorica-native-media-'));
try{
 const path=join(directory,'pcm.json');
 const result=spawnSync('cargo',['test','--offline','--locked','--manifest-path',join(repo,'src-tauri/Cargo.toml'),'native_genesis::tests::native_media_pcm_fixture','--','--exact'],{encoding:'utf8',env:{...process.env,TETORICA_NATIVE_MEDIA_FIXTURE:path}});
 assert.equal(result.status,0,result.stdout+result.stderr);
 const fixtures=JSON.parse(readFileSync(path,'utf8'));
 const [factory,psgFactory,fmApi,psgApi,dac,sample]=await Promise.all([
  import(pathToFileURL(join(source,'docs/generated/ym2612_wasm.js'))),import(pathToFileURL(join(source,'docs/generated/segapsg_wasm.js'))),
  import(pathToFileURL(join(source,'web/ym2612.js'))),import(pathToFileURL(join(source,'web/segapsg.js'))),
  import(pathToFileURL(join(source,'web/ym2612_dac.js'))),import(pathToFileURL(join(source,'web/native_sample_processor.js'))),
 ]);
 const effectModule=await WebAssembly.compile(readFileSync(join(source,'web/native_audio_effect.wasm')));
 const worklet=readFileSync(join(source,'web/ym2612-worklet.js'),'utf8');
 const begin=worklet.indexOf('  renderFrames(leftOut, rightOut, offset, frames) {'),end=worklet.indexOf('\n  flushMixerMeters(',begin);
 assert(begin>=0&&end>begin);
 const method=worklet.slice(begin,end).trim().replace(/^renderFrames/,'function');
 for(const fixture of fixtures){
  const rate=fixture.sampleRate,frames=fixture.pcm.length,scenario=fixture.scenario;
  const output=[new Float32Array(frames),new Float32Array(frames)];
  if(scenario==='sample'){
   const api=new WebAssembly.Instance(effectModule,{env:{emscripten_notify_memory_growth(){}}}).exports;api._initialize?.();
   const processor=sample.createSampleProcessor(api,rate);
   const left=Float32Array.from({length:1024},(_,i)=>Math.fround(Math.sin(i*.12))*.2),right=Float32Array.from({length:1024},(_,i)=>Math.fround(Math.cos(i*.17))*.15);
   processor.command({action:'load',name:'pcm',pcm:{sampleRate:44100,channels:[left,right]}});
   const cap=api.gain_capacity();const input=new Float32Array(api.memory.buffer,api.gain_input(),cap*2);
   for(let frame=0;frame<frames;frame++){
    if(frame===137)processor.command({action:'play',name:'pcm',options:{playbackRate:-.75,gain:.7,pan:-.35,offset:.001,duration:.035,loop:true,loopStart:.002,loopEnd:.018,fadeIn:.002,fadeOut:.003}});
    input[0]=input[cap]=0;api.sample_mix(1);output[0][frame]=input[0];output[1][frame]=input[cap];
   }
  }else{
   const ym=await fmApi.createYm2612(factory.default,{wasmBinary:readFileSync(join(source,'docs/generated/ym2612_wasm.wasm'))});
   const psg=await psgApi.SegaPSG.create({moduleFactory:psgFactory.default,moduleOptions:{wasmBinary:readFileSync(join(source,'docs/generated/segapsg_wasm.wasm'))},sampleRate:ym.sampleRate(),clock:3579545});
   try{
    const idle=ym.generateStereoView(1);const state={ym2612:ym,psg,resampleRemainder:0,psgRemainder:0,lastLeft:0,lastRight:0,psgLeft:0,psgRight:0,idleLeft:idle.left[0],idleRight:idle.right[0],mixRemaining:{ym2612:0,segapsg:0},mixCurrent:{ym2612:[1,1],segapsg:[1,1]},mixMeterEnabled:{ym2612:false,segapsg:false}};ym.reset();
    const render=new Function('sampleRate','YM_GAIN','PSG_GAIN','MIX_SOURCES','clampSample',`return ${method}`)(rate,.9,.35,['ym2612','segapsg'],value=>Math.max(-1,Math.min(1,value)));
    const player=new dac.YM2612DacPlayer(rate,(p,r,v)=>ym.writeRegister(r,v,p));
    if(scenario==='dac')player.command({action:'load',name:'dac',sampleRate:22050,data:Uint8Array.from({length:257},(_,i)=>Math.round(128+110*Math.sin(i*.19)))},0);
    let index=0;
    for(let frame=0;frame<frames;frame++){
     if(scenario==='dac'){
      if(frame===137)player.command({action:'play',name:'dac',offset:13,size:211,pan:'left'},frame);
      player.advance(frame);
     }else{
      if(frame===137){ym.writeRegister(0xb6,.0+0x40,1);ym.writeRegister(0x2b,0x80,0);}
      while(index<257&&frame>=137+Math.round(index*2*rate/44100)){ym.writeRegister(0x2a,Math.round(128+100*Math.sin(index*.2)),0);index++;}
      if(frame===3000)ym.writeRegister(0x2b,0,0);
     }
     render.call(state,output[0],output[1],frame,1);
    }
   }finally{ym.dispose();psg.dispose();}
  }
  let difference=0,peak=0;
  for(let i=0;i<frames;i++)for(let channel=0;channel<2;channel++){
   const actual=fixture.pcm[i][channel],expected=output[channel][i];assert(Number.isFinite(actual)&&Number.isFinite(expected));peak=Math.max(peak,Math.abs(actual));difference=Math.max(difference,Math.abs(actual-expected));
  }
  assert(peak>.01);assert(difference<2e-7,`${scenario}/${rate}: maximum native/WASM difference ${difference}`);
  console.log(`${scenario} ${rate} Hz: native/WASM PCM max difference ${difference}`);
 }
}finally{rmSync(directory,{recursive:true,force:true});}
