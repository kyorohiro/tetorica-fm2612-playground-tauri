const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const {randomUUID} = require('node:crypto');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname,'../desktop/project-interface.js'),'utf8');
function boot() {
  const files = new Map([['/index.js',{path:'/index.js',type:'text',data:'saved'}],['/sound.bin',{path:'/sound.bin',type:'binary',data:new Uint8Array(2)}]]);
  let editor = 'human edit', response;
  const virtualFiles = {get:p=>files.get(p),list:()=>[...files.values()],writeText(p,data){files.set(p,{path:p,type:'text',data})},replace(entries){files.clear();for(const file of entries)files.set(file.path,file)}};
  const window = {__tetoricaMcpEnabled:true,__tetoricaMcpGeneration:'current',__TAURI_INTERNALS__:{invoke:async(cmd,args)=>{response=args.response}}};
  const playgroundShell={invalidate(){}};
  const context = {window,virtualFiles,playgroundShell,crypto:{randomUUID},activeVirtualPath:'/index.js',getEditorValue:()=>editor,
    editorAdapter:{setValue:v=>editor=v,syncVirtualFiles(){}},renderVirtualFileExplorer(){},renderRunFileOptions(){},setStatus(){},
    isSystemVirtualPath:p=>p.startsWith('/sys/'),TextEncoder,AbortController,setTimeout,clearTimeout,setInterval,clearInterval};
  vm.runInNewContext(source,context);
  return {files,window,virtualFiles,playgroundShell,async callAsync(name,args={},deadline=Date.now()+1000){await window.__tetoricaMcpRequest({id:'async',name,arguments:args,deadline,generation:'current'});return response},edit:v=>editor=v,editor:()=>editor,call(name,args={},deadline=Date.now()+1000){window.__tetoricaMcpRequest({id:'test',name,arguments:args,deadline,generation:'current'});return response}};
}
test('reads live editor and writes with exact-content concurrency control',()=>{
  const app=boot();const read=app.call('read_file',{path:'/index.js'}).result;
  assert.equal(read.content,'human edit');
  app.edit('new human edit');
  const args={path:read.path,projectId:read.projectId,expectedContent:read.content,content:'AI edit'};
  assert.match(app.call('write_file',args).error,/changed since read/);
  assert.equal(app.editor(),'new human edit');
  args.expectedContent='new human edit';
  assert.equal(app.call('write_file',args).result.updated,true);
  assert.equal(app.editor(),'AI edit');assert.equal(app.files.get('/index.js').data,'AI edit');
});
test('creates text files, rejects stale projects, binary edits, reserved paths and collisions',()=>{
  const app=boot();const projectId=app.call('list_files').result.projectId;
  const args={projectId,path:'/lib/new.js',content:'new',expectedContent:null};
  assert.equal(app.call('write_file',args).result.updated,true);
  for(const path of ['/sys/a.js','/../bad.js','relative.js','/lib','/sound.bin','/lib/new.js/child']) {
    assert.ok(app.call('write_file',{...args,path}).error,path);
  }
  app.playgroundShell.invalidate('New Cassette');app.virtualFiles.replace([{path:'/index.js',type:'text',data:'saved'}]);
  assert.match(app.call('write_file',{...args,path:'/other.js'}).error,/Project changed/);
});
test('disabled or expired requests never mutate files',()=>{
  const app=boot();const projectId=app.call('list_files').result.projectId;
  const args={projectId,path:'/new.js',content:'bad',expectedContent:null};
  assert.match(app.call('write_file',args,0).error,/expired/);
  app.window.__tetoricaMcpEnabled=false;
  assert.match(app.call('write_file',args).error,/disabled/);
  assert.equal(app.files.has('/new.js'),false);
});
test('shell execution validates inputs, keeps project identity for atomic replacements, and returns bounded output',async()=>{
 const app=boot(),projectId=app.call('list_files').result.projectId;let called=0;
 app.playgroundShell.execute=async(command,options)=>{called++;options.beforeExecute();app.virtualFiles.replace([{path:'/index.js',type:'text',data:'patched'}]);return {code:0,stdout:'x'.repeat(33000),stderr:'warn'};};
 const reply=await app.callAsync('shell_execute',{projectId,command:['echo','hello'],stdin:'input'});assert.equal(reply.result.code,0);assert.equal(reply.result.projectId,projectId);assert.equal(reply.result.stdout.length,32000);assert(reply.result.truncated.stdout);assert.equal(called,1);
 for(const args of [{projectId:'stale',command:'pwd'},{projectId,command:''},{projectId,command:['echo',1]},{projectId,command:'pwd',timeoutMs:99},{projectId,command:'pwd',stdin:1}])assert((await app.callAsync('shell_execute',args)).error);
 assert.equal(called,1);
});
test('shell timeout and MCP disable abort only the associated request; queued work checks freshness',async()=>{
 const app=boot(),projectId=app.call('list_files').result.projectId;let signal,guard;
 app.playgroundShell.execute=(command,options)=>{signal=options.signal;guard=options.beforeExecute;return new Promise(()=>{});};
 const timeout=await app.callAsync('shell_execute',{projectId,command:'js /hang.js',timeoutMs:100});assert.equal(timeout.result.code,130);assert(signal.aborted);assert.throws(()=>guard(),/cancelled/);
 const pending=app.callAsync('shell_execute',{projectId,command:'pwd'});app.window.__tetoricaMcpEnabled=false;const off=await pending;assert.equal(off.result.code,130);assert(signal.aborted);
});
test('project changes invalidate outstanding shell work',async()=>{
 const app=boot(),projectId=app.call('list_files').result.projectId;let signal;
 app.playgroundShell.execute=(command,options)=>{signal=options.signal;return new Promise(()=>{});};
 const pending=app.callAsync('shell_execute',{projectId,command:'pwd'});app.playgroundShell.invalidate('New Cassette');const reply=await pending;assert.match(reply.error,/Project changed/);assert(signal.aborted);
});
