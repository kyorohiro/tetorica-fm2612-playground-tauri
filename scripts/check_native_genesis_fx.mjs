/** Compare actual native C++ and shipped WASM DSP, without a physical device. */
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {execFileSync} from 'node:child_process';

const project=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const upstream=path.resolve(project,process.argv[2]??'../..');
const {NativeFXEngine}=await import(pathToFileURL(path.join(upstream,'web/native_fx_engine.js')));
const {createNativeFXController,FX_TYPES}=await import(pathToFileURL(path.join(upstream,'web/native_fx.js')));
const wasm=await WebAssembly.compile(await fs.readFile(path.join(upstream,'web/native_audio_effect.wasm')));
const temp=await fs.mkdtemp(path.join(os.tmpdir(),'tetorica-native-fx-'));
try {
  const binary=path.join(temp,'fixture');
  execFileSync('clang++',['-std=c++14','-O2','-Wall','-Wextra',path.join(project,'src-tauri/src/native_genesis_fx.cpp'),path.join(project,'src-tauri/src/native_genesis_fx_fixture.cpp'),'-o',binary],{stdio:'pipe'});
  const cases=[...Object.keys(FX_TYPES),'parallel'];
  let total=0,maximum=0;
  for(const rate of [44100,48000,96000])for(const name of cases) {
    const frames=16384,engine=new NativeFXEngine(wasm,rate),messages=[];
    const fx=createNativeFXController(message=>{messages.push(message);engine.command(structuredClone(message));});
    if(name==='parallel')fx.setChain([fx.eq({bass:3,mid:-2,treble:2}),fx.parallel(fx.branch(fx.gain({gain:.6})),fx.branch(fx.delay({time:.023,feedback:.35,mix:1}),fx.reverb({mix:.25,room:.65}),fx.gain({gain:.3})))]);
    else fx.setChain([fx[name]()]);
    const command=messages.at(-1),nodes=[];
    function add(node) {
      const children=(node.children??[]).map(add),id=nodes.length;
      nodes.push({type:node.type==='chain'?0:node.type==='parallel'?1:FX_TYPES[node.type],slot:node.slot??0,children});return id;
    }
    add({type:'chain',children:command.children});
    const keyOrder={gain:['gain'],eq:['bass','mid','treble'],gate:['threshold','hysteresis','attack','hold','release'],compressor:['threshold','ratio','attack','release','makeup'],reverb:['mix','room','damping','tone'],filter:['cutoff','q','mode','mix'],delay:['time','feedback','mix'],distortion:['drive','mix'],bitcrusher:['bitDepth','holdFrames','mix'],wobble:['cutoff','depth','rate','resonance','mix'],flanger:['time','depth','rate','feedback','mix'],slicer:['phase','duty','floor','mix'],chorus:['time','depth','rate','mix']};
    const input=Buffer.alloc(24+nodes.length*140+command.units.length*72+frames*8);
    input.writeUInt32LE(rate,0);input.writeUInt32LE(nodes.length,4);input.writeUInt32LE(command.units.length,8);input.writeUInt32LE(frames,12);input.writeDoubleLE(.5,16);
    let cursor=24;
    for(const node of nodes){input.writeInt32LE(node.type,cursor);input.writeInt32LE(node.slot,cursor+4);input.writeInt32LE(node.children.length,cursor+8);node.children.forEach((id,index)=>input.writeInt32LE(id,cursor+12+index*4));cursor+=140;}
    for(const unit of command.units){input.writeInt32LE(FX_TYPES[unit.type],cursor);input.writeInt32LE(unit.slot,cursor+4);keyOrder[unit.type].forEach((key,index)=>input.writeDoubleLE(unit.values[key]??(key==='mix'?1:0),cursor+8+index*8));cursor+=72;}
    const expected=new Float32Array(frames*2);let random=1;
    for(let offset=0;offset<frames;offset+=128){
      const source=[new Float32Array(128),new Float32Array(128)],target=[new Float32Array(128),new Float32Array(128)];
      for(let i=0;i<128;i++){random=(Math.imul(random,1664525)+1013904223)>>>0;const noise=(random/2**32-.5)*.05;source[0][i]=Math.sin(2*Math.PI*330*(offset+i)/rate)*.1+noise;source[1][i]=Math.sin(2*Math.PI*557*(offset+i)/rate)*.08-noise;input.writeFloatLE(source[0][i],cursor+(offset+i)*8);input.writeFloatLE(source[1][i],cursor+(offset+i)*8+4);}
      engine.process(source,target);
      for(let i=0;i<128;i++){expected[(offset+i)*2]=target[0][i];expected[(offset+i)*2+1]=target[1][i];}
    }
    const inPath=path.join(temp,'input.bin'),outPath=path.join(temp,'output.bin');await fs.writeFile(inPath,input);execFileSync(binary,[inPath,outPath],{stdio:'pipe'});
    const output=await fs.readFile(outPath);assert.equal(output.length,expected.byteLength);
    let difference=0;for(let i=0;i<expected.length;i++){const value=output.readFloatLE(i*4);assert.ok(Number.isFinite(value),`${name}@${rate}: nonfinite PCM`);difference=Math.max(difference,Math.abs(value-expected[i]));}
    assert.ok(difference<2e-6,`${name}@${rate}: native/WASM difference ${difference}`);maximum=Math.max(maximum,difference);total++;
    console.log(`${name}@${rate}: maximum difference ${difference}`);
  }
  console.log(`Native FX: ${total} actual native/WASM cases passed; maximum difference ${maximum}`);
} finally {await fs.rm(temp,{recursive:true,force:true});}
