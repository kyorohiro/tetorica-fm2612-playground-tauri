// Appended to the pinned Playground module, where runtime and stopRun are available.
(() => {
  const menu=document.querySelector('#mainMenu > .toolbar-menu-panel');
  if(!menu||!window.__TAURI_INTERNALS__||document.getElementById('desktop-audio-mode'))return;
  const invoke=(op,args)=>window.__TAURI_INTERNALS__.invoke('audio_request',{op,args:args??null});
  // User-code network guards replace the global constructor while loops run.
  // This connection belongs to the desktop output, not to the user script.
  const AudioSocket=globalThis.WebSocket;
  const group=document.createElement('fieldset');
  const title=document.createElement('legend');title.textContent='Audio output';
  const mode=document.createElement('select');mode.id='desktop-audio-mode';mode.setAttribute('aria-label','Audio output engine');
  mode.add(new Option('WebView (default)','webview'));mode.add(new Option('Audify','audify'));
  const device=document.createElement('select');device.id='desktop-audio-device';device.setAttribute('aria-label','Audio output device');
  const refresh=document.createElement('button');refresh.type='button';refresh.textContent='Refresh devices';
  const apply=document.createElement('button');apply.type='button';apply.textContent='Apply output';
  const status=document.createElement('p');status.setAttribute('role','status');status.id='desktop-audio-status';status.textContent='WebView output';
  const details=document.createElement('textarea');details.id='desktop-audio-error';details.hidden=true;
  details.readOnly=true;details.rows=8;details.setAttribute('aria-label','Last Audify error');
  Object.assign(details.style,{width:'100%',boxSizing:'border-box',resize:'vertical',userSelect:'text'});
  const copyError=document.createElement('button');copyError.type='button';copyError.textContent='Copy error';copyError.hidden=true;
  const clearError=document.createElement('button');clearError.type='button';clearError.textContent='Clear error';clearError.hidden=true;
  copyError.addEventListener('click',async()=>{
    try{await navigator.clipboard.writeText(details.value);}
    catch{details.focus();details.select();}
  });
  clearError.addEventListener('click',()=>{details.hidden=copyError.hidden=clearError.hidden=true;details.value='';});
  const hint=document.createElement('p');hint.textContent='Changing output stops playback. Select a device, apply, then Run.';
  group.append(title,mode,device,refresh,apply,status,details,copyError,clearError,hint);menu.append(group);
  device.hidden=true;refresh.hidden=true;apply.hidden=true;
  let session=null,busy=false,loadedContext=null;
  const show=()=>{device.hidden=refresh.hidden=apply.hidden=mode.value!=='audify';};
  function reportError(error){
    const description=String(error.message??error);
    status.textContent=`WebView output — ${description.split('\n')[0]}`;status.title=description;
    details.value=description;details.hidden=copyError.hidden=clearError.hidden=false;
    logLine('[Audify] '+description);
  }
  function detach(){
    const old=session;session=null;if(!old)return;
    old.ws.onclose=null;old.ws.onerror=null;old.ws.close();
    old.tap.port.postMessage({type:'stop'});old.tap.port.onmessage=null;
    try{old.source.disconnect(old.tap);}catch{}
    try{old.tap.disconnect();}catch{}
    if(old.routed&&old.context.state!=='closed')try{old.source.connect(old.context.destination);}catch{}
  }
  async function refreshDevices(){
    const previous=device.value;const list=await invoke('devices');device.replaceChildren(new Option('System default',''));
    for(const entry of list){
      // Some RtAudio backends return a name in the system's legacy encoding.
      const name=entry.name.includes('\uFFFD')?`${entry.name.split('\uFFFD')[0].trim()||'Audio output'} (device ${entry.id})`:entry.name;
      device.add(new Option(name+(entry.isDefault?' — default':''),String(entry.id)));
    }
    if([...device.options].some(option=>option.value===previous))device.value=previous;
    status.textContent=list.length?'Select a device and Apply output.':'No stereo output devices found.';
  }
  async function connect(){
    await runtime.ensureReady();const context=runtime.megaDrive.audioContext;
    if(loadedContext!==context){await context.audioWorklet.addModule(new URL('./desktop-audio-worklet.js',import.meta.url));loadedContext=context;}
    const info=await invoke('start',{sampleRate:context.sampleRate,deviceId:device.value===''?null:Number(device.value)});
    const ws=new AudioSocket(info.url),tap=new AudioWorkletNode(context,'tetorica-desktop-output',{
      numberOfInputs:1,numberOfOutputs:1,outputChannelCount:[2],processorOptions:{frames:info.frames,outputSampleRate:info.sampleRate}});
    const source=runtime.megaDrive.audio.outputNode??runtime.megaDrive.audio.masterOutputNode;
    const current={ws,tap,source,context};session=current;
    async function failed(message){
      if(session!==current||current.failed)return;current.failed=true;
      reportError(message);
      await stopRun();if(session!==current)return;
      detach();show();void invoke('stop').catch(()=>{});
    }
    tap.port.onmessage=event=>{
      if(session!==current||ws.readyState!==AudioSocket.OPEN)return;
      if(ws.bufferedAmount>info.frames*8*4){void failed('Audio connection stalled');return;}
      ws.send(event.data);
    };
    ws.onmessage=event=>{
      try{const message=JSON.parse(event.data);
        if(message.type==='credit')tap.port.postMessage(message);
        if(message.type==='error')void failed(message.error);
      }catch{void failed('Invalid audio response');}
    };
    try{
      await new Promise((resolve,reject)=>{
        const timeout=setTimeout(()=>reject(Error('Audify connection timed out')),5000);
        ws.onopen=()=>{clearTimeout(timeout);resolve();};
        ws.onerror=()=>{clearTimeout(timeout);reject(Error('Cannot connect to Audify'));};
        ws.onclose=()=>{clearTimeout(timeout);reject(Error('Audify disconnected'));};
      });
      source.disconnect(context.destination);current.routed=true;source.connect(tap);tap.connect(context.destination);
      ws.onerror=()=>{void failed('Audio connection error');};
      ws.onclose=()=>{void failed('Audio connection closed');};
      status.textContent=`Audify → ${device.selectedOptions[0].textContent} (${info.sampleRate} Hz)`;
    }catch(error){if(session===current)detach();throw error;}
  }
  async function change(action){
    if(busy)return;busy=true;mode.disabled=device.disabled=refresh.disabled=apply.disabled=true;
    const run=document.getElementById('runButton');if(run)run.disabled=true;
    try{await action();}
    catch(error){
      detach();void invoke('stop').catch(()=>{});show();
      reportError(error);
    }
    finally{busy=false;mode.disabled=device.disabled=refresh.disabled=apply.disabled=false;if(run)run.disabled=false;}
  }
  mode.addEventListener('change',()=>{show();void change(async()=>{
    await stopRun();detach();await invoke('stop');
    if(mode.value==='audify')await refreshDevices();else status.textContent='WebView output';
  });});
  refresh.addEventListener('click',()=>{void change(async()=>{await stopRun();detach();await invoke('stop');await refreshDevices();});});
  apply.addEventListener('click',()=>{void change(async()=>{await stopRun();detach();await connect();});});
  window.addEventListener('pagehide',()=>{detach();void window.__TAURI_INTERNALS__.invoke('audio_shutdown').catch(()=>{});});
  document.getElementById('runButton')?.addEventListener('click',event=>{if(busy){event.preventDefault();event.stopImmediatePropagation();}},true);
})();
