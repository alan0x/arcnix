"""Open and save root-owned SSH text through both real desktop editors."""
import codecs
import shlex
import tempfile
import time
from pathlib import Path

from desktop_fixture import Desktop, ROOT, expect
from native_ssh_fixture import NativeSsh, wsl


def exercise(out):
    with NativeSsh(out / 'ssh') as peer:
        folder = peer.root + '/protected'
        q = shlex.quote
        names = ['text.txt', 'code.txt', 'denied.txt', 'public.txt', 'empty.txt']
        wsl('set -eu\nmkdir -m 0755 ' + q(folder) + '\numask 077\n' +
            '\n'.join("printf '\\357\\273\\277original\\r\\n' > " + q(folder + '/' + name) for name in names) +
            '\nchmod 644 ' + q(folder + '/public.txt'))

        wsl('truncate -s 0 ' + q(folder + '/empty.txt'))
        large = q(folder + '/large.log')
        make_log = "import sys; sys.stdout.write('protected line\\n' * 90000 + 'final record\\n')"
        wsl('umask 077\npython3 -c ' + q(make_log) + ' > ' + large)

        def verify(name, content):
            path = q(folder + '/' + name)
            assert wsl('cat -- ' + path).stdout == codecs.BOM_UTF8 + content + b'\r\n'
            assert wsl("stat -c '%u %g %a' -- " + path).stdout.strip() == b'0 0 600'

        profile = dict(peer.profile(), initial_remote_path=folder)
        config = dict(version=1, config=dict(servers=[profile], groups=[], local_directory=str(out),
            trusted_hosts=[dict(host=profile['host'], port=profile['port'], fingerprint=peer.data['fingerprint'])],
            settings=dict(language='English', theme='Light', show_hidden_files=True, resource_interval_seconds=3)))
        with Desktop(ROOT / 'artifacts/editor-auth' / str(time.time_ns()), config) as app:
            app.connect()
            app.application('Files')
            app.file('text.txt').dblclick()
            prompt = app.page.get_by_role('dialog', name='Administrator permission required')
            denied = 'You do not have permission to read this file.'

            def authorize(action):
                expect(prompt).to_be_visible()
                prompt.get_by_label('Sudo administrator password').fill(peer.data['sudo'])
                prompt.get_by_role('button', name=action, exact=True).click()
                expect(prompt).to_be_hidden()

            expect(prompt).to_contain_text(denied)
            prompt.get_by_role('button', name='Open as administrator').click()
            expect(prompt.get_by_role('button', name='Open as administrator')).to_be_disabled()
            prompt.get_by_role('button', name='Cancel').click()
            failure = app.page.locator('.editor-read-error:visible')
            expect(failure).to_contain_text(denied)
            failure.get_by_role('button', name='Retry').click()
            authorize('Open as administrator')
            text = app.page.get_by_role('textbox', name='File contents', exact=True)
            expect(text).to_have_value('original\n')
            text.fill('Text Editor change\n')
            expect(app.page.locator('.editor-feedback:visible')).to_have_text('Unsaved changes')
            text.press('Control+s')
            expect(prompt).to_be_visible()
            prompt.get_by_role('button', name='Cancel').click()
            expect(text).to_have_value('Text Editor change\n')
            verify('text.txt', b'original')
            text.press('Control+s')
            authorize('Retry as administrator')
            expect(app.page.locator('.editor-feedback:visible')).to_have_text('Remote file saved')
            verify('text.txt', b'Text Editor change')

            app.application('Code Editor')
            code = app.code()
            def open_code(name):
                code.get_by_role('treeitem', name=name, exact=True).dblclick()
            lines = code.locator('.monaco-editor:visible .view-lines').last
            open_code('public.txt')
            expect(lines).to_contain_text('original')
            expect(prompt).to_have_count(0)
            open_code('code.txt')
            expect(prompt).to_contain_text(denied)
            authorize('Open as administrator')
            expect(lines).to_contain_text('original')
            area = app.replace_code('Code Editor change\n')
            area.press('Control+s')
            authorize('Retry as administrator')
            expect(code.locator('.tabs-container .tab.active.dirty')).to_have_count(0)
            verify('code.txt', b'Code Editor change')

            open_code('empty.txt')
            authorize('Open as administrator')
            expect(code.locator('.tabs-container .tab.active')).to_contain_text('empty.txt')
            expect(lines).not_to_contain_text('original')
            open_code('large.log')
            expect(prompt).to_contain_text(denied)
            authorize('Open as administrator')
            viewer = code.locator('.arcnix-paged-file')
            expect(viewer.locator('.paged-row code').first).to_contain_text('protected line')
            viewer.locator('.paged-lines').press('Control+End')
            expect(viewer).to_contain_text('final record', timeout=30000)
            expect(prompt).to_have_count(0)

            # Listing is permitted; only the /usr/bin/id command can run as root.
            policy = 'Defaults listpw=never,timestamp_timeout=0\n' + peer.data['username'] + ' ALL=(root) NOPASSWD: /usr/bin/id\n'
            wsl('printf %s ' + q(policy) + ' > ' + q(peer.root + '/private/sudoers'))
            app.application('Files')
            app.file('denied.txt').click(button='right')
            menu = app.page.locator('.file-context-menu:visible')
            menu.get_by_role('button', name='Open with', exact=True).hover()
            menu.locator('.file-open-submenu').get_by_role('button', name='Code Editor', exact=True).click()
            expect(code.locator('.notifications-toasts')).to_contain_text(denied)
            expect(code.locator('.notifications-toasts')).to_contain_text('server declined')
            expect(prompt).to_have_count(0)
            verify('denied.txt', b'original')

            # An already-open document must keep its draft when save policy changes.
            app.application('Text Editor')
            expect(text).to_have_value('Text Editor change\n')
            text.fill('unpublished edit\n')
            expect(app.page.locator('.editor-feedback:visible')).to_have_text('Unsaved changes')
            text.press('Control+s')
            expect(app.page.locator('.editor-feedback:visible')).to_contain_text('You do not have permission to save this file.')
            expect(prompt).to_have_count(0)
            expect(text).to_have_value('unpublished edit\n')
            verify('text.txt', b'Text Editor change')

            passwordless = peer.data['username'] + ' ALL=(root) NOPASSWD: ALL\n'
            wsl('printf %s ' + q(passwordless) + ' > ' + q(peer.root + '/private/sudoers'))
            text.press('Control+s')
            prompt.get_by_role('button', name='Retry as administrator').click()
            expect(app.page.locator('.editor-feedback:visible')).to_have_text('Remote file saved')
            verify('text.txt', b'unpublished edit')
            app.application('Files')
            app.file('denied.txt').dblclick()
            prompt.get_by_role('button', name='Open as administrator').click()
            expect(text).to_have_value('original\n')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='arcnix-editor-auth-', ignore_cleanup_errors=True) as tmp:
        exercise(Path(tmp))
    print('SSH Text Editor and Code Editor authorization passed.')
