"""Exercise SSH Files authorization against an isolated OpenSSH and sudoers policy."""
import io
import shlex
import tempfile
import time
import zipfile
from pathlib import Path

from desktop_fixture import Desktop, ROOT, expect
from native_ssh_fixture import NativeSsh, wsl


def exercise(out):
    with NativeSsh(out / 'ssh') as peer, peer.client() as ssh, ssh.open_sftp() as sftp:
        folder = peer.root + '/protected'
        source = "notes 'one'.txt"
        q = shlex.quote
        wsl(f"set -eu\nmkdir -p {q(folder)}/tree/sub\nprintf note > {q(folder + '/' + source)}"
            f"\nprintf leaf > {q(folder)}/tree/sub/leaf\nchmod 600 {q(folder)}/tree/sub/leaf")
        batch = folder + '/batch'
        wsl(f"set -eu\nmkdir {q(batch)}\nchown {peer.data['uid']}:{peer.data['gid']} {q(batch)}"
            f"\nprintf A > {q(batch)}/A\nprintf B > {q(batch)}/B\nchmod 600 {q(batch)}/B")

        def policy(rule):
            # This is the fixture's private backing file for its mount namespace.
            sudoers = peer.root + '/private/sudoers'
            content = f"Defaults listpw=never,timestamp_timeout=0,passwd_tries=1,lecture=never\n{peer.data['username']} ALL=(root) {rule}\n"
            wsl(f"printf %s {q(content)} > {q(sudoers)}")

        profile = {**peer.profile(), 'initial_remote_path': folder}
        config = {'version': 1, 'config': {
            'servers': [profile], 'groups': [], 'local_directory': str(out),
            'trusted_hosts': [{'host': profile['host'], 'port': profile['port'],
                               'fingerprint': peer.data['fingerprint']}],
            'settings': {'language': 'English', 'theme': 'Light', 'show_hidden_files': True, 'resource_interval_seconds': 3}}}
        with Desktop(ROOT / 'artifacts/ssh-file-sudo' / str(time.time_ns()), config) as app:
            app.connect()
            app.application('Files')
            prompt = app.page.get_by_role('dialog', name='Administrator permission required')
            submit = prompt.get_by_role('button', name='Retry as administrator')

            def action(name, label):
                app.file(name).click(button='right')
                app.action(label)

            def authorize():
                expect(prompt).to_be_visible()
                prompt.get_by_label('Sudo administrator password').fill(peer.data['sudo'])
                submit.click()
                expect(prompt).to_be_hidden()

            action(source, 'Duplicate')
            expect(prompt).to_contain_text(folder + '/' + source)
            submit.click()
            expect(submit).to_be_disabled()
            prompt.get_by_role('button', name='Cancel', exact=True).click()
            assert "notes 'one' copy.txt" not in sftp.listdir(folder)
            action(source, 'Duplicate')
            authorize()
            expect(app.file("notes 'one' copy.txt")).to_be_visible()
            assert sftp.open(folder + "/notes 'one' copy.txt").read() == b'note'
            action('tree', 'Duplicate')
            authorize()
            expect(app.file('tree copy')).to_be_visible()
            assert sftp.open(folder + '/tree copy/sub/leaf').read() == b'leaf'
            action('tree', 'Make alias')
            authorize()
            expect(app.file('tree alias')).to_be_visible()
            assert sftp.readlink(folder + '/tree alias') == folder + '/tree'
            action('tree', 'Compress')
            authorize()
            expect(app.file('tree.zip')).to_be_visible()
            with zipfile.ZipFile(io.BytesIO(sftp.open(folder + '/tree.zip').read())) as archive:
                assert archive.read('tree/sub/leaf') == b'leaf'
            action(source, 'Copy')
            app.file('tree').dblclick()
            app.file('sub').click(button='right')
            app.action('Paste')
            expect(prompt).to_contain_text(folder + '/tree')
            authorize()
            expect(app.file(source)).to_be_visible()
            assert sftp.open(folder + '/tree/' + source).read() == b'note'
            assert not any(name.startswith('.arcnix-') for name in sftp.listdir(folder))
            breadcrumbs = app.page.locator('.app-files.active .breadcrumbs')
            breadcrumbs.get_by_role('button', name=folder, exact=True).click()
            app.file('batch').dblclick()
            app.file('A').click()
            app.file('B').click(modifiers=['Control'])
            action('B', 'Duplicate')
            expect(prompt).to_be_visible()
            expect(prompt.locator('.file-properties')).to_have_text(batch + '/B')
            assert sftp.open(batch + '/A copy').read() == b'A'
            authorize()
            expect(app.file('B copy')).to_be_visible()
            assert 'A copy (2)' not in sftp.listdir(batch)

            policy('NOPASSWD: ALL')
            action('B', 'Duplicate')
            expect(prompt).to_be_visible()
            submit.click()
            expect(app.file('B copy (2)')).to_be_visible()

            policy('NOPASSWD: /usr/bin/id')
            action('B', 'Duplicate')
            expect(app.page.get_by_text('The server declined the sudo policy check', exact=False).first).to_be_visible()
            expect(prompt).to_have_count(0)
            assert 'B copy (3)' not in sftp.listdir(batch)
            assert peer.data['sudo'] not in app.config.read_text(encoding='utf-8')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='arcnix-managed-sudo-', ignore_cleanup_errors=True) as tmp:
        exercise(Path(tmp))
    print('SSH Files sudo passed: cancel, duplicate, recursive copy, alias, ZIP, paste, partial batch, passwordless sudo and policy denial.')
