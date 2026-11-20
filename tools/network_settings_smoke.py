"""Verify the read-only network page and its confirmation before opening Terminal."""
import json
import time

from desktop_fixture import Desktop, Peer, ROOT, expect


def main():
    out = ROOT / 'artifacts/network-settings' / f'run-{time.time_ns()}'
    with Peer(out / 'peer') as peer:
        with Desktop(out / 'desktop', peer.config([peer.profile('Network fixture')], out)) as app:
            page = app.page
            app.connect()
            app.application('Files')
            files = page.locator('.files-window-tools')
            expect(files.get_by_role('button', name='Transfer files', exact=True)).to_have_count(0)
            expect(files.get_by_role('button', name='Help: Download destination', exact=True)).to_have_count(0)

            app.application('Settings')
            settings = page.locator('.desktop-settings-page')
            settings.get_by_role('button', name='Network', exact=True).click()
            network = settings.locator('.network-settings')
            for value in ['eth0', '192.0.2.42/24', '2001:db8::42/64', 'No IP addresses',
                          '192.0.2.1', 'fe80::1', '192.0.2.53', 'fe80::53%eth0']:
                expect(network).to_contain_text(value)
            expect(network.get_by_role('textbox')).to_have_count(0)
            app.screenshot('network-settings.png')

            unavailable = peer.root / 'network-unavailable'
            unavailable.touch()
            refresh = network.get_by_role('button', name='Refresh', exact=True)
            refresh.click()
            expect(network.locator('.error')).to_contain_text('iproute2 is required')
            expect(network).not_to_contain_text('192.0.2.42/24')
            unavailable.unlink()
            refresh.click()
            expect(network).to_contain_text('192.0.2.42/24')

            configure = network.get_by_role('button', name='Configure in Terminal…', exact=True)
            warning = page.get_by_role('dialog', name='Configure network', exact=True)
            terminal = page.locator('.desktop-window.app-terminal.active')
            running_terminal = page.locator('.desktop-dock .app-terminal.running')
            configure.click()
            expect(warning).to_contain_text('interrupt SSH and prevent reconnection')
            expect(page.locator('.desktop-modal-backdrop')).to_be_visible()
            expect(terminal).to_have_count(0)
            expect(running_terminal).to_have_count(0)
            warning.get_by_role('button', name='Cancel', exact=True).click()
            expect(warning).to_have_count(0)
            expect(network).to_be_visible()
            expect(terminal).to_have_count(0)
            expect(running_terminal).to_have_count(0)
            configure.click()
            warning.get_by_role('button', name='Continue to Terminal', exact=True).click()
            expect(warning).to_have_count(0)
            expect(terminal.locator('.terminal-status')).to_have_text('Interactive SSH')

            commands = [json.loads(line)['command'] for line in
                        (peer.root / 'server-app-queries.jsonl').read_text().splitlines()]
            queries = [command.splitlines()[0] for command in commands if command.startswith('# arcnix:network:')]
            assert queries == ['# arcnix:network:read'] * 3, queries
    print('Network inventory, refresh, unavailable state and SSH warning passed.')


if __name__ == '__main__':
    main()
