if (window.arcnixWorkspace.zh) await import('/zh.js');
try {
  await import('/workbench.js');
} catch (error) {
  document.body.textContent = error.message;
  window.ipc.postMessage(
    JSON.stringify({ op: 'error', message: error.message }),
  );
}
