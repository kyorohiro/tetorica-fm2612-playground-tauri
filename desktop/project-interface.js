// Appended to the pinned playground.js HTTP response by the desktop wrapper.
// Runs in the same module scope, so no duplicate project store is introduced.
{
  let projectId = crypto.randomUUID();
  const replaceProject = virtualFiles.replace.bind(virtualFiles);
  virtualFiles.replace = entries => {
    replaceProject(entries);
    projectId = crypto.randomUUID();
  };
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
  Object.defineProperty(window, '__tetoricaMcpRequest', {value(request) {
    let response;
    try {
      if (!window.__tetoricaMcpEnabled || request.generation !== window.__tetoricaMcpGeneration || Date.now() > request.deadline) throw Error('MCP is disabled or request expired');
      response = {result:handle(request.name, request.arguments)};
    } catch (error) { response = {error:String(error.message || error)}; }
    void window.__TAURI_INTERNALS__.invoke('mcp_reply', {id:request.id, response}).catch(() => {});
  }});
}
