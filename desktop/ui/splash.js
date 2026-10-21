const msg = document.getElementById('msg');
const detail = document.getElementById('detail');
const retry = document.getElementById('retry');

// Called by the app when the controller could not be started (or died): message, then log path and log tail.
window.__mossError = (text, log) => {
  msg.textContent = text;
  msg.className = 'error';
  msg.setAttribute('role', 'alert');
  msg.setAttribute('aria-busy', 'false');
  detail.textContent = log || '';
  detail.hidden = !log;
  retry.disabled = false;
  retry.hidden = false;
  retry.focus();
};

// Called when a retry begins.
window.__mossStarting = () => {
  msg.textContent = 'Starting Moss…';
  msg.className = '';
  msg.setAttribute('role', 'status');
  msg.setAttribute('aria-busy', 'true');
  detail.hidden = true;
  retry.hidden = true;
};

retry.addEventListener('click', () => {
  retry.disabled = true;
  window.__mossStarting();
  window.__TAURI_INTERNALS__.invoke('retry');
});
