const test=require('node:test'),assert=require('node:assert/strict');
test('Windows enumeration selects WASAPI explicitly; other platforms preserve the default',async()=>{
  const {outputApi}=await import('../audio-sidecar/audio_api.mjs');
  const audify={RtAudioApi:{WINDOWS_ASIO:6,WINDOWS_WASAPI:7}};
  assert.equal(outputApi(audify,'win32'),7);assert.equal(outputApi(audify,'darwin'),undefined);assert.equal(outputApi(audify,'linux'),undefined);
  assert.throws(()=>outputApi({},'win32'),/WASAPI/);
});
test('the output adapter uses the same selected API as device enumeration',async()=>{
  const {createOutput}=await import('../audio-sidecar/output_audify.mjs');
  const moduleUrl='data:text/javascript,'+encodeURIComponent(`
    export const state={};export const RtAudioFormat={RTAUDIO_FLOAT32:16};
    export class RtAudio{constructor(...args){state.args=args;}openStream(){return 512;}
      getDefaultOutputDevice(){return 1;}getStreamSampleRate(){return 48000;}
      isStreamOpen(){return true;}isStreamRunning(){return false;}
      clearOutputQueue(){}closeStream(){} }
  `);
  const {state}=await import(moduleUrl);
  const options={moduleUrl,sampleRate:48000,bufferFrames:512,onDrain(){},onError(){}};
  const explicit=await createOutput({...options,api:7});assert.deepEqual(state.args,[7]);explicit.close();
  const automatic=await createOutput(options);assert.deepEqual(state.args,[]);automatic.close();
});
