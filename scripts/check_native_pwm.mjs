#!/usr/bin/env node
// Native C ABI vs synchronous JS reference: no audio device or browser.
import {writeFile,mkdtemp} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {dirname,resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {fileURLToPath,pathToFileURL} from 'node:url';
const repository=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const option=name=>{const equal=process.argv.find(v=>v.startsWith(name+'='));if(equal)return equal.slice(name.length+1);const index=process.argv.indexOf(name);return index<0?undefined:process.argv[index+1];};
const source=resolve(option('--source')??resolve(repository,'../..')),out=option('--out');
const work=await mkdtemp(resolve(tmpdir(),'tetorica-native-pwm-')),executable=resolve(work,'fixture');
const build=spawnSync('clang++',['-std=c++17','-O2','-ffp-contract=off',`-I${resolve(repository,'src-tauri/native-pwm/snapshot/wasm')}`,resolve(repository,'src-tauri/src/native_genesis_pwm_fixture.cpp'),resolve(repository,'src-tauri/src/native_genesis_pwm.cpp'),'-o',executable],{encoding:'utf8'});
if(build.status!==0)throw new Error(build.stderr);
const self=spawnSync(executable,['--self-test'],{encoding:'utf8'});if(self.status!==0)throw new Error(self.stderr);
const {PWM32X}=await import(pathToFileURL(resolve(source,'web/pwm32x.js')));
const W=(frame,address,value)=>({frame,op:'write',address,value}),C=(frame,value)=>({frame,op:'clock',address:0,value});
const stream=(control,cycle,count=500)=>[W(0,0,control),W(0,1,cycle),...Array.from({length:count},(_,i)=>W(i*3,2+i%3,(i*213+47)%4096))];
const cases=[];
for(const control of [0,1,2,4,5,6,9,10,15,0x505])cases.push({name:`routing-${control}`,frames:8192,events:stream(control,1040)});
for(const cycle of [0,1,2,999,4095])cases.push({name:`cycle-${cycle}`,frames:8192,events:stream(5,cycle)});
cases.push({name:'fifo-overflow-underflow-held-output',frames:8192,events:[W(0,0,5),W(0,1,1000),...Array.from({length:8},(_,i)=>W(0,4,i*400)),W(1000,2,777),W(1000,3,999),W(2000,1,1),W(3000,1,4095),W(3500,0,10),W(3500,4,2000)]});
cases.push({name:'clock-continuity',frames:8192,events:[...stream(5,1140),C(100,100000),C(200,100000000),C(300,23011361),C(1000,7670453),C(2000,12000000),C(3000,23011361)]});
cases.push({name:'stop-and-reset',frames:8192,events:[...stream(5,1000,20),{frame:300,op:'stop',address:0,value:0},W(500,4,750),{frame:1000,op:'reset',address:0,value:0},...stream(10,1200,20).map(event=>({...event,frame:event.frame+1100}))]});
let seed=0x32;const random=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed;};
cases.push({name:'raw-deterministic',frames:8192,events:Array.from({length:600},(_,i)=>W(i*11,random()%5,random()&65535))});
const report={date:'2026-10-22',reference:'synchronous JavaScript PWM32X',deviceOpened:false,selfTest:JSON.parse(self.stderr),cases:[],maximumDifference:0,stereoFrames:0};
for(const rate of [44100,48000,96000])for(const test of cases){
  const events=test.events.slice().sort((a,b)=>a.frame-b.frame),input=events.map(e=>`${e.frame} ${e.op} ${e.address} ${e.value}`).join('\n')+'\n';
  const native=spawnSync(executable,[String(rate),String(test.frames)],{input,maxBuffer:32*1024*1024});if(native.status!==0)throw new Error(`${test.name}: ${native.stderr}`);
  const pcm=new Float32Array(native.stdout.buffer,native.stdout.byteOffset,native.stdout.byteLength/4),chip=new PWM32X({sampleRate:rate,outputMode:'duty',gain:1});
  let frame=0,index=0,max=0;
  while(frame<test.frames){
    while(index<events.length&&events[index].frame===frame){const event=events[index++];
      if(event.op==='write')chip.write(event.address,event.value);else if(event.op==='clock')chip.clock=event.value;else if(event.op==='reset')chip.reset();
      else if(event.op==='stop'){chip.leftFifo=[];chip.rightFifo=[];chip.left=chip.right=null;}
    }
    const end=Math.min(test.frames,events[index]?.frame??test.frames,frame+257),result=chip.generateStereo(end-frame);
    for(let i=0;i<result.left.length;i++)max=Math.max(max,Math.abs(result.left[i]-pcm[(frame+i)*2]),Math.abs(result.right[i]-pcm[(frame+i)*2+1]));
    frame=end;
  }
  chip.dispose();if(max!==0)throw new Error(`${test.name}@${rate}: PCM difference ${max}`);
  report.cases.push({name:test.name,sampleRate:rate,frames:test.frames,maximumDifference:max});report.stereoFrames+=test.frames;report.maximumDifference=Math.max(report.maximumDifference,max);
}
if(out)await writeFile(out,JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({cases:report.cases.length,stereoFrames:report.stereoFrames,maximumDifference:report.maximumDifference,selfTest:report.selfTest,deviceOpened:false}));
