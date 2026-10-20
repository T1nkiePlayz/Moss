// Called by the app when the controller could not be started.
window.__mossError = (text) => {
  const m = document.getElementById('msg');
  m.textContent = text;
  m.className = 'error';
  m.setAttribute('role', 'alert');
};
