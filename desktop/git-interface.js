// Appended to the pinned Playground module. Git uses the shared JavaScript
// engine; Rust supplies only native HTTPS transport and repository import.
{
  const flushTextEditor=saveActiveVirtualFile;
  saveActiveVirtualFile=()=>{
    const file=virtualFiles.get(activeVirtualPath);
    if(file?.type==='text'&&!isSystemVirtualPath(activeVirtualPath)&&!activeVirtualPath.startsWith('/.git/'))flushTextEditor();
  };
  const editorText=getEditorValue;
  getEditorValue=()=>{const value=editorText(),stored=virtualFiles.get(activeVirtualPath)?.data;return typeof stored==='string'&&stored.startsWith('\uFEFF')&&!value.startsWith('\uFEFF')?'\uFEFF'+value:value;};
  const {installPlaygroundGit}=await import('/desktop-git-runtime.js');
  // Rust Result<T, String> rejects IPC with a string. isomorphic-git annotates
  // caught errors (error.caller), which requires a writable Error object.
  const invoke=async(command,args)=>{
    try{return await window.__TAURI_INTERNALS__.invoke(command,args);}
    catch(error){throw new Error(typeof error==='string'?error:error?.message??String(error));}
  };
  function encode(bytes){let value='';for(let i=0;i<bytes.length;i+=32768)value+=String.fromCharCode(...bytes.subarray(i,i+32768));return btoa(value);}
  const decode=data=>Uint8Array.from(atob(data),c=>c.charCodeAt(0));
  async function beginRemote(request){
    const session=await invoke('git_https_begin',{request});
    return {close:()=>invoke('git_https_end',{session}),http:{async request({url,method='GET',headers={},body,signal}){
      signal?.throwIfAborted();const chunks=[];let length=0;
      if(body)for await(const chunk of body){signal?.throwIfAborted();length+=chunk.length;if(length>64*1024*1024)throw Error('Git request exceeds 64 MiB');chunks.push(chunk);}
      const bytes=new Uint8Array(length);let offset=0;for(const chunk of chunks){bytes.set(chunk,offset);offset+=chunk.length;}
      const permitted=Object.fromEntries(Object.entries(headers).filter(([name])=>['content-type','accept','git-protocol'].includes(name.toLowerCase())));
      const result=await invoke('git_https_request',{request:{session,url,method,headers:permitted,body:encode(bytes)}});
      signal?.throwIfAborted();return {...result,method,body:[decode(result.body)]};
    }}};
  }
  async function importFolder(){
    saveActiveVirtualFile();const fingerprint=()=>JSON.stringify({revisions:virtualFiles.checkpoint().revisions,path:activeVirtualPath,value:virtualFiles.get(activeVirtualPath)?.type==='text'?getEditorValue():null});const before=fingerprint();
    const result=await invoke('git_request',{request:{action:'open'}});
    if(fingerprint()!==before)throw Error('Files changed while opening; current files were kept.');
    await runtime.stopWithFade();
    if(fingerprint()!==before)throw Error('Files changed while stopping audio; current files were kept.');
    const files=[...result.snapshot.files,...(result.gitSnapshot?.files??[])];
    restoreVirtualFilesFromCassette({files:new Map(files.map(file=>[file.path.slice(1),decode(file.data)])),directories:[...result.snapshot.directories,...(result.gitSnapshot?.directories??[])]});
    for(const file of result.gitSnapshot?.files??[])virtualFiles.writeBinary(file.path,decode(file.data));
    editorAdapter.syncVirtualFiles?.(virtualFiles.list());renderVirtualFileExplorer();renderRunFileOptions();desktopAutosave?.changed();
  }
  installPlaygroundGit({fs:virtualFiles,shell:playgroundShell.shell,lifecycle:playgroundShell,beginRemote,importFolder,beforeCommand:saveActiveVirtualFile,afterCommand(){
    editorAdapter.syncVirtualFiles?.(virtualFiles.list());renderVirtualFileExplorer();renderRunFileOptions();desktopAutosave?.changed();
  }});
}
