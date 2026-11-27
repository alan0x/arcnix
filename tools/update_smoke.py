"""Exercise updater network, cancellation, installation and rollback on owned copies.

Only executables underneath this run's artifacts directory are ever replaced or
terminated. The real desktop installation and user configuration are untouched.
"""
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hashlib, io, json, os, zipfile, shutil, subprocess, threading, time
from desktop_fixture import ROOT, Desktop, Peer, executable, eventually, expect


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def portable(executable):
    archive = io.BytesIO()
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as package:
        package.writestr('arcnix-v9.9.9-windows-amd64/arcnix.exe', executable)
    return archive.getvalue()


class ReleaseServer:
    def __init__(self, payload, current):
        self.payload = payload
        self.current = current
        self.mode = 'good'
        self.stopped = threading.Event()
        owner = self
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_GET(self):
                try:
                    if self.path == '/latest':
                        if owner.mode == 'none':
                            self.send_error(404); return
                        version = owner.current if owner.mode == 'current' else '9.9.9'
                        digest = '0' * 64 if owner.mode == 'corrupt' else hashlib.sha256(owner.payload).hexdigest()
                        assets = [] if owner.mode == 'missing' else [dict(name=f'arcnix-v{version}-windows-amd64.zip', state='uploaded', size=len(owner.payload), digest='sha256:'+digest, browser_download_url=owner.base+'/candidate.zip')]
                        data = json.dumps(dict(tag_name='v'+version, html_url=owner.base+'/release', body='Owned update fixture', assets=assets)).encode()
                        self.send_response(200); self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data)
                    else:
                        self.send_response(200); self.send_header('Content-Length', str(len(owner.payload))); self.end_headers()
                        self.wfile.write(owner.payload[:1024]); self.wfile.flush()
                        if owner.mode == 'stalled': owner.stopped.wait(40)
                        self.wfile.write(owner.payload[1024:])
                except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError): pass
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.base = f'http://127.0.0.1:{self.server.server_port}'
    def __enter__(self):
        threading.Thread(target=self.server.serve_forever, daemon=True).start(); return self
    def __exit__(self, *_):
        self.stopped.set(); self.server.shutdown(); self.server.server_close()


STUB = r'''
use std::{env, fs, thread, time::Duration};
fn main() {
    let args:Vec<String> = env::args().collect();
    if args.iter().any(|a|a=="--version") { println!("arcnix VERSION"); return; }
    if FAIL { std::process::exit(17); }
    if let Some(i)=args.iter().position(|a|a=="--update-health") {
        fs::write(&args[i+1], "arcnix VERSION").unwrap();
    }
    loop {thread::sleep(Duration::from_millis(100));}
}
'''


def build_stub(out, name, version='9.9.9', fail=False):
    source = out/(name+'.rs'); binary=out/(name+'.exe')
    source.write_text(STUB.replace('VERSION', version).replace('FAIL', str(fail).lower()))
    subprocess.run(['rustc', str(source), '--edition=2024', '-o', str(binary)], cwd=ROOT, check=True, capture_output=True)
    return binary.read_bytes()


def owned_processes(directory, terminate=False):
    directory = directory.resolve()
    assert directory.is_relative_to(ROOT/'artifacts/update-ui') and directory != ROOT/'artifacts/update-ui'
    env=os.environ.copy(); env['ARCNIX_OWNED_UPDATE_TEST_ROOT']=str(directory)
    script = "$root=[IO.Path]::GetFullPath($env:ARCNIX_OWNED_UPDATE_TEST_ROOT).TrimEnd('\\')+'\\'; $items=@(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($root,[StringComparison]::OrdinalIgnoreCase) }); "
    script += "$items | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }" if terminate else "$items | ForEach-Object { $_.ProcessId }"
    result=subprocess.run(['powershell', '-NoProfile', '-Command', script], env=env, capture_output=True, text=True, check=True)
    return [int(value) for value in result.stdout.split()]


