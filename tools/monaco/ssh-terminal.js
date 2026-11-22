import terminal, { SimpleTerminalBackend, SimpleTerminalProcess } from '@codingame/monaco-vscode-terminal-service-override';
import { Emitter } from '@codingame/monaco-vscode-api/vscode/vs/base/common/event';

export function sshTerminalServices(root, send, shellLabel) {
  const sessions = new Map();
  let next = 0;
  class SshProcess extends SimpleTerminalProcess {
    constructor(cols, rows) {
      const output = new Emitter();
      const token = ++next;
      super(token, token, root, output.event);
      this.output = output;
      this.ready = new Emitter();
      this.onProcessReady = this.ready.event;
      this.exit = new Emitter();
      this.onProcessExit = this.exit.event;
      this.decoder = new TextDecoder();
      this.dimensions = { cols, rows };
      sessions.set(token, this);
    }
    post(action, fields = {}) {
      send({ op: 'terminal', token: this.id, action, ...fields });
    }
    async start() {
      if (sessions.has(this.id)) this.post('open', this.dimensions);
    }
    write(bytes) {
      for (let i = 0; i < bytes.length; i += 16384) {
        const chunk = bytes.subarray(i, i + 16384);
        this.post('input', { data: btoa(String.fromCharCode(...chunk)) });
      }
    }
    input(text) { this.write(new TextEncoder().encode(text)); }
    processBinary = (data) => this.write(Uint8Array.from(data, c => c.charCodeAt(0)));
    clearBuffer() { /* No local PTY scrollback. The terminal view clears its own buffer. */ }
    resize(cols, rows) {
      this.dimensions = { cols, rows };
      this.post('resize', this.dimensions);
    }
    shutdown() {
      if (sessions.has(this.id)) this.post('close');
      this.finish();
    }
    finish(code) {
      if (!sessions.delete(this.id)) return;
      const tail = this.decoder.decode();
      if (tail) this.output.fire(tail);
      this.exit.fire(code);
      this.output.dispose();
      this.ready.dispose();
      this.exit.dispose();
    }
    async receive(base64) {
      const bytes = Uint8Array.from(atob(base64), c => c.charCodeAt(0));
      const data = this.decoder.decode(bytes, { stream: true });
      const packet = { data, trackCommit: true };
      if (data) this.output.fire(packet);
      await packet.writePromise;
      if (sessions.has(this.id)) this.post('consumed', { bytes: bytes.length });
    }
  }
  class SshBackend extends SimpleTerminalBackend {
    getProfiles = async () => [{ profileName: shellLabel, path: shellLabel, isDefault: true }];
    async getDefaultSystemShell() { return shellLabel; }
    async createProcess(_config, _cwd, cols, rows) { return new SshProcess(cols, rows); }
  }
  window.arcnixTerminalEvents = events => {
    for (const { token, ready, output, error, code } of events) {
      const session = sessions.get(token);
      if (!session) continue;
      if (ready) session.ready.fire({ pid: token, cwd: root });
      else if (output !== undefined) void session.receive(output);
      else {
        if (error) session.output.fire('\r\n' + error + '\r\n');
        session.finish(error ? 1 : code ?? undefined);
      }
    }
  };
  const backend = new SshBackend();
  backend.setReady();
  return terminal(backend);
}
