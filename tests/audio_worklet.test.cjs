const test=require('node:test'),assert=require('node:assert/strict'),vm=require('node:vm'),fs=require('node:fs'),path=require('node:path');
function processor(frames=256,outputSampleRate=44100){let Class;const messages=[];
  vm.runInNewContext(fs.readFileSync(path.join(__dirname,'../desktop/audio-worklet.js'),'utf8'),{
    Float32Array,sampleRate:44100,AudioWorkletProcessor:class{constructor(){this.port={postMessage:buffer=>messages.push(new Float32Array(buffer))};}},
    registerProcessor(_name,value){Class=value;}});
  return {instance:new Class({processorOptions:{frames,outputSampleRate}}),messages};
}
test('credits bound PCM capture and packets retain stereo order while WebView output stays silent',()=>{
  const {instance,messages}=processor();const input=[new Float32Array(128).fill(.1),new Float32Array(128).fill(.2)],out=[new Float32Array(128).fill(1),new Float32Array(128).fill(1)];
  for(let i=0;i<20;i++)instance.process([input],[out]);assert.equal(messages.length,0);assert.ok(out.every(channel=>channel.every(sample=>sample===0)));
  instance.port.onmessage({data:{type:'credit',count:2}});for(let i=0;i<20;i++)instance.process([input],[out]);
  assert.equal(messages.length,2);assert.equal(messages[0].length,512);assert.ok(Math.abs(messages[0][0]-.1)<1e-6);assert.ok(Math.abs(messages[0][1]-.2)<1e-6);
});
test('after credit starvation, fresh packets contain current audio and stop discards partial frames',()=>{
  const {instance,messages}=processor();const render=value=>instance.process([[new Float32Array(128).fill(value)]],[[new Float32Array(128)]]);
  instance.port.onmessage({data:{type:'credit',count:1}});render(1);instance.port.onmessage({data:{type:'stop'}});render(9);
  instance.port.onmessage({data:{type:'credit',count:1}});render(2);render(2);assert.ok(messages[0].every(sample=>sample===2));
});
test('resampling follows the device rate without changing constant PCM amplitude',()=>{
  for(const rate of [48000,32000]){
    const {instance,messages}=processor(256,rate);
    for(let frame=0;frame<44100;frame+=128){
      instance.port.onmessage({data:{type:'credit',count:4}});
      instance.process([[new Float32Array(Math.min(128,44100-frame)).fill(.5)]],[[new Float32Array(128)]]);
    }
    const frames=messages.length*256+instance.offset;assert.ok(Math.abs(frames-rate)<=2);
    assert.ok(messages.every(packet=>packet.every(value=>value===.5)));
  }
});
