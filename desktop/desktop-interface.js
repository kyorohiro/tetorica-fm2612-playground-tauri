(() => {
  const devOrigin = __DESKTOP_DEV_ORIGIN__;
  const local = location.protocol === 'tauri:' ||
    (['http:', 'https:'].includes(location.protocol) && location.hostname === 'tauri.localhost');
  if (window.top !== window || !(local || (devOrigin && location.origin === devOrigin))) return;

  function mount() {
    const menu = document.getElementById('mainMenu');
    const panel = menu?.querySelector(':scope > .toolbar-menu-panel');
    if (!panel || document.getElementById('desktop-window-top')) return;
    const top = document.createElement('button');
    top.id = 'desktop-window-top';
    top.type = 'button';
    top.setAttribute('aria-pressed', 'false');
    top.textContent = 'Always on Top: Off';
    const status = document.createElement('p');
    status.setAttribute('role', 'status');
    status.hidden = true;
    panel.append(top, status);
    const invoke = (command, args) => window.__TAURI_INTERNALS__.invoke(command, args);
    let busy = false;
    function show(enabled) {
      top.setAttribute('aria-pressed', String(enabled));
      top.textContent = `Always on Top: ${enabled ? 'On' : 'Off'}`;
    }
    async function update(action) {
      if (busy) return;
      busy = true;
      top.disabled = true;
      status.textContent = '';
      status.hidden = true;
      try { show(await action()); }
      catch (error) {
        try { show(await invoke('window_top_get')); } catch {}
        status.textContent = String(error);
        status.hidden = false;
      } finally { busy = false; top.disabled = false; }
    }
    menu.addEventListener('toggle', () => {
      if (menu.open) update(() => invoke('window_top_get'));
    });
    top.addEventListener('click', () => update(() => invoke('window_top_set', {
      enabled: top.getAttribute('aria-pressed') !== 'true',
    })));

    const mcp = document.createElement('button');
    mcp.type = 'button';
    mcp.id = 'desktop-mcp';
    mcp.textContent = 'MCP: Off';
    mcp.setAttribute('aria-pressed', 'false');
    const connection = document.createElement('button');
    connection.type = 'button';
    connection.textContent = 'MCP connection…';
    const dialog = document.createElement('dialog');
    dialog.setAttribute('aria-label', 'MCP connection');
    const explanation = document.createElement('p');
    explanation.textContent = 'MCP edits the open project. It does not run code. Export cassette to save. Keep this token private; it changes whenever MCP is enabled.';
    const config = document.createElement('textarea');
    config.readOnly = true;
    config.rows = 12;
    config.cols = 65;
    config.setAttribute('aria-label', 'MCP client connection configuration');
    const close = document.createElement('button');
    close.type = 'button';
    close.textContent = 'Close';
    close.addEventListener('click', () => dialog.close());
    dialog.append(explanation, config, close);
    document.body.append(dialog);
    panel.append(mcp, connection);
    let mcpBusy = false;
    async function refreshMcp(enabled) {
      if (mcpBusy) return;
      mcpBusy = true;
      mcp.disabled = true;
      try {
        const current = await invoke(enabled === undefined ? 'mcp_status' : 'mcp_set', enabled === undefined ? {} : {enabled});
        // Restore the frontend gate after a page reload as well.
        window.__tetoricaMcpEnabled = current.enabled;
        window.__tetoricaMcpGeneration = current.token;
        mcp.textContent = `MCP: ${current.enabled ? 'On' : 'Off'}`;
        mcp.setAttribute('aria-pressed', String(current.enabled));
        config.value = JSON.stringify({mcpServers:{'tetorica-playground':{
          url:current.url, headers:{Authorization:`Bearer ${current.token}`}
        }}}, null, 2);
      } catch (error) {
        status.textContent = String(error);
        status.hidden = false;
      } finally { mcpBusy = false; mcp.disabled = false; }
    }
    mcp.addEventListener('click', () => refreshMcp(mcp.getAttribute('aria-pressed') !== 'true'));
    connection.addEventListener('click', async () => {
      await refreshMcp();
      dialog.showModal();
      config.focus();
      config.select();
    });
    menu.addEventListener('toggle', () => { if (menu.open) void refreshMcp(); });
    void refreshMcp();

  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount, {once:true});
  else mount();
})();
