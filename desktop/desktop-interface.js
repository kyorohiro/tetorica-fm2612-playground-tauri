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

  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount, {once:true});
  else mount();
})();
