// `window.annotate`: the same narrow API the original Electron preload exposed,
// implemented over Tauri IPC. Each Rust command checks which window called it.
const T = window.__TAURI__;
const invoke = (command, args = {}) => T.core.invoke(command, args).catch(error => { throw error instanceof Error ? error : new Error(String(error)); });
const listen = (name, callback) => { void T.event.listen(name, event => callback(event.payload)); };

window.annotate = {
  state: () => invoke('state'),
  settings: value => invoke('settings', { value }),
  shortcutRecording: active => invoke('shortcut_recording', { active }),
  shortcutCapture: () => invoke('shortcut_capture'),
  consoleSize: mode => invoke('console_size', { mode }),
  detailOpen: (id, top) => invoke('detail_open', { id, top: Number(top) || 0 }),
  detail: () => invoke('detail'),
  detailEdit: () => invoke('detail_edit'),
  detailSave: (id, value) => invoke('detail_save', { id, value }),
  captureKind: kind => invoke('capture_kind', { kind }),
  desktopKind: kind => invoke('desktop_kind', { kind }),
  onDesktopKind: callback => listen('desktop-kind', callback),
  detailShow: id => invoke('detail_show', { id }),
  detailClose: () => invoke('detail_close'),
  developerPage: () => invoke('developer_page'),
  target: id => invoke('target', { id }),
  desktopConfig: () => invoke('desktop_config'),
  desktopReady: () => invoke('desktop_ready'),
  desktopMark: (points, complete = false) => invoke('desktop_mark', { points, complete }),
  onDesktopProgress: callback => listen('desktop-progress', callback),
  capture: options => invoke('capture', { options: options || null }),
  select: id => invoke('select', { id }),
  cropApp: points => invoke('crop_app', { points }),
  mark: (kind, points) => invoke('mark', { kind, points }),
  frozen: () => invoke('frozen'),
  cancel: () => invoke('cancel'),
  add: item => invoke('add', { item }),
  remove: id => invoke('remove', { id }),
  comment: (id, comment) => invoke('comment', { id, comment }),
  clear: () => invoke('clear'),
  send: (targetId, deliverAs) => invoke('send', { targetId, deliverAs }),
  quit: () => invoke('quit'),
  // Additions: clipboard, harness folder and integrations.
  annotationImage: id => invoke('annotation_image', { id }),
  clipboardWrite: (kind, data, alt = null) => invoke('clipboard_write', { kind, data, alt }),
  chooseFolder: () => invoke('choose_folder'),
  openFolder: () => invoke('open_folder'),
  integrations: () => invoke('integrations'),
  integrationSet: (id, install) => invoke('integration_set', { id, install }),
  rescan: () => invoke('rescan'),
  onChange: callback => listen('state-changed', () => callback()),
  onNotice: callback => listen('notice', callback),
};

// Frameless windows: drag from title bars (and the command pill's background),
// like Electron's -webkit-app-region: drag. Controls stay interactive.
document.addEventListener('mousedown', event => {
  if (event.button !== 0 || event.detail > 1) return;
  const target = event.target;
  if (!(target instanceof Element) || !target.closest('.drag, .layout-pill')) return;
  if (target.closest('button, select, textarea, input, a, label, nav, .tools, #console-body, #settings, .info, .project-location, .delivery, #privacy-warning, .entry-row')) return;
  event.preventDefault();
  void T.window.getCurrentWindow().startDragging();
});
