// Appended to the pinned playground.js HTTP response by the desktop wrapper.
// Runs in the same module scope, so no duplicate project store is introduced.
{
  let projectId = crypto.randomUUID();
  const invalidateProject = playgroundShell.invalidate.bind(playgroundShell);
  playgroundShell.invalidate = (...args) => {
    projectId = crypto.randomUUID();
    return invalidateProject(...args);
  };
  function validateRequest(request) {
    if (!window.__tetoricaMcpEnabled || request.generation !== window.__tetoricaMcpGeneration || Date.now() >= request.deadline) throw Error('MCP is disabled or request expired');
  }
  async function executeShell(args, request) {
    if (args.projectId !== projectId) throw Error('Project changed: list/read files again');
    const command=args.command;
    if (!(typeof command === 'string' && command.trim() || Array.isArray(command) && command.length > 0 && command.length <= 128 && command.every(v=>typeof v === 'string') && command[0])) throw Error('command must be nonempty shell text or an argument array');
    if (new TextEncoder().encode(JSON.stringify(command)).length > 16384) throw Error('command exceeds 16 KiB');
    if (args.stdin !== undefined && (typeof args.stdin !== 'string' || new TextEncoder().encode(args.stdin).length > 1024 * 1024)) throw Error('stdin must be text, at most 1 MiB');
    const timeout=args.timeoutMs ?? 10000;
    if (!Number.isInteger(timeout) || timeout < 100 || timeout > 30000) throw Error('timeoutMs must be 100..30000');
    const controller=new AbortController(),initialProject=projectId;
    const guard=()=>{if(controller.signal.aborted)throw Error('Shell request cancelled');validateRequest(request);if(initialProject!==projectId)throw Error('Project changed during shell execution');};
    let reason='Shell command timed out',resolveCancelled;
    const cancelled=new Promise(resolve=>{resolveCancelled=resolve;});
    const abort=message=>{reason=message;controller.abort();resolveCancelled({code:130,stdout:'',stderr:reason});};
    const timer=setTimeout(()=>abort('Shell command timed out; no automatic retry'),Math.max(0,Math.min(timeout,request.deadline-Date.now())));
    const watch=setInterval(()=>{try{guard();}catch(error){abort(error.message);}},50);
    try {
      guard();
      const result=await Promise.race([playgroundShell.execute(command,{signal:controller.signal,stdin:args.stdin??'',beforeExecute:guard}),cancelled]);
      if (initialProject!==projectId) throw Error('Project changed during shell execution');
      const stdout=String(result.stdout??''),stderr=String(result.stderr??'');
      return {projectId,code:result.code,stdout:stdout.slice(0,32000),stderr:stderr.slice(0,8000),...(result.stages?{stages:result.stages}:{}),truncated:{stdout:stdout.length>32000,stderr:stderr.length>8000}};
    } finally {clearTimeout(timer);clearInterval(watch);}
  }
  const textAt = path => path === activeVirtualPath ? getEditorValue() : virtualFiles.get(path)?.data;
  const pathOf = path => {
    if (typeof path !== 'string' || !path.startsWith('/') || path.includes('\\') ||
        path.split('/').some(part => part === '..' || part === '.') || path.includes('//') || path.endsWith('/')) {
      throw Error('Use an absolute project file path such as /index.js');
    }
    if (path === '/sys' || path.startsWith('/sys/')) throw Error('/sys is read-only; use MCP resources for API references');
    return path;
  };
  const handle = (name, args = {}) => {
    if (name === 'list_files') return {projectId, files: virtualFiles.list()
      .filter(file => !isSystemVirtualPath(file.path))
      .map(file => ({path:file.path, type:file.type, size:file.type === 'text' ? new TextEncoder().encode(textAt(file.path)).length : file.data.byteLength}))};
    const path = pathOf(args.path);
    const file = virtualFiles.get(path);
    if (file && file.type !== 'text') throw Error('Only text files can be read or edited through this MCP tool');
    if (name === 'read_file') {
      if (!file) throw Error('File not found');
      return {projectId, path, content:textAt(path)};
    }
    if (name !== 'write_file') throw Error('Unknown project operation');
    if (args.projectId !== projectId) throw Error('Project changed: list/read files again');
    if (typeof args.content !== 'string' || new TextEncoder().encode(args.content).length > 1024 * 1024) throw Error('content must be text, at most 1 MiB');
    if (!Object.hasOwn(args, 'expectedContent') || (file ? args.expectedContent !== textAt(path) : args.expectedContent !== null)) {
      throw Error('File changed since read: read it again before editing. Use expectedContent:null only for new files.');
    }
    if (virtualFiles.list().some(f => f.path !== path && (f.path.startsWith(path + '/') || path.startsWith(f.path + '/')))) throw Error('Path conflicts with an existing file');
    virtualFiles.writeText(path, args.content);
    if (path === activeVirtualPath) editorAdapter.setValue(args.content);
    editorAdapter.syncVirtualFiles?.(virtualFiles.list());
    renderVirtualFileExplorer();
    renderRunFileOptions();
    setStatus(`MCP updated ${path}.`);
    return {projectId, path, updated:true};
  };
  Object.defineProperty(window, '__tetoricaMcpRequest', {value:async function(request) {
    let response;
    try {
      validateRequest(request);
      response = {result:request.name==='shell_execute'?await executeShell(request.arguments??{},request):handle(request.name, request.arguments)};
    } catch (error) { response = {error:String(error.message || error)}; }
    void window.__TAURI_INTERNALS__.invoke('mcp_reply', {id:request.id, response}).catch(() => {});
  }});
}
