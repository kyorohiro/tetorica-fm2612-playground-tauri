// Appended to the pinned Playground module; all file editing uses its existing store.
{
  // The editor buffer can still contain the previous code while a binary/TFI
  // preview is selected. Never flush that buffer into the selected binary file.
  const flushTextEditor = saveActiveVirtualFile;
  saveActiveVirtualFile = () => {
    const file = virtualFiles.get(activeVirtualPath);
    if (file?.type === 'text' && !isSystemVirtualPath(activeVirtualPath)) flushTextEditor();
  };
  const editorText = getEditorValue;
  getEditorValue = () => {
    const value = editorText(), stored = virtualFiles.get(activeVirtualPath)?.data;
    return typeof stored === 'string' && stored.startsWith('\uFEFF') && !value.startsWith('\uFEFF') ? '\uFEFF' + value : value;
  };
  let project = null, epoch = 0, busy = false, installing = false;
  const invoke = request => window.__TAURI_INTERNALS__.invoke('git_request', {request});
  const invalidate = playgroundShell.invalidate.bind(playgroundShell);
  playgroundShell.invalidate = (...args) => {
    if (busy && !installing) void window.__TAURI_INTERNALS__.invoke('git_cancel').catch(()=>{});
    epoch++;
    if (!installing) project = null;
    return invalidate(...args);
  };
  const panel = document.querySelector('#mainMenu > .toolbar-menu-panel');
  const button = document.createElement('button');button.type='button';button.id='desktop-git';button.textContent='Git…';
  const dialog = document.createElement('dialog');dialog.id='desktop-git-dialog';dialog.setAttribute('aria-label','Project Git');
  dialog.style.cssText='width:min(760px,calc(100vw - 32px));max-height:calc(100dvh - 32px);overflow:auto';
  dialog.innerHTML=`<h2>Project Git</h2><p id="gitFolder">Open a project folder to use Git.</p>
    <button data-git="create">Save current project and start Git…</button>
    <p class="subtext">Choose or create an empty folder. Your current editor files are saved there and kept open. Git is initialized automatically.</p>
    <div class="git-actions"><button data-git="open">Open folder…</button><button data-git="save">Save files</button><button data-git="reload">Reload files</button><button data-git="init">Initialize Git</button><button data-git="status">Changes</button><button data-git="diff">Diff</button><button data-git="log">History</button></div>
    <div id="gitChanges"></div>
    <label>Commit message <input id="gitMessage" type="text" autocomplete="off"></label>
    <div><label>Author <input id="gitAuthor" type="text" autocomplete="off"></label> <label>Email <input id="gitEmail" type="email" autocomplete="off"></label></div>
    <button data-git="commit">Stage selected and commit</button>
    <p class="subtext">The commit includes files already staged. Review them with <code>git diff --cached</code>.</p>
    <hr><label>SSH repository URL <input id="gitRemote" type="text" placeholder="git@github.com:owner/repository.git" autocomplete="off"></label>
    <div class="git-actions"><button data-git="remote">Set origin…</button><button data-git="clone">Clone into empty folder…</button><button data-git="fetch">Fetch</button><button data-git="pull">Pull (fast-forward)</button><button data-git="push">Push…</button></div>
    <div class="git-actions"><button data-git="generate-key">Generate SSH key…</button><button data-git="keys">Show public key</button></div>
    <p class="subtext">Only Playground-generated keys are used. Passphrases are entered in a native window. Pull requires saved, committed changes; automatic merge is not available.</p>
    <pre id="gitOutput" role="status" style="max-height:240px;overflow:auto;white-space:pre-wrap"></pre><button id="gitCancel" hidden>Request cancel</button><button id="gitClose">Close</button>`;
  document.body.append(dialog);panel?.append(button);
  const style=document.createElement('style');
  style.textContent=`#desktop-git-dialog{border:1px solid var(--line);border-radius:12px;padding:20px;background:var(--panel);color:var(--ink)}
    #desktop-git-dialog h2{margin:0 0 8px} #desktop-git-dialog>label{display:block;margin:10px 0}
    #desktop-git-dialog>label input{display:block;width:100%;box-sizing:border-box;margin-top:4px;padding:6px}
    #desktop-git-dialog .git-actions{display:flex;flex-wrap:wrap;gap:6px;margin:8px 0}
    #gitChanges{max-height:140px;overflow:auto} #gitChanges label{padding:3px 0}
    #gitOutput{min-height:60px;margin:12px 0;padding:12px} #gitFolder{overflow-wrap:anywhere}`;
  document.head.append(style);
  const field=id=>dialog.querySelector('#'+id);
  const output=text=>{field('gitOutput').textContent=String(text);};
  function sync(){field('gitFolder').textContent=project?project.folder:'Open a project folder to use Git.';}
  function encode(bytes){let binary='';for(let i=0;i<bytes.length;i+=32768)binary+=String.fromCharCode(...bytes.subarray(i,i+32768));return btoa(binary);}
  function capture(){
    const encoder=new TextEncoder();
    return {files:virtualFiles.list().filter(f=>!isSystemVirtualPath(f.path)).map(f=>({path:f.path,data:encode(typeof f.data==='string'?encoder.encode(f.path===activeVirtualPath?getEditorValue():f.data):f.data)})),
      directories:virtualFiles.listDirectories().filter(p=>p!=='/sys'&&!isSystemVirtualPath(p))};
  }
  function fingerprint(){return JSON.stringify(capture());}
  async function request(action,args={}){
    const previousEpoch=epoch, bound=project;
    if(!['open','clone','create','keys','generate-key'].includes(action)&&!bound)throw Error('Save your current project first: run git start, or use Menu → Git… → Save current project and start Git…. Choose an empty folder, then retry this command.');
    const result=await invoke({action,...(bound?{projectId:bound.projectId,revision:bound.revision}:{}),...args});
    if(epoch!==previousEpoch)throw Error('Project changed during Git operation; reopen the folder before continuing.');
    // A reload's revision becomes valid only after its files are installed.
    // If the editor changed during the request, keep the stale revision so a
    // subsequent Save cannot overwrite the unseen disk changes.
    if(bound&&!result.snapshot&&result.projectId===bound.projectId&&Number.isInteger(result.revision))bound.revision=result.revision;
    return result;
  }
  function decode(file){
    const bytes=Uint8Array.from(atob(file.data),c=>c.charCodeAt(0));let data=bytes;
    if(!/\.(tfi|bin|vgm|vgz|wav|flac|ogg|mp3|png|jpg|zip|wasm)$/i.test(file.path)&&!bytes.includes(0)){
      try{data=new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(bytes);}catch{}
    }
    return {path:file.path,data};
  }
  async function install(result,expected){
    const installEpoch=epoch;
    if(fingerprint()!==expected)throw Error('Files were edited while loading. They were kept; reopen the folder to continue.');
    await runtime.stopWithFade();
    if(epoch!==installEpoch||fingerprint()!==expected)throw Error('Files changed while stopping audio. Reopen the folder to continue.');
    const files=result.snapshot.files.map(decode);
    if(!files.some(f=>f.path==='/index.js'))files.push({path:'/index.js',data:'// Playground project\n'});
    const system=virtualFiles.list().filter(f=>isSystemVirtualPath(f.path));
    installing=true;
    try{playgroundShell.invalidate('Git project loaded');virtualFiles.replace([...system,...files],result.snapshot.directories);}
    finally{installing=false;}
    project={projectId:result.projectId,revision:result.revision,folder:result.folder};
    activeVirtualPath='/index.js';runVirtualPath='/index.js';showVirtualFile(virtualFiles.get('/index.js'));
    editorAdapter.syncVirtualFiles?.(virtualFiles.list());renderVirtualFileExplorer();renderRunFileOptions();desktopAutosave?.changed();sync();
  }
  async function save(){saveActiveVirtualFile();return request('save',{snapshot:capture()});}
  let changes=[];
  function labels(change){return [change.conflict?'conflict':'',change.staged?'staged':'',change.untracked?'untracked':'',change.worktree?'unstaged':''].filter(Boolean).join(', ')||'changed';}
  function statusText(result){
    if(!result.initialized)return 'This folder is not a Git repository. Run git init.\n';
    const files=result.files??[];
    return `${result.branch??'No commits yet'}\n`+(files.length?files.map(f=>`[${labels(f)}] ${f.path}`).join('\n')+'\n':'Working tree clean.\n');
  }
  function showChanges(result){
    changes=result.files??[];const root=field('gitChanges');root.replaceChildren();
    if(!result.initialized){output('This folder is not a Git repository. Select Initialize Git.');return;}
    for(const change of changes){if(!change.path)continue;const row=document.createElement('label');row.style.cssText='display:block;overflow-wrap:anywhere';
      const checkbox=document.createElement('input');checkbox.type='checkbox';checkbox.value='/'+change.path;checkbox.checked=true;
      row.append(checkbox,document.createTextNode(` [${labels(change)}] ${change.path}`));root.append(row);}
    output(`${result.branch??'New repository'} · ${changes.length} changed file(s)`);
  }
  async function execute(action,args={}){
    if(action==='status'||action==='diff'){await save();return request(action,args);}
    if(action==='create'){
      saveActiveVirtualFile();
      const result=await request('create',{snapshot:capture()});
      project={projectId:result.projectId,revision:result.revision,folder:result.folder};
      sync();return result;
    }
    if(action==='save')return save();
    if(['open','clone','reload','pull'].includes(action)){
      if(!window.confirm('Replace the current editor files with the folder contents? Save or export changes you want to keep first.'))throw Error('Cancelled');
      const expected=fingerprint();
      const result=await request(action,args);await install(result,expected);return {loaded:project.folder};
    }
    if(action==='commit'){
      const paths=[...field('gitChanges').querySelectorAll('input:checked')].map(input=>input.value);
      if(!paths.length)throw Error('Choose files from Changes first.');
      await save();await request('stage',{paths});
      return request('commit',{message:field('gitMessage').value,name:field('gitAuthor').value,email:field('gitEmail').value,...args});
    }
    return request(action,args);
  }
  async function run(action,args={}){
    if(busy)throw Error('A Git operation is already running.');busy=true;
    for(const el of dialog.querySelectorAll('button[data-git]'))el.disabled=true;
    output('Working…');
    field('gitCancel').hidden=false;
    try{
      const result=await execute(action,args);
      if(action==='create'){showChanges(result.status);output(`Saved current project to ${result.folder}. Git is ready.\nNext: git add index.js`);}
      else if(action==='commit'){showChanges(await request('status'));output(`Committed ${result.commit}`);}
      else if(action==='status'||action==='init')showChanges(result);
      else if(action==='diff')output((result.text||'No unstaged changes. Use git diff --cached to review staged changes.\n')+(result.truncated?'\n[Diff truncated]':''));
      else if(action==='keys'||action==='generate-key')output(result.generated?`${result.fingerprint}\n\n${result.publicKey}`:'No Playground SSH key. Select Generate SSH key.');
      else output(JSON.stringify(result,null,2));
      sync();return result;
    }catch(error){output(error.message??String(error));throw error;}
    finally{busy=false;field('gitCancel').hidden=true;for(const el of dialog.querySelectorAll('button[data-git]'))el.disabled=false;}
  }
  dialog.querySelector('#gitClose').addEventListener('click',()=>dialog.close());
  field('gitCancel').addEventListener('click',()=>{
    void window.__TAURI_INTERNALS__.invoke('git_cancel').then(()=>output('Cancellation requested. Waiting for the operation to finish…'),error=>output(String(error)));
  });
  button.addEventListener('click',()=>{document.getElementById('mainMenu').open=false;sync();dialog.showModal();});
  dialog.addEventListener('click',event=>{
    const action=event.target.closest('button[data-git]')?.dataset.git;if(!action)return;
    const args=['remote','clone'].includes(action)?{url:field('gitRemote').value}:{};
    void run(action,args).catch(()=>{});
  });
  playgroundShell.shell.register('git',async context=>{
    context.assertActive();
    const cancel=()=>{void window.__TAURI_INTERNALS__.invoke('git_cancel').catch(()=>{});};
    context.signal?.addEventListener('abort',cancel,{once:true});
    try {
    const [command,...args]=context.args;
    if(!command||command==='help')return 'git start | init | status | diff [--cached] | add PATH... | commit -m MESSAGE | log | fetch | pull | push\nStart with git start to save current files into an empty folder and initialize Git. Configure SSH using Menu → Git.\n';
    let result;
    if(command==='start'&&!args.length){
      result=await run('create');
      return `Saved current project to ${result.folder}. Git is ready.\nNext: git add index.js\n`;
    }else if(command==='add'){
      if(!args.length)throw Error('Usage: git add PATH...');
      await save();
      context.assertActive();
      const status=await request('status');
      const paths=args.length===1&&args[0]==='.'?(status.files??[]).map(f=>'/'+f.path):args.map(p=>context.resolve(p));
      result=await request('stage',{paths});
    }else if(command==='commit'){
      if(args.length!==2||args[0]!=='-m')throw Error('Usage: git commit -m MESSAGE');
      await save();context.assertActive();result=await request('commit',{message:args[1],name:field('gitAuthor').value,email:field('gitEmail').value});
    }else if(command==='diff'){
      if(args.length>1||args.length&&args[0]!=='--cached')throw Error('Usage: git diff [--cached]');
      await save();context.assertActive();
      const staged=args[0]==='--cached';
      result=await request('diff',{staged});return (result.text||(staged?'No staged changes.':'No unstaged changes. Use git diff --cached to review staged changes.'))+'\n';
    }else if(['init','status','log','fetch','pull','push'].includes(command)&&!args.length){
      if(command==='pull')result=await run('pull');else result=await run(command);
    }else throw Error('Unsupported Git command. Type git help.');
    return command==='status'?statusText(result):JSON.stringify(result,null,2)+'\n';
    } finally {context.signal?.removeEventListener('abort',cancel);}
  });
}
