"""Exercise Code Editor Ctrl+S with real SSH, SFTP and password-protected sudo."""
import argparse
import codecs
import shlex
import stat
import tempfile
from pathlib import Path

from desktop_fixture import Desktop, expect
from native_ssh_fixture import NativeSsh, wsl


def exercise(out, binary):
    with NativeSsh(out / 'ssh') as peer, peer.client() as ssh, ssh.open_sftp() as sftp:
        folder = peer.root + '/protected'
        path = folder + '/settings.txt'
        original = codecs.BOM_UTF8 + b'original\r\n'
        wsl(f"set -eu\nmkdir -m 0755 {shlex.quote(folder)}\numask 022\n"
            f"printf '\\357\\273\\277original\\r\\n' > {shlex.quote(path)}")
        assert peer.data['uid'] != 0

        def verify(expected):
            with sftp.open(path, 'rb') as file:
                assert file.read() == expected
                metadata = file.stat()
            assert (metadata.st_uid, metadata.st_gid, stat.S_IMODE(metadata.st_mode)) == (0, 0, 0o644)

        profile = {**peer.profile(), 'initial_remote_path': folder}
        config = dict(version=1, config=dict(
            servers=[profile], groups=[], local_directory=str(out),
            trusted_hosts=[dict(host=profile['host'], port=profile['port'], fingerprint=peer.data['fingerprint'])],
            settings=dict(language='English', theme='Light', show_hidden_files=True, resource_interval_seconds=3),
        ))
        with Desktop(out / 'app', config, binary=binary) as app:
            app.connect()
            app.application('Code Editor')
            code = app.code()
            code.get_by_role('treeitem', name='settings.txt', exact=True).dblclick()
            lines = code.locator('.monaco-editor:visible .view-lines').last
            expect(lines).to_contain_text('original')
            area = app.replace_code('first edit\n')
            dirty = code.locator('.tabs-container .tab.active.dirty')
            prompt = app.page.get_by_role('dialog', name='Administrator permission required', exact=True)
            notifications = code.locator('.notifications-toasts')

            def save():
                expect(dirty).to_be_visible()
                area.press('Control+s')
                expect(prompt).to_be_visible()
                expect(prompt).to_contain_text(path)

            def authorize(password):
                prompt.get_by_label('Sudo administrator password', exact=True).fill(password)
                prompt.get_by_role('button', name='Retry as administrator', exact=True).click()
                expect(prompt).to_be_hidden()

            save()
            verify(original)
            prompt.get_by_role('button', name='Cancel', exact=True).click()
            expect(prompt).to_be_hidden()
            expect(dirty).to_be_visible()
            verify(original)

            save()
            authorize('incorrect-fixture-password')
            expect(notifications).to_contain_text('Administrator file operation failed')
            expect(dirty).to_be_visible()
            verify(original)

            save()
            authorize(peer.data['sudo'])
            expect(dirty).to_have_count(0)
            verify(codecs.BOM_UTF8 + b'first edit\r\n')

            # A second save needs the new version returned by the privileged write.
            app.replace_code('second edit\n')
            save()
            authorize(peer.data['sudo'])
            expect(dirty).to_have_count(0)
            verify(codecs.BOM_UTF8 + b'second edit\r\n')

            app.replace_code('local draft\n')
            save()
            wsl(f"printf 'external edit\\n' > {shlex.quote(path)}")
            authorize(peer.data['sudo'])
            expect(notifications).to_contain_text('Remote file changed since it was opened')
            expect(dirty).to_be_visible()
            expect(lines).to_contain_text('local')
            verify(b'external edit\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    with tempfile.TemporaryDirectory(prefix='arcnix-code-save-', ignore_cleanup_errors=True) as tmp:
        exercise(Path(tmp), parser.parse_args().binary)
    print('Code Editor sudo save passed: cancel, bad password, two saves and concurrent edit.')
