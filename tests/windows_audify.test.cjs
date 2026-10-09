const test=require('node:test'),assert=require('node:assert/strict');
const {readFileSync}=require('node:fs');

test('pinned WASAPI patch releases the enumerator before COM shutdown and is repeatable',async()=>{
  const {patchWasapi}=await import('../scripts/windows_audify.mjs');
  const source=readFileSync(require.resolve('audify/vendor/rtaudio/RtAudio.cpp'),'utf8');
  for(const newline of ['\n','\r\n']){
    const input=source.replaceAll('\r\n','\n').replaceAll('\n',newline);
    const patched=patchWasapi(input);
    const start=patched.indexOf('RtApiWasapi::~RtApiWasapi()');
    const end=patched.indexOf('//-----------------------------------------------------------------------------',start);
    const destructor=patched.slice(start,end);
    assert.ok(destructor.indexOf('deviceEnumerator_.Reset();')>=0);
    assert.ok(destructor.indexOf('deviceEnumerator_.Reset();')<destructor.indexOf('CoUninitialize();'));
    assert.equal(patchWasapi(patched),patched);
    assert.equal(patched.slice(0,start),input.slice(0,start));
    assert.equal(patched.slice(end),input.slice(input.indexOf('//-----------------------------------------------------------------------------',start)));
  }
});

test('an incompatible upstream destructor fails instead of silently shipping an unpatched binary',async()=>{
  const {patchWasapi}=await import('../scripts/windows_audify.mjs');
  assert.throws(()=>patchWasapi(''),/Unsupported/);
  assert.throws(()=>patchWasapi('RtApiWasapi::~RtApiWasapi() {}\n//-----------------------------------------------------------------------------'),/Unsupported/);
});
