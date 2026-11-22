"""Exercise the Windows WSL picker and SSH transport against a dedicated test distribution."""
import argparse
import json
import re
import tempfile
import time
from pathlib import Path

from desktop_fixture import Desktop, Peer, expect


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--distribution', required=True)
    parser.add_argument('--port', type=int, required=True)
    parser.add_argument('--key', type=Path, required=True, help='Unencrypted test login key')
    parser.add_argument('--fingerprint', required=True, help='Expected SHA256 SSH host fingerprint')
    args = parser.parse_args()
    root = Path(tempfile.mkdtemp(prefix='arcnix-wsl-', dir=Path(__file__).resolve().parents[1] / 'artifacts'))
    config = dict(version=1, config=dict(groups=[], servers=[], trusted_hosts=[], local_directory=str(root)))
    with Desktop(root / 'client', config) as app:
        page = app.page
        page.locator('.tree-new-server').click()
        page.get_by_role('button', name='Choose local WSL distribution', exact=True).click()
        page.get_by_role('button').filter(has=page.get_by_text(args.distribution, exact=True)).click()
        user = page.get_by_role('textbox', name='Username', exact=True).input_value()
        assert user
        page.get_by_role('textbox', name='Port', exact=True).fill(str(args.port))
        page.get_by_role('button', name='Private key', exact=True).click()
        page.get_by_role('textbox', name='Private key file').fill(str(args.key.resolve(strict=True)))
        page.get_by_role('button', name='Save server').click()
        app.workspace('Home')
        page.locator('.overview-transfer').click()
        expect(page.locator('.fingerprint')).to_have_text(args.fingerprint)
        page.get_by_role('button', name='Trust and save').click()
        expect(page.locator('.connection-state')).to_have_text('SSH connected')
        expect(page.get_by_text(re.compile(r'^SFTP · \d+ items?$'))).to_be_visible()
    with Desktop(root / 'client', None) as app:
        app.connect()
        app.application('Terminal')
        screen = app.page.locator('.terminal-surface:visible')
        expect(screen.locator('.xterm-helper-textarea')).to_be_attached()
        # WSL can idle out even with a background sshd and an open TCP connection.
        time.sleep(12)
        screen.locator('.xterm-helper-textarea').focus()
        app.page.keyboard.type(r"""printf 'wsl-result=%s\n' "$(id -un)" """.strip())
        app.page.keyboard.press('Enter')
        expect(screen.locator('.xterm-accessibility-tree')).to_contain_text(f'wsl-result={user}')
        config = json.loads(app.config.read_text(encoding='utf-8'))
    with Peer(root / 'other-server') as other:
        profile = config['config']['servers'][0]
        assert profile['wsl_distribution'] == args.distribution
        profile['port'] = int(other.data['address'].rsplit(':', 1)[1])
        config['config']['trusted_hosts'] = []
        with Desktop(root / 'wrong-port', config) as app:
            app.page.locator('.host-select').click()
            app.workspace('Home')
            app.page.locator('.overview-transfer').click()
            expect(app.page.get_by_text(re.compile('SSH host key does not belong')).first).to_be_visible()
            expect(app.page.locator('.fingerprint')).to_have_count(0)

if __name__ == '__main__':
    main()
