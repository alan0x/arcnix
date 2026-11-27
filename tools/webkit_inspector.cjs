// Evaluate a test expression in this app's native WebKitGTK shell, not in a browser profile.
const [endpoint, expression] = process.argv.slice(2);
(async () => {
  const html = await fetch(endpoint).then(r => r.text());
  const rows = html.match(/<tr>.*?<\/tr>/gs) || [];
  const row = rows.find(r => r.includes('glory://localhost/index.html'));
  if (!row) throw new Error('Arcnix shell target is not ready');
  const path = row.match(/\/socket\/\d+\/\d+\/WebPage/)[0];
  const ws = new WebSocket(endpoint.replace(/^http/, 'ws') + path);
  const timer = setTimeout(() => { console.error('WebKit inspector timed out'); process.exit(1); }, 10000);
  ws.onmessage = event => {
    const message = JSON.parse(event.data);
    if (message.method === 'Target.targetCreated' && message.params.targetInfo.type === 'page') {
      ws.send(JSON.stringify({id: 1, method: 'Target.sendMessageToTarget', params: {
        targetId: message.params.targetInfo.targetId,
        message: JSON.stringify({id: 1, method: 'Runtime.evaluate', params: {expression, returnByValue: true}})
      }}));
    }
    if (message.method === 'Target.dispatchMessageFromTarget') {
      const result = JSON.parse(message.params.message);
      if (result.id !== 1) return;
      clearTimeout(timer); ws.close();
      if (result.error || result.result.wasThrown) {
        console.error(JSON.stringify(result)); process.exitCode = 1;
      } else console.log(JSON.stringify(result.result.result.value ?? null));
    }
  };
  ws.onerror = error => { console.error(String(error)); process.exit(1); };
})().catch(error => { console.error(String(error)); process.exit(1); });
