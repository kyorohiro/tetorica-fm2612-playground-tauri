import {createServer} from 'node:http';
import {createInterface} from 'node:readline';
import {randomBytes} from 'node:crypto';
import {WebSocketServer,WebSocket} from 'ws';
import {createOutput} from './output_audify.mjs';

let output=null,server=null,wss=null,socket=null;
let receivedFrames=0,peak=0;
let lastError=null;
let deviceAudio=null;
const LIMIT=4;
async function stop(){
  const old=output;output=null;
  if(socket){socket.terminate();socket=null;}
  old?.close();
  if(wss){wss.close();wss=null;}
  if(server){await new Promise(resolve=>server.close(resolve));server=null;}
}
async function devices(){
  const loaded=await import('audify');const {RtAudio}=loaded.default??loaded;
  deviceAudio??=new RtAudio();return deviceAudio.getDevices().filter(device=>device.outputChannels>=2)
    .map(device=>({id:device.id,name:device.name,channels:device.outputChannels,
      preferredSampleRate:device.preferredSampleRate,isDefault:device.isDefaultOutput}));
}
async function start({deviceId,sampleRate}){
  await stop();
  receivedFrames=0;peak=0;lastError=null;
  if(!Number.isInteger(sampleRate)||sampleRate<8000||sampleRate>192000)throw Error('Invalid sample rate');
  if(deviceId!==null&&deviceId!==undefined&&(!Number.isInteger(deviceId)||deviceId<0))throw Error('Invalid device');
  const selected=(await devices()).find(device=>deviceId==null?device.isDefault:device.id===deviceId);
  if(!selected)throw Error('Output device is no longer available; refresh devices');
  const inputSampleRate=sampleRate;sampleRate=selected.preferredSampleRate||sampleRate;
  let credits=0,started=false;
  function grant(){
    if(!output||socket?.readyState!==WebSocket.OPEN)return;
    const available=LIMIT-Math.ceil(output.queuedFrames/output.frames)-credits;
    if(available>0){credits+=available;socket.send(JSON.stringify({type:'credit',count:available}));}
  }
  output=await createOutput({sampleRate,bufferFrames:512,deviceId:deviceId??undefined,
    onDrain:grant,onError:error=>{lastError=error.message;if(socket?.readyState===WebSocket.OPEN)socket.send(JSON.stringify({type:'error',error:error.message}));void stop();}});
  const frames=output.frames,token=randomBytes(32).toString('hex');
  server=createServer((_request,response)=>{response.writeHead(404);response.end();});
  wss=new WebSocketServer({noServer:true,maxPayload:frames*8,perMessageDeflate:false});
  server.on('upgrade',(request,connection,head)=>{
    if(request.url!==`/${token}`||socket){connection.destroy();return;}
    wss.handleUpgrade(request,connection,head,ws=>wss.emit('connection',ws));
  });
  wss.on('connection',ws=>{
    socket=ws;grant();
    ws.on('message',(bytes,binary)=>{
      try{
        if(!binary||bytes.length!==frames*8||credits<1||!output)throw Error('Invalid audio packet');
        credits--;
        const left=new Float32Array(frames),right=new Float32Array(frames);
        for(let i=0;i<frames;i++){left[i]=bytes.readFloatLE(i*8);right[i]=bytes.readFloatLE(i*8+4);}
        output.write({left,right});
        receivedFrames+=frames;for(const sample of left)peak=Math.max(peak,Math.abs(sample));for(const sample of right)peak=Math.max(peak,Math.abs(sample));
        if(!started&&output.queuedFrames>=frames*2){started=true;output.start();}
      }catch(error){lastError=error.message;ws.send(JSON.stringify({type:'error',error:error.message}));void stop();}
    });
    ws.on('close',()=>{if(socket===ws)void stop();});
    ws.on('error',error=>{lastError=error.message;if(socket===ws)void stop();});
  });
  await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(0,'127.0.0.1',resolve);});
  return {url:`ws://127.0.0.1:${server.address().port}/${token}`,frames,sampleRate,inputSampleRate};
}
const input=createInterface({input:process.stdin});let queue=Promise.resolve();
input.on('line',line=>{queue=queue.then(async()=>{
  let request;
  try{
    request=JSON.parse(line);
    if(request.op==='status'&&request.args?.reset){receivedFrames=0;peak=0;}
    const result=request.op==='devices'?await devices():request.op==='start'?await start(request.args??{}):request.op==='stop'?await stop():request.op==='status'?{active:!!output,receivedFrames,peak,lastError,output:output?.getState()??null}:null;
    if(!['devices','start','stop','status'].includes(request.op))throw Error('Unknown audio command');
    process.stdout.write(JSON.stringify({id:request.id,result:result??null})+'\n');
  }catch(error){process.stdout.write(JSON.stringify({id:request?.id,error:error.message})+'\n');}
});});
input.on('close',()=>{void queue.finally(async()=>{await stop();process.exit();});});
process.on('SIGTERM',()=>{void stop().finally(()=>process.exit());});
