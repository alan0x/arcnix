"""Connecting asks for a password or key passphrase only when one may be needed."""
import time

from desktop_fixture import Desktop, Peer, ROOT, expect


def main():
    out = ROOT / 'artifacts/connect-prompt' / f'run-{time.time_ns()}'
    out.mkdir(parents=True)
    with Peer(out / 'peer') as peer:
        unencrypted = peer.profile('Unencrypted key')
        encrypted = peer.profile('Encrypted key')
        encrypted['private_key'] = peer.data['client_key_ed25519_encrypted']
        password = peer.profile('Password login')
        password.update(auth='Password', private_key=None)
        rejected = peer.profile('Rejected key')
        rejected['private_key'] = peer.data['client_key_untrusted']
        config = peer.config([unencrypted, encrypted, password, rejected], out)
        with Desktop(out / 'desktop', config) as app:
            page = app.page
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            dialog = page.get_by_role('dialog', name='Connect to server', exact=True)
            state = page.locator('.connection-state')

            def start(name):
                app.servers()
                page.locator('.host-row').filter(has=page.get_by_text(name, exact=True)).locator('.host-select').click()
                app.workspace('Home')
                page.locator('.overview-transfer').click()

            def disconnect():
                page.get_by_role('button', name='Disconnect', exact=True).click()
                expect(state).to_have_text('SSH disconnected')

            start('Unencrypted key')
            expect(state).to_have_text('SSH connected')
            expect(dialog).to_have_count(0)
            disconnect()
            print('An unencrypted private key connects without a credential dialog.')

            start('Password login')
            expect(dialog).to_be_visible()
            expect(dialog).to_contain_text('Password')
            dialog.get_by_role('textbox', name='Password or key passphrase', exact=True).fill('arcnix-test')
            dialog.get_by_role('button', name='Connect', exact=True).click()
            expect(state).to_have_text('SSH connected')
            disconnect()
            print('Password logins still ask for the password.')

            start('Encrypted key')
            expect(dialog).to_be_visible()
            expect(dialog).to_contain_text('Private key passphrase')
            dialog.get_by_role('textbox', name='Password or key passphrase', exact=True).fill('arcnix-test-key-passphrase')
            dialog.get_by_role('button', name='Connect', exact=True).click()
            expect(state).to_have_text('SSH connected')
            disconnect()
            print('An encrypted private key still asks for its passphrase.')

            start('Rejected key')
            expect(dialog).to_be_visible(timeout=20000)
            expect(dialog).to_contain_text('Connection failed')
            expect(dialog).to_contain_text('Server rejected the SSH key')
            dialog.get_by_role('button', name='Cancel', exact=True).click()
            expect(dialog).to_have_count(0)
            expect(state).to_have_text('SSH disconnected')
            print('A failed attempt without a prompt reopens the dialog with the error.')
            assert not errors, errors


if __name__ == '__main__':
    main()
