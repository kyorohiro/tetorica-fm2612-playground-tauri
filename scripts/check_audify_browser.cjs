// Run prepare_audio_sidecar.mjs first. Uses a temporary docs server and real low-volume hardware output.
const {createServer}=require('node:http');const {chromium}=require('playwright');const {spawn}=require('node:child_process');const {createInterface}=require('node:readline');const path=require('node:path');const fs=require('node:fs');const assert=require('node:assert/strict');
const root=path.resolve(__dirname,'..');
const child=spawn(path.join(root,'audio-sidecar-bundle',process.platform==='win32'?'node.exe':'node'),[path.join(root,'audio-sidecar-bundle/server.mjs')],{stdio:['pipe','pipe','inherit']});
let id=0;const pending=new Map();createInterface({input:child.stdout}).on('line',line=>{const response=JSON.parse(line),request=pending.get(response.id);pending.delete(response.id);if(request)response.error?request.reject(Error(response.error)):request.resolve(response.result);});
child.on('exit',()=>{for(const request of pending.values())request.reject(Error('Sidecar exited'));pending.clear();});
const request=(op,args)=>new Promise((resolve,reject)=>{const key=++id;pending.set(key,{resolve,reject});child.stdin.write(JSON.stringify({id:key,op,args})+'\n');});
(async()=>{let browser;const docs=path.resolve(root,'../../docs');const assets=createServer((req,res)=>{
  try{const pathname=decodeURIComponent(new URL(req.url,'http://localhost').pathname);const filename=path.resolve(docs,'.'+pathname);if(!filename.startsWith(docs+path.sep))throw Error('Bad path');
    let body;if(pathname.endsWith('/desktop-audio-worklet.js'))body=fs.readFileSync(path.join(root,'desktop/audio-worklet.js'));
    else{body=fs.readFileSync(filename);if(pathname==='/playground/playground.js')body=Buffer.concat([body,Buffer.from('\n;\n'+fs.readFileSync(path.join(root,'desktop/audio-interface.js'),'utf8'))]);}
    const extension=path.extname(pathname);res.setHeader('Content-Type',extension==='.wasm'?'application/wasm':extension==='.html'?'text/html':extension==='.js'?'text/javascript':extension==='.json'?'application/json':'application/octet-stream');res.end(body);
  }catch{res.writeHead(404);res.end();}
});await new Promise(resolve=>assets.listen(0,'127.0.0.1',resolve));try{
  const devices=await request('devices');assert.ok(devices.length);browser=await chromium.launch({headless:true,args:['--autoplay-policy=no-user-gesture-required']});const page=await browser.newPage();
  await page.exposeBinding('nativeAudio',(_source,op,args)=>request(op,args));
  await page.addInitScript(()=>{window.__TAURI_INTERNALS__={invoke:(command,args)=>command==='audio_request'?window.nativeAudio(args.op,args.args):command==='audio_shutdown'?window.nativeAudio('stop'):Promise.resolve()};});
  await page.goto(`http://127.0.0.1:${assets.address().port}/playground/index.html`);await page.waitForFunction(()=>window.monaco?.editor?.getEditors().length);
  assert.equal(await page.locator('#desktop-audio-mode').inputValue(),'webview');assert.equal(await page.locator('#desktop-audio-device').isVisible(),false);
  await page.locator('#mainMenu').evaluate(menu=>menu.open=true);await page.locator('#desktop-audio-mode').selectOption('audify');await page.waitForFunction(()=>!document.getElementById('desktop-audio-mode').disabled);assert.ok(await page.locator('#desktop-audio-device option').count()>1);
  await page.locator('#desktop-audio-device').selectOption(String(devices.find(device=>device.isDefault)?.id??devices[0].id));await page.getByRole('button',{name:'Apply output',exact:true}).click();await page.waitForFunction(()=>!document.getElementById('desktop-audio-mode').disabled);assert.match(await page.locator('#desktop-audio-status').textContent(),/^Audify →/);
  await page.locator('#mainMenu').evaluate(menu=>menu.open=false);
  await page.locator('#masterVolumeRange').evaluate(input=>{input.value='1';input.dispatchEvent(new Event('input',{bubbles:true}));});
  const source=`setBpm(120); liveLoop('audio-test',async()=>{await play('C4',{channel:CH1,duration:0.1});await sleep(0.2);});`;
  await page.evaluate(source=>monaco.editor.getEditors().find(editor=>editor.getModel()?.uri.path.startsWith('/project/')).setValue(source),source);
  for(const worker of [false,true]){
    await page.waitForTimeout(80);await request('status',{reset:true});
    await page.locator('#workerExecution').setChecked(worker);await page.locator('#runButton').click();await page.waitForTimeout(700);const status=await request('status');assert.ok(status.peak>0,JSON.stringify({status,console:await page.locator('#consoleOutput').textContent(),audio:await page.locator('#desktop-audio-status').textContent()}));assert.ok(status.output.consumedFrames>0);assert.ok(status.output.queuedFrames<=status.output.bufferFrames*4);
    await page.locator('#stopButton').click();await page.waitForFunction(()=>!document.getElementById('stopButton').disabled);
  }
  await page.locator('#mainMenu').evaluate(menu=>menu.open=true);await page.locator('#desktop-audio-mode').selectOption('webview');await page.waitForFunction(()=>!document.getElementById('desktop-audio-mode').disabled);assert.equal((await request('status')).active,false);
  await page.locator('#desktop-audio-mode').selectOption('audify');await page.waitForFunction(()=>!document.getElementById('desktop-audio-mode').disabled);await page.getByRole('button',{name:'Apply output',exact:true}).click();await page.waitForFunction(()=>!document.getElementById('desktop-audio-mode').disabled);assert.match(await page.locator('#desktop-audio-status').textContent(),/^Audify →/);
  await request('stop');await page.waitForFunction(()=>document.getElementById('desktop-audio-mode').value==='webview');assert.match(await page.locator('#desktop-audio-status').textContent(),/Audify stopped/);
  console.log(JSON.stringify({passed:true,devices:devices.length,selectedDevice:true,actualAudifyPCM:true,mainAndWorker:true,boundedQueue:true,webviewDefault:true,disconnectRecovery:true}));
}finally{await browser?.close();assets.close();child.stdin.end();setTimeout(()=>child.kill(),2000).unref();}})().catch(error=>{console.error(error);process.exitCode=1;});
