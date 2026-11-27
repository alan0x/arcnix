"""Ubuntu regressions for #43/#44 using owned SSH/HTTP fixtures and WebKit's inspector.

Run with a debug build, Node 22+, Paramiko, and a live desktop session:
    python tools/ubuntu_browser_smoke.py --exe target/debug/arcnix
No saved servers or real SSH accounts are used. Logs remain under artifacts/ubuntu-issues.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import socket
import signal
import ssl
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]


def eventually(probe, timeout=25):
    end = time.monotonic() + timeout
    last = None
    while time.monotonic() < end:
        try:
            result = probe()
            if result:
                return result
        except (OSError, ValueError, subprocess.SubprocessError, AssertionError) as error:
            last = error
        time.sleep(.15)
    raise AssertionError(f'Timed out waiting for fixture condition: {last}')


class Site(BaseHTTPRequestHandler):
    def do_GET(self):
        path = urlsplit(self.path)
        with self.server.lock:
            self.server.requests.append((self.headers.get('Host'), self.path))
        kind = 'text/html; charset=utf-8'
        cookie = None
        if path.path == '/styled':
            port = self.server.server_port
            body = f'''<title>Waiting for routed resources</title><h1 id="sample">SSH-only site</h1>
<script>
function report() {{ if(window.fixtureScript && getComputedStyle(document.querySelector('#sample')).color==='rgb(26, 89, 172)') {{
 document.title='Routed styles and scripts'; fetch('/proof?ipc='+typeof window.ipc+'&glory='+typeof window.__glory);
}} }}
document.addEventListener('visibilitychange',()=>fetch('/visibility?state='+document.visibilityState));
</script>
<link rel="stylesheet" href="http://assets.browser-fixture.test:{port}/style.css" onload="report()">
<script defer src="http://scripts.browser-fixture.test:{port}/script.js" onload="report()"></script>'''
            cookie = 'arcnix_fixture=first; Path=/; SameSite=Lax; Max-Age=600'
        elif path.path == '/style.css':
            kind, body = 'text/css', '#sample { color: rgb(26, 89, 172); }'
        elif path.path == '/script.js':
            kind, body = 'text/javascript', 'window.fixtureScript=true;'
        elif path.path == '/cookie':
            body = '<title>' + ('Cookie kept' if 'arcnix_fixture=first' in self.headers.get('Cookie', '') else 'Isolated profile') + '</title>'
        elif path.path == '/second':
            body = '<title>Second routed page</title><h1>Second tab</h1>'
        else:
            body = '<title>Fixture proof</title>'
        body = body.encode()
        self.send_response(200)
        self.send_header('Content-Type', kind)
        self.send_header('Content-Length', str(len(body)))
        if cookie:
            self.send_header('Set-Cookie', cookie)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def site(tls=None):
    server = ThreadingHTTPServer(('127.0.0.1', 0), Site)
    server.requests, server.lock = [], threading.Lock()
    if tls:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(*tls)
        server.socket = context.wrap_socket(server.socket, server_side=True, do_handshake_on_connect=False)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


class Desktop:
    def __init__(self, binary, config, out, backend):
        self.out = out
        self.backend = backend
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        self.endpoint = f'http://127.0.0.1:{port}'
        env = os.environ.copy()
        for key in ['WEBKIT_DMABUF_RENDERER_FORCE_SHM', 'WEBKIT_DISABLE_DMABUF_RENDERER', 'WEBKIT_DISABLE_COMPOSITING_MODE']:
            env.pop(key, None)
        env.update(GDK_BACKEND=backend, LANG='zh_CN.UTF-8', WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{port}')
        self.log = (out / f'{backend}.log').open('w')
        self.process = subprocess.Popen([str(binary), '--config', str(config)], cwd=ROOT, env=env, stdout=self.log, stderr=self.log)
        eventually(lambda: self.eval('!!document.querySelector(".navigation-rail")'), 30)

    def eval(self, expression):
        assert self.process.poll() is None, f'{self.backend} app exited with {self.process.returncode}; see {self.log.name}'
        result = subprocess.run(['node', str(ROOT/'tools/webkit_inspector.cjs'), self.endpoint, expression], cwd=ROOT, capture_output=True, text=True, timeout=12)
        if result.returncode:
            raise AssertionError(result.stderr)
        return json.loads(result.stdout)

    def click(self, selector):
        assert self.eval(f'(() => {{const b=document.querySelector({json.dumps(selector)}); if(!b)return false; b.click();return true}})()'), selector
        time.sleep(.55)

    def text(self):
        return self.eval('document.body.innerText')

    def connected(self):
        assert self.eval('document.querySelector(".connection-state")?.innerText === "SSH connected"'), self.text()[-3000:]

    def navigate(self, address, expected):
        self.eval(f'''(() => {{const input=document.querySelector('.browser-address'); input.value={json.dumps(address)}; input.dispatchEvent(new Event('input',{{bubbles:true}}));}})()''')
        self.click('.browser-open-address')
        if expected.startswith('Unable to load') or expected.startswith('The website certificate'):
            eventually(lambda: self.eval(f'(() => {{const e=document.querySelector(".browser-error"); return !!e && e.innerText.includes({json.dumps(expected)});}})()'), 35)
        else:
            eventually(lambda: self.title() == expected, 35)
        self.connected()

    def title(self):
        return self.eval('document.querySelector(".desktop-window.app-browser:not(.hidden) .browser-tab.active .browser-tab-label")?.innerText.trim()')

    def close(self):
        assert self.process.poll() is None, f'{self.backend} app exited unexpectedly: {self.process.returncode}'
        if self.backend == 'x11':
            self.close_native_window()
            self.process.wait(timeout=10)
            assert self.process.returncode == 0, f'Native close failed: {self.process.returncode}'
        else:
            self.process.terminate()
            self.process.wait(timeout=10)
        assert self.process.returncode in (0, -signal.SIGTERM), f'{self.backend} shutdown: {self.process.returncode}'
        (self.out/f'{self.backend}-exit.json').write_text(json.dumps(dict(exit_code=self.process.returncode)))
        self.log.close()

    def close_native_window(self):
        import re
        tree=subprocess.check_output(['xwininfo','-root','-tree'],text=True)
        target=None
        for identifier in re.findall(r'(0x[0-9a-f]+).*?\("arcnix" "Arcnix"\)',tree):
            prop=subprocess.check_output(['xprop','-id',identifier,'_NET_WM_PID'],text=True)
            if prop.strip().endswith('= '+str(self.process.pid)):
                target=int(identifier,16);break
        assert target is not None, 'Missing owned Arcnix X11 window'
        class Data(ctypes.Union):
            _fields_=[('l',ctypes.c_long*5),('b',ctypes.c_char*20)]
        class Message(ctypes.Structure):
            _fields_=[('type',ctypes.c_int),('serial',ctypes.c_ulong),('send_event',ctypes.c_int),('display',ctypes.c_void_p),('window',ctypes.c_ulong),('message_type',ctypes.c_ulong),('format',ctypes.c_int),('data',Data)]
        class Event(ctypes.Union):
            _fields_=[('message',Message),('pad',ctypes.c_long*24)]
        x=ctypes.CDLL('libX11.so.6');x.XOpenDisplay.restype=ctypes.c_void_p
        display=x.XOpenDisplay(None)
        x.XInternAtom.argtypes=[ctypes.c_void_p,ctypes.c_char_p,ctypes.c_int];x.XInternAtom.restype=ctypes.c_ulong
        x.XSendEvent.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.c_int,ctypes.c_long,ctypes.POINTER(Event)]
        x.XFlush.argtypes=[ctypes.c_void_p];x.XCloseDisplay.argtypes=[ctypes.c_void_p]
        event=Event();event.message=Message(33,0,1,display,target,x.XInternAtom(display,b'WM_PROTOCOLS',0),32,Data())
        event.message.data.l[0]=x.XInternAtom(display,b'WM_DELETE_WINDOW',0)
        x.XSendEvent(display,target,0,0,ctypes.byref(event));x.XFlush(display);x.XCloseDisplay(display)


def run_backend(binary, out, backend, peer, web, tls):
    profile = dict(id='ubuntu-a', name='Ubuntu fixture A', host='127.0.0.1', port=int(peer['address'].rsplit(':', 1)[1]), username='arcnix', auth='PrivateKey', private_key=peer['client_key_ed25519'], initial_remote_path='/__fixture_non_utf8_names__-ubuntu')
    config = dict(version=1, config=dict(servers=[profile, dict(profile, id='ubuntu-b', name='Ubuntu fixture B')], groups=[], trusted_hosts=[dict(host='127.0.0.1', port=profile['port'], fingerprint=peer['host_key_rsa_fingerprint'])], local_directory=str(out), settings=dict(language='English', theme='Light', show_hidden_files=True, resource_interval_seconds=3)))
    path = out / f'{backend}-config.json'
    path.write_text(json.dumps(config))
    ui = Desktop(binary, path, out, backend)
    checks = []
    try:
        ui.click('.host-select')
        ui.click('.overview-transfer')
        eventually(lambda: 'SSH connected' in ui.text())
        eventually(lambda: 'non-UTF-8' in ui.text())
        ui.click('.workbench-tabs button[aria-label="Home"]')
        ui.click('.overview-connect')
        for name in ['Files', 'Terminal', 'Settings', 'Files', 'Settings']:
            ui.eval(f'''(() => {{const icon=document.querySelector('.desktop-shortcut[aria-label="{name}"]'); if(!icon)throw Error('Missing {name}'); icon.dispatchEvent(new MouseEvent('dblclick',{{bubbles:true,button:0,detail:2}}));}})()''')
            time.sleep(.8)
            ui.connected()
            expected = 'files' if name == 'Files' else name.lower()
            assert ui.eval(f'!!document.querySelector(".desktop-window.app-{expected}:not(.hidden)")'), ui.text()[-2500:]
            ui.click(f'.desktop-window.app-{expected}:not(.hidden) .window-close')
            ui.connected()
        checks.append('Desktop icons survive repeated double-clicks with non-UTF-8 names and denied settings writes; SSH remains connected')
        ui.click('.desktop-dock-app[aria-label="Settings"]')
        ui.eval("""(() => {
            const select=[...document.querySelectorAll('.desktop-window.app-settings select')].find(s=>s.getClientRects().length && s.options.length>1);
            if(!select)throw Error('Missing resource interval selector');
            select.selectedIndex=select.selectedIndex===1 ? 2 : 1;
            select.dispatchEvent(new Event('input',{bubbles:true}));
            select.dispatchEvent(new Event('change',{bubbles:true}));
        })()""")
        eventually(lambda: (Path(peer['root'])/'desktop-write-denied-attempted').exists())
        ui.connected()
        ui.click('.desktop-window.app-settings .window-close')
        checks.append('Dock launch also remains connected')
        ui.click('.desktop-dock-app[aria-label="Browser"]')
        port = web.server_port
        ui.navigate(f'http://browser-fixture.test:{port}/styled', 'Routed styles and scripts')
        eventually(lambda: any('/proof?ipc=undefined&glory=undefined' in path for _, path in web.requests))
        checks.append('Page, CSS, JavaScript, and fetch use the SSH server; page has no desktop IPC')
        ui.click('.browser-page button[aria-label="New web tab"]')
        ui.navigate(f'http://127.0.0.1:{port}/second', 'Second routed page')
        ui.click('.browser-tab-label')
        eventually(lambda: ui.eval('document.querySelector(".browser-address").value').endswith('/styled'))
        ui.connected()
        checks.append('Tab switching and remote loopback work')
        ui.navigate(f'http://browser-fixture.test:{port}/second', 'Second routed page')
        ui.click('.browser-back')
        eventually(lambda: ui.title() == 'Routed styles and scripts')
        ui.click('.browser-toolbar button[aria-label="Forward"]')
        eventually(lambda: ui.title() == 'Second routed page')
        requests = sum(path == '/second' for _, path in web.requests)
        ui.click('.browser-toolbar button[aria-label="Reload page"]')
        eventually(lambda: sum(path == '/second' for _, path in web.requests) > requests)
        ui.click('.browser-back')
        eventually(lambda: ui.title() == 'Routed styles and scripts')
        checks.append('Back, forward, and reload operate on the selected native page')
        start = len(web.requests)
        ui.click('.desktop-dock-app[aria-label="Settings"]')
        ui.connected()
        eventually(lambda: any('/visibility?state=hidden' in path for _, path in web.requests[start:]))
        ui.click('.desktop-window.app-settings .window-close')
        ui.connected()
        eventually(lambda: any('/visibility?state=visible' in path for _, path in web.requests[start:]))
        checks.append('The native browser hides while covered and reappears after uncovering')
        ui.navigate('http-only.browser-fixture.test/second', 'Second routed page')
        assert ui.eval('document.querySelector(".browser-address").value').startswith('http://')
        ui.navigate('192.0.2.87/second', 'Second routed page')
        checks.append('Bare HTTP-only hostnames and IP addresses fall back through SSH')
        ui.navigate('https://example.com/', 'Example Domain')
        ui.navigate('example.com', 'Example Domain')
        checks.append('Valid explicit and bare HTTPS retain TLS through the SSH server')
        ui.navigate('https://http-only.browser-fixture.test/explicit-https', 'Unable to load this website')
        assert not any('/explicit-https' in path for _, path in web.requests)
        ui.navigate('https://tls.browser-fixture.test/', 'The website certificate could not be verified')
        ui.navigate('tls.browser-fixture.test/', 'The website certificate could not be verified')
        attempts = [json.loads(line) for line in (Path(peer['root'])/'browser-connect-attempts.jsonl').read_text().splitlines()]
        for host in ['browser-fixture.test', 'assets.browser-fixture.test', 'scripts.browser-fixture.test', '127.0.0.1']:
            assert any(x['host'] == host and x['port'] == port for x in attempts), (host, attempts)
        assert any(x['host'] == 'example.com' and x['port'] == 443 for x in attempts)
        assert not any(x['host']=='tls.browser-fixture.test' and x['port']==80 for x in attempts)
        (out/f'{backend}-ssh-attempts.json').write_text(json.dumps(attempts, indent=2))
        checks.append('Explicit schemes are binding; certificate errors on explicit and bare HTTPS never downgrade to HTTP')
        ui.navigate(f'http://browser-fixture.test:{port}/cookie', 'Cookie kept')
        ui.eval("document.querySelectorAll('.host-select')[1].click()"); time.sleep(.55)
        ui.click('.overview-connect')
        eventually(lambda: 'SSH connected' in ui.text())
        ui.click('.desktop-dock-app[aria-label="Browser"]')
        ui.navigate(f'http://browser-fixture.test:{port}/cookie', 'Isolated profile')
        checks.append('Cookies are isolated between server profiles')
        ui.click('.desktop-window.app-browser .window-close')
        ui.connected()
        checks.append('Closing the browser keeps SSH and the desktop alive')
        return dict(backend=backend, checks=checks, passed=True)
    except Exception:
        if ui.process.poll() is None:
            try: (out/f'{backend}-failure.txt').write_text(ui.text())
            except Exception: pass
        raise
    finally:
        ui.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exe', type=Path, default=ROOT/'target/debug/arcnix')
    parser.add_argument('--backend', choices=['x11', 'wayland', 'both'], default='both')
    parser.add_argument('--https-address', help='Optional public example.com address resolved by the SSH fixture')
    args = parser.parse_args()
    out = ROOT/'artifacts/ubuntu-issues/runtime'
    out.mkdir(parents=True, exist_ok=True)
    environment = dict(
        os_release=Path('/etc/os-release').read_text(),
        arcnix=subprocess.check_output([str(args.exe.resolve()), '--version'], text=True).strip(),
        webkit_package=subprocess.run(['dpkg-query', '-W', '-f=${Version}', 'libwebkit2gtk-4.1-0'], capture_output=True, text=True).stdout.strip(),
        source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True)),
        desktop_process_locale='zh_CN.UTF-8',
        fixture_scope='Owned loopback SSH/SFTP fixture; the original CentOS 8 host is not accessed',
    )
    (out/'environment.json').write_text(json.dumps(environment, indent=2))
    cert, key = out/'cert.pem', out/'key.pem'
    subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-subj', '/CN=tls.browser-fixture.test', '-addext', 'subjectAltName=DNS:tls.browser-fixture.test', '-keyout', str(key), '-out', str(cert)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    web, tls = site(), site((cert, key))
    ready = out/'peer.json'
    # The fixture owns and removes this metadata when it exits.
    if ready.exists(): ready.unlink()
    log = (out/'peer.log').open('w')
    command = [sys.executable, str(ROOT/'tools/sftp_fixture.py'), '--ready', str(ready), '--browser-port', str(web.server_port), '--browser-tls-port', str(tls.server_port), '--browser-web']
    if args.https_address:
        command += ['--browser-https-address', args.https_address]
    process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=log)
    try:
        peer = eventually(lambda: json.loads(ready.read_text()))
        root = Path(peer['root'])
        (root/'__fixture_non_utf8_names__-ubuntu').mkdir()
        (root/'__fixture_non_utf8_names__-ubuntu'/'readable.txt').write_text('Readable file')
        (root/'desktop-write-denied').touch()
        (root/'browser-forwarding-repaired.json').write_text(json.dumps(dict(arcnix=True)))
        backends = ['x11', 'wayland'] if args.backend=='both' else [args.backend]
        results = [run_backend(args.exe.resolve(), out, backend, peer, web, tls) for backend in backends]
        (out/'results.json').write_text(json.dumps(results, indent=2))
        print(json.dumps(results, indent=2))
    finally:
        process.terminate(); process.wait(timeout=10); log.close()
        web.shutdown(); tls.shutdown()


if __name__ == '__main__':
    main()
