(() => {
  const devOrigin = __DESKTOP_DEV_ORIGIN__;
  const local = location.protocol === 'tauri:' ||
    (['http:', 'https:'].includes(location.protocol) && location.hostname === 'tauri.localhost');
  if (window.top !== window || !(local || (devOrigin && location.origin === devOrigin))) return;

  function confirmAction(message) {
    return new Promise(resolve => {
      const previousFocus = document.activeElement;
      const dialog = document.createElement('dialog');
      dialog.setAttribute('aria-label', 'Confirm action');
      const text = document.createElement('p');
      text.textContent = message;
      const cancel = document.createElement('button');
      cancel.type = 'button';
      cancel.textContent = 'Cancel';
      const proceed = document.createElement('button');
      proceed.type = 'button';
      proceed.textContent = 'Continue';
      let accepted = false;
      cancel.addEventListener('click', () => dialog.close());
      proceed.addEventListener('click', () => { accepted = true; dialog.close(); });
      dialog.addEventListener('close', () => {
        dialog.remove();
        previousFocus?.focus?.();
        resolve(accepted);
      }, {once:true});
      dialog.append(text, cancel, proceed);
      document.body.append(dialog);
      dialog.showModal();
      cancel.focus();
    });
  }

  function mount() {
    const menu = document.getElementById('mainMenu');
    const panel = menu?.querySelector(':scope > .toolbar-menu-panel');
    if (!panel || document.getElementById('desktop-window-top')) return;
    const settings = document.getElementById('desktopSettingsMenu') ?? panel;
    const pinSlot = document.getElementById('desktopPinSlot');
    const top = document.createElement('button');
    top.id = 'desktop-window-top';
    top.type = 'button';
    top.setAttribute('aria-pressed', 'false');
    top.textContent = pinSlot ? 'Pin' : 'Always on Top: Off';
    const status = document.createElement('p');
    status.setAttribute('role', 'status');
    status.hidden = true;
    (pinSlot ?? panel).append(top);
    panel.append(status);
    const invoke = (command, args) => window.__TAURI_INTERNALS__.invoke(command, args);
    const resetCache = document.getElementById('resetOfflineCacheButton');
    resetCache?.addEventListener('click', async (event) => {
      // Own this action in Tauri, including releases without sw-register.js.
      event.preventDefault();
      event.stopImmediatePropagation();
      if (resetCache.disabled) return;
      resetCache.disabled = true;
      if (!await confirmAction('Clear the offline cache and reload?')) { resetCache.disabled = false; return; }
      resetCache.disabled = true;
      status.hidden = true;
      try {
        await window.__tetoricaAutosaveFlush?.();
        if (window.caches) {
          const keys = await window.caches.keys();
          await Promise.all(keys.filter(key => key.startsWith('hello-ymfm-docs-'))
            .map(key => window.caches.delete(key)));
        }
        const registrations = await window.navigator?.serviceWorker?.getRegistrations?.() ?? [];
        await Promise.all(registrations.map(registration => registration.unregister()));
        await invoke('window_reload');
      } catch (error) {
        status.textContent = String(error);
        status.hidden = false;
        resetCache.disabled = false;
      }
    }, {capture:true});
    let busy = false;
    function show(enabled) {
      top.setAttribute('aria-pressed', String(enabled));
      top.textContent = pinSlot ? (enabled ? 'Unpin' : 'Pin') : `Always on Top: ${enabled ? 'On' : 'Off'}`;
      top.title = enabled ? 'Unpin window' : 'Keep window on top';
      top.setAttribute('aria-label', top.title);
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
    if (pinSlot) void update(() => invoke('window_top_get'));

    const mcp = document.createElement('button');
    mcp.type = 'button';
    mcp.id = 'desktop-mcp';
    mcp.textContent = 'MCP: Off';
    mcp.setAttribute('aria-pressed', 'false');
    const connection = document.createElement('button');
    connection.type = 'button';
    connection.id = 'desktop-mcp-connection';
    connection.className = 'menu-setting-row';
    const connectionLabel = document.createElement('span');
    connectionLabel.textContent = 'MCP connection…';
    const connectionValue = document.createElement('small');
    connectionValue.id = 'desktop-mcp-value';
    connectionValue.className = 'menu-value';
    connectionValue.textContent = 'Off';
    connection.append(connectionLabel, connectionValue);
    connection.setAttribute('aria-haspopup', 'dialog');
    const regenerate = document.createElement('button');
    regenerate.id = 'desktop-mcp-regenerate';
    regenerate.type = 'button';
    regenerate.textContent = 'Regenerate MCP token';
    regenerate.disabled = true;
    const dialog = document.createElement('dialog');
    dialog.id = 'desktop-mcp-dialog';
    dialog.className = 'playground-settings-dialog';
    dialog.setAttribute('aria-label', 'MCP connection');
    Object.assign(dialog.style, {width:'min(42rem, calc(100vw - 2rem))', boxSizing:'border-box'});
    const heading = document.createElement('h2');
    heading.textContent = 'MCP connection';
    const explanation = document.createElement('p');
    explanation.textContent = 'MCP edits the open project. Turn MCP on before copying this configuration. If the saved port is occupied, a free local port is selected automatically; update your client URL when it changes. Keep the token private. It is retained across restarts.';
    const tokenHint = document.createElement('p');
    tokenHint.textContent = 'To replace the token, turn MCP off and regenerate it. Existing clients will need the new configuration.';
    const config = document.createElement('textarea');
    config.readOnly = true;
    config.rows = 12;
    config.cols = 65;
    Object.assign(config.style, {width:'100%', boxSizing:'border-box', resize:'vertical'});
    config.setAttribute('aria-label', 'MCP client connection configuration');
    const mcpStatus = document.createElement('p');
    mcpStatus.id = 'desktop-mcp-status';
    mcpStatus.setAttribute('role', 'status');
    mcpStatus.hidden = true;
    const close = document.createElement('button');
    close.type = 'button';
    close.textContent = 'Close';
    close.addEventListener('click', () => dialog.close());
    dialog.addEventListener('close', () => menu.querySelector(':scope > summary')?.focus());
    dialog.append(heading, mcp, explanation, config, tokenHint, regenerate, mcpStatus, close);
    document.body.append(dialog);
    settings.append(connection);
    let mcpBusy = false;
    async function refreshMcp(enabled, rotate = false) {
      if (mcpBusy) return;
      mcpBusy = true;
      mcp.disabled = true;
      regenerate.disabled = true;
      mcpStatus.hidden = true;
      try {
        const current = await invoke(rotate ? 'mcp_regenerate_token' : enabled === undefined ? 'mcp_status' : 'mcp_set', enabled === undefined ? {} : {enabled});
        // Restore the frontend gate after a page reload as well.
        window.__tetoricaMcpEnabled = current.enabled;
        window.__tetoricaMcpGeneration = current.generation;
        mcp.textContent = `MCP: ${current.enabled ? 'On' : 'Off'}`;
        connectionValue.textContent = current.enabled ? 'On' : 'Off';
        mcp.setAttribute('aria-pressed', String(current.enabled));
        regenerate.disabled = current.enabled;
        config.value = JSON.stringify({mcpServers:{'tetorica-playground':{
          url:current.url, headers:{Authorization:`Bearer ${current.token}`}
        }}}, null, 2);
      } catch (error) {
        mcpStatus.textContent = String(error);
        mcpStatus.hidden = false;
      } finally { mcpBusy = false; mcp.disabled = false; regenerate.disabled = mcp.getAttribute('aria-pressed') === 'true'; }
    }
    mcp.addEventListener('click', () => refreshMcp(mcp.getAttribute('aria-pressed') !== 'true'));
    regenerate.addEventListener('click', async () => {
      if (!await confirmAction('Regenerate the MCP token? Existing client configurations will need the new token.')) return;
      await refreshMcp(undefined, true);
      config.focus();
      config.select();
    });
    connection.addEventListener('click', async () => {
      menu.open = false;
      document.activeElement?.blur?.();
      await refreshMcp();
      if (!dialog.open) dialog.showModal();
      config.focus();
      config.select();
    });
    menu.addEventListener('toggle', () => { if (menu.open) void refreshMcp(); });
    void refreshMcp();

  }
  window.__tetoricaCloseRequested = async () => {
    try { await window.__tetoricaAutosaveFlush?.(); }
    catch (error) {
      if (!await confirmAction(`Autosave failed: ${error.message ?? error}. Close without saving?`)) return;
    }
    await window.__TAURI_INTERNALS__.invoke('window_close');
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount, {once:true});
  else mount();
})();
