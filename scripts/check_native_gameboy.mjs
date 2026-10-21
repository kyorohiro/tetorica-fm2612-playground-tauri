#!/usr/bin/env node
// Offline PCM comparison against the shipped browser WASM. No output device.
import {readFile,writeFile,mkdtemp} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {dirname,resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {fileURLToPath,pathToFileURL} from 'node:url';
const repository=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const option=name=>{const equal=process.argv.find(v=>v.startsWith(name+'='));if(equal)return equal.slice(name.length+1);const index=process.argv.indexOf(name);return index<0?undefined:process.argv[index+1];};
const source=resolve(option('--source')??resolve(repository,'../..'));
const out=option('--out');
const work=await mkdtemp(resolve(tmpdir(),'tetorica-native-gameboy-'));
const executable=resolve(work,'fixture');
const build=spawnSync('clang++',['-std=c++17','-O2','-ffp-contract=off','-fwrapv','-fno-strict-aliasing',`-I${resolve(repository,'src-tauri/native-gameboy/snapshot/third_party/mame-gameboy')}`,resolve(repository,'src-tauri/src/native_genesis_gameboy_fixture.cpp'),resolve(repository,'src-tauri/src/native_genesis_gameboy.cpp'),resolve(repository,'src-tauri/native-gameboy/snapshot/third_party/mame-gameboy/gameboy_apu.cpp'),'-o',executable],{encoding:'utf8'});
if(build.status!==0)throw new Error(build.stderr);
const self=spawnSync(executable,['--self-test'],{encoding:'utf8'});if(self.status!==0)throw new Error(self.stderr);
const {GameboyApu}=await import(pathToFileURL(resolve(source,'web/gameboyapu.js')));
const {ChipClockResampler}=await import(pathToFileURL(resolve(source,'web/playground_chip_clock.js')));
const {default:factory}=await import(pathToFileURL(resolve(source,'docs/generated/gameboy_apu_wasm.js')));
const wasmBinary=await readFile(resolve(source,'docs/generated/gameboy_apu_wasm.wasm'));
const W=(frame,address,value)=>({frame,op:'write',address,value});
const C=(frame,value)=>({frame,op:'clock',address:0,value});
const base=[W(0,22,128),W(0,20,0x77),W(0,21,0xff)];
const pulse=(frame,ch,period,duty=2,envelope=0xa0)=>[W(frame,ch*5+1,duty<<6),W(frame,ch*5+2,envelope),W(frame,ch*5+3,period&255),W(frame,ch*5+4,128|(period>>8))];
const wave=(frame,period)=>[W(frame,10,0),...Array.from({length:16},(_,i)=>W(frame,32+i,(i<<4)|(15-i))),W(frame,12,0x20),W(frame,10,128),W(frame,13,period&255),W(frame,14,128|(period>>8))];
const noise=(frame,value)=>[W(frame,17,0xa1),W(frame,18,value),W(frame,19,128)];
const cases=[];
for(let duty=0;duty<4;duty++)cases.push({name:`pulse-duty-${duty}`,frames:8192,events:[...base,...pulse(0,0,1750,duty),...pulse(127,1,1930,3-duty),W(4097,2,0)]});
cases.push({name:'pulse-sweep-envelope-length',frames:20000,events:[...base,W(0,0,0x11),...pulse(0,0,1200,2,0xf2),W(2048,0,0x19),W(3000,4,0xc4),W(7000,6,63),...pulse(7000,1,1860,1,0x39),W(7001,9,0xc7)]});
cases.push({name:'wave-accessibility',frames:8192,events:[...base,...wave(0,1900),...Array.from({length:64},(_,i)=>W(100+i*13,32+(i%16),(i*37)&255)),W(1900,10,0),...wave(1901,1980),W(3000,12,0x40),W(4000,12,0x60),W(6000,10,0)]});
for(const nr43 of [0,0x17,0x39,0x67])cases.push({name:`noise-${nr43}`,frames:8192,events:[...base,...noise(0,nr43),W(2049,18,nr43^8),W(4099,17,0)]});
cases.push({name:'pan-master-power',frames:8192,events:[...base,...pulse(0,0,1700),...pulse(0,1,1900),...wave(0,1950),...noise(0,0x29),W(1000,21,0x51),W(2000,20,0x21),W(3000,22,0),W(3001,32,0xf1),W(4000,22,128),W(4000,20,0x77),W(4000,21,0xff),...pulse(4000,0,1800),W(5000,21,0x10)]});
cases.push({name:'outer-clock-resampling',frames:14000,events:[...base,...pulse(0,0,1830),...wave(0,1970),...noise(0,0x31),C(999,2097152),C(1999,8388608),C(3001,3579545),C(5001,4194304),C(6001,100000),C(6501,100000000),C(6701,4194304),W(9000,2,0)]});
cases.push({name:'reset-mid-note',frames:8192,events:[...base,...wave(0,1980),...pulse(0,0,1750),{frame:3000,op:'reset',address:0,value:0},...base.map(e=>({...e,frame:3010})),...pulse(3010,1,1900),...wave(3011,1900)]});
let seed=0x1842;const random=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed;};
const raw=[...base];for(let i=0;i<300;i++){const address=random()%48,value=random()&255;raw.push(W(1+i*23,address,address===22?128:value));}
cases.push({name:'deterministic-raw-writes',frames:8192,events:raw});
const report={date:'2026-10-22',reference:'existing docs/generated/gameboy_apu_wasm.wasm',deviceOpened:false,selfTest:JSON.parse(self.stderr),cases:[],maximumDifference:0,stereoFrames:0};
for(const rate of [44100,48000,96000])for(const test of cases){
  const events=test.events.slice().sort((a,b)=>a.frame-b.frame),input=events.map(e=>`${e.frame} ${e.op} ${e.address} ${e.value}`).join('\n')+'\n';
  const native=spawnSync(executable,[String(rate),String(test.frames)],{input,maxBuffer:32*1024*1024});if(native.status!==0)throw new Error(`${test.name}: ${native.stderr}`);
  const pcm=new Float32Array(native.stdout.buffer,native.stdout.byteOffset,native.stdout.byteLength/4);
  const chip=await GameboyApu.create({moduleFactory:factory,moduleOptions:{wasmBinary},sampleRate:rate});
  const resampler=new ChipClockResampler();let frame=0,index=0,max=0;
  try{while(frame<test.frames){while(index<events.length&&events[index].frame===frame){const e=events[index++];if(e.op==='write')chip.writeRegister(e.address,e.value);else if(e.op==='clock')resampler.setClock(e.value,4194304);else if(e.op==='reset'){chip.reset();resampler.resetPhase();}}
    const end=Math.min(test.frames,events[index]?.frame??test.frames,frame+257),left=new Float32Array(end-frame),right=new Float32Array(end-frame);resampler.render(chip,left,right);
    for(let i=0;i<left.length;i++){max=Math.max(max,Math.abs(left[i]-pcm[(frame+i)*2]),Math.abs(right[i]-pcm[(frame+i)*2+1]));}
    frame=end;
  }}finally{chip.dispose();}
  if(max!==0)throw new Error(`${test.name}@${rate}: PCM difference ${max}`);
  report.cases.push({name:test.name,sampleRate:rate,frames:test.frames,maximumDifference:max});report.stereoFrames+=test.frames;report.maximumDifference=Math.max(report.maximumDifference,max);
}
if(out)await writeFile(out,JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({cases:report.cases.length,stereoFrames:report.stereoFrames,maximumDifference:report.maximumDifference,selfTest:report.selfTest,deviceOpened:false}));