def main():
    out=ROOT/'artifacts/update-ui'/str(time.time_ns()); out.mkdir(parents=True)
    source=executable(); original=sha(source); checks=[]
    good=build_stub(out, 'healthy'); failed=build_stub(out, 'failed', fail=True); wrong=build_stub(out, 'wrong', version='9.9.8')
    version=subprocess.check_output([str(source), '--version'],text=True).strip().split()[-1]
    with Peer(out) as peer, ReleaseServer(portable(good),version) as server:
        def desktop(name, automatic=False, query=False, defaults=False):
            case=out/name; case.mkdir(); binary=case/'arcnix.exe'; shutil.copy2(source,binary)
            preferences = {} if defaults else dict(automatic_checks=False, automatic_download=automatic, install_when_idle=automatic)
            config=peer.config([peer.profile('Update fixture')], case, updates=preferences)
            if defaults:
                config['config']['settings']['language'] = 'SimplifiedChinese'
            if query:config['config']['snippets']=[dict(id='update-query',name='Read before update',description='',command='fixture-command',administrator=False)]
            return Desktop(case, config, binary=binary, extra_env={'ARCNIX_TEST_UPDATE_URL':server.base+'/latest'})
        def settings(app):
            app.settings('Updates'); return app.page.locator('.update-panel')
        def check(panel): panel.get_by_role('button', name='Check now', exact=True).click()
        def download(panel):
            panel.get_by_role('button', name='Download update', exact=True).click()
            expect(panel.locator('.update-status')).to_have_text('Update verified and ready to install', timeout=20000)
        def install(panel):
            panel.get_by_role('button', name='Restart and install', exact=True).click()
            dialog = panel.page.get_by_role('dialog', name='Software update', exact=True)
            dialog.get_by_role('button', name='Restart and install', exact=True).click()
        try:
            readonly = desktop('readonly')
            invalid_config = b'{"version":1,"config":'
            readonly.config.write_bytes(invalid_config)
            with readonly as app:
                panel = settings(app)
                expect(panel.get_by_text('Settings are read-only. You can still update Arcnix manually.')).to_be_visible()
                check(panel)
                expect(panel.locator('.update-status')).to_have_text('A new version is available')
                expect(app.page.get_by_text('Could not save settings:', exact=False)).not_to_be_visible()
                download(panel)
                expect(app.page.locator('.update-banner')).not_to_be_visible()
                install(panel)
                receipt = eventually(lambda: json.loads((app.out/'updates/installed.json').read_text()), timeout=30)
                assert receipt['version'] == '9.9.9' and sha(app.binary) == hashlib.sha256(good).hexdigest()
                assert app.config.read_bytes() == invalid_config
                checks.append('A read-only configuration permits manual download and confirmed installation while preserving the original bytes')
                owned_processes(app.out, True)
            with desktop('checks') as app:
                panel=settings(app)
                server.mode='none'; check(panel); expect(panel.locator('.update-status')).to_have_text('No published release is available yet')
                server.mode='current'; check(panel); expect(panel.locator('.update-status')).to_have_text('You are using the latest stable version')
                expect(app.page.locator('.rail-update')).to_have_count(0)
                server.mode='missing'; check(panel); expect(panel.locator('.update-status')).to_have_text('A new version is available'); expect(panel.get_by_role('button',name='Download update',exact=True)).not_to_be_visible()
                icon = app.page.get_by_role('button', name='Update available', exact=True)
                icon.click()
                dialog = app.page.get_by_role('dialog', name='Software update', exact=True)
                expect(dialog.get_by_role('button', name='Open releases', exact=True)).to_be_visible()
                expect(dialog.get_by_role('button', name='Restart and install', exact=True)).to_have_count(0)
                dialog.get_by_role('button', name='Later', exact=True).click()
                server.mode='corrupt'; check(panel); expect(panel.get_by_role('button',name='Download update',exact=True)).to_be_enabled()
                panel.get_by_role('button',name='Download update',exact=True).click(); expect(panel.locator('.update-error')).to_contain_text('checksum mismatch')
                assert sha(app.binary)==original and not (app.out/'updates/ready.json').exists()
                server.mode='stalled'; check(panel); expect(panel.get_by_role('button',name='Download update',exact=True)).to_be_enabled()
                panel.get_by_role('button',name='Download update',exact=True).click(); expect(panel.locator('.update-status')).to_contain_text('Downloading')
                start=time.monotonic(); panel.get_by_role('button',name='Cancel download',exact=True).click()
                expect(panel.locator('.update-error')).to_contain_text('canceled', timeout=4000)
                assert time.monotonic()-start<4 and sha(app.binary)==original
                checks.append('Handles missing/current/uninstallable releases, refuses corrupt checksums, and cancels a stalled network read promptly without replacing the application')
                server.mode='good'; server.payload=portable(wrong); check(panel); expect(panel.get_by_role('button',name='Download update',exact=True)).to_be_enabled(); download(panel); install(panel)
                expect(panel.locator('.update-error')).to_contain_text('version does not match', timeout=20000)
                assert app.process.poll() is None and sha(app.binary)==original
                checks.append('Rejects a verified executable whose reported version differs from the release before shutting down the application')
            server.payload=portable(good)
            with desktop('confirmation', defaults=True) as app:
                page = app.page
                rail = page.locator('.rail-bottom')
                icon = rail.get_by_role('button', name='有可用更新', exact=True)
                expect(icon).to_be_visible(timeout=25000)
                eventually(lambda: (app.out/'updates/ready.json').is_file(), timeout=20)
                page.wait_for_timeout(31000)
                assert app.process.poll() is None and sha(app.binary) == original
                expect(page.locator('.update-banner')).not_to_be_visible()
                assert icon.bounding_box()['y'] < rail.locator('.settings-link').bounding_box()['y']
                icon.click()
                dialog = page.get_by_role('dialog', name='更新 Arcnix', exact=True)
                expect(dialog).to_contain_text('Arcnix 9.9.9')
                expect(dialog).to_contain_text('Owned update fixture')
                expect(dialog.locator('.update-status')).to_have_text('更新已校验，等待安装', timeout=20000)
                dialog.get_by_role('button', name='稍后', exact=True).click()
                expect(dialog).not_to_be_visible()
                expect(icon).to_be_visible()
                icon.click()
                app.screenshot('update-confirmation.png')
                dialog.get_by_role('button', name='重启并安装', exact=True).click()
                receipt = eventually(lambda: json.loads((app.out/'updates/installed.json').read_text()), timeout=30)
                assert receipt['version'] == '9.9.9' and sha(app.binary) == hashlib.sha256(good).hexdigest()
                owned_processes(app.out,True)
                checks.append('Default preferences check and download in the background, display the rail icon above Settings, defer without restarting, and install on confirmation')
            with desktop('result-query',automatic=True,query=True) as app:
                p=app.page;p.locator('.rail-scripts').click()
                p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click()
                expect(p.locator('.script-row')).to_have_count(1)
                p.get_by_role('button',name='Run on servers…',exact=True).click()
                d=p.get_by_role('dialog',name='Run script on servers',exact=True)
                d.locator('.target-choice').first.click();d.get_by_role('button',name='Check targets',exact=True).click()
                expect(p.locator('.task-status')).to_have_text(['Ready'])
                p.get_by_role('button',name='Execute ready targets',exact=True).click()
                expect(p.locator('.task-status')).to_have_text(['Succeeded'],timeout=20000)
                panel=settings(app);check(panel)
                expect(panel.locator('.update-status')).to_have_text('Update verified and ready to install',timeout=20000)
                p.locator('.rail-tasks').click();p.locator('.task-batch-refresh:visible').click()
                d=p.get_by_role('dialog',name='Query batch results',exact=True)
                expect(d.locator('.task-refresh-choice.active')).to_have_count(1)
                p.wait_for_timeout(1200);expect(p.locator('.update-banner')).not_to_be_visible()
                delay=peer.root/'task-query-delay';delay.write_text('3')
                try:
                    d.get_by_role('button',name='Query selected results',exact=True).click();expect(d).not_to_be_visible()
                    expect(p.locator('.task-query-progress')).to_have_text(['Reading server result…'])
                    panel=settings(app)
                    expect(panel.get_by_role('button',name='Restart and install',exact=True)).to_be_disabled()
                    p.wait_for_timeout(1100);expect(p.locator('.update-banner')).not_to_be_visible()
                    expect(panel.get_by_role('button',name='Restart and install',exact=True)).to_be_enabled(timeout=10000)
                    expect(p.locator('.update-banner')).to_contain_text('Restarting in')
                    p.locator('.update-banner').get_by_role('button',name='Later',exact=True).click()
                    assert app.process.poll() is None and sha(app.binary)==original
                finally:delay.unlink(missing_ok=True)
                checks.append('Both manual restart and the automatic countdown wait for finite server-result queries to complete')
            with desktop('manual', automatic=True) as app:
                app.connect(); panel=settings(app); check(panel)
                expect(panel.locator('.update-status')).to_have_text('Update verified and ready to install',timeout=20000)
                expect(panel.get_by_role('button',name='Restart and install',exact=True)).to_be_disabled()
                expect(app.page.locator('.update-banner')).not_to_be_visible()
                app.servers(); app.page.get_by_role('button', name='Disconnect', exact=True).click(); expect(app.page.locator('.connection-state')).to_have_text('SSH disconnected')
                expect(app.page.locator('.update-banner')).to_contain_text('Restarting in')
                app.page.locator('.update-banner').get_by_role('button',name='View updates',exact=True).click()
                expect(app.page.locator('.page-header-title')).to_have_text('Updates')
                expect(app.page.locator('.update-panel')).to_be_visible()
                app.page.locator('.update-banner').get_by_role('button',name='Later',exact=True).click()
                expect(app.page.locator('.update-banner')).not_to_be_visible()
                panel=settings(app); app.screenshot('verified-update.png'); install(panel)
                receipt=eventually(lambda: json.loads((app.out/'updates/installed.json').read_text()),timeout=30)
                assert receipt['version']=='9.9.9' and sha(app.binary)==hashlib.sha256(good).hexdigest()
                assert sha(Path(str(app.binary)+'.previous'))==original and app.process.wait(timeout=5)==0
                assert receipt['pid'] in owned_processes(app.out)
                checks.append('Downloads while SSH is connected, waits for disconnection, offers a deferrable countdown, then installs and starts the verified executable with the old binary retained')
                owned_processes(app.out,True)
            server.payload=portable(failed)
            with desktop('rollback') as app:
                panel=settings(app); check(panel); expect(panel.get_by_role('button',name='Download update',exact=True)).to_be_enabled(); download(panel); install(panel)
                failure=eventually(lambda: json.loads((app.out/'updates/failure.json').read_text()),timeout=40)
                assert failure['version']=='9.9.9' and 'restored' in failure['message']
                assert sha(app.binary)==original and app.process.wait(timeout=5)==0
                eventually(lambda: owned_processes(app.out))
                checks.append('A replacement that exits before startup acknowledgement is rolled back to the exact original executable and the previous application is restarted')
                owned_processes(app.out,True)
            server.payload=portable(good)
            with desktop('automatic',automatic=True) as app:
                panel=settings(app); panel.get_by_role('checkbox',name='Check for updates automatically',exact=True).check()
                expect(panel.locator('.update-status')).to_have_text('Update verified and ready to install',timeout=25000)
                expect(app.page.locator('.update-banner')).to_contain_text('Restarting in')
                receipt=eventually(lambda: json.loads((app.out/'updates/installed.json').read_text()),timeout=45)
                assert receipt['version']=='9.9.9' and sha(app.binary)==hashlib.sha256(good).hexdigest()
                checks.append('The automatic scheduler checks, downloads, verifies, counts down while idle and completes an unattended installation')
                owned_processes(app.out,True)
        finally:
            owned_processes(out,True)
    result=dict(passed=len(checks), checks=checks, artifacts=str(out)); (out/'results.json').write_text(json.dumps(result,indent=2)); print(json.dumps(result))


if __name__=='__main__': main()
