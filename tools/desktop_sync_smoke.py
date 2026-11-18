"""Two independent Arcnix clients on one owned SSH account, with interrupted sync."""
import json
import shutil
import time

from desktop_browser_smoke import site
from desktop_fixture import Desktop, Peer, ROOT, eventually, executable, expect


def settings(client, section):
    client.application('Settings')
    area = client.page.locator('.desktop-settings-page')
    area.get_by_role('button', name=section, exact=True).click()
    return area


def browser(client):
    client.application('Web browser')
    return client.page.locator('.desktop-window.app-browser')


def visit(view, url, title):
    address = view.get_by_role('textbox', name='Web address', exact=True)
    address.fill(url)
    address.press('Enter')
    expect(view.locator('.browser-tab.active .browser-tab-label')).to_have_text(title, timeout=25000)


def pending(client, filename):
    return next(iter((client.out / 'sync-outbox').glob(f'*/{filename}')), None)


def main():
    out = ROOT / 'artifacts' / 'desktop-sync' / f'run-{time.time_ns()}'
    binaries = []
    for name in ('a', 'b'):
        binary = out / name / 'arcnix.exe'
        binary.parent.mkdir(parents=True)
        shutil.copy2(executable(), binary)
        binaries.append(binary)
    env = {'WEBVIEW2_USER_DATA_FOLDER': ''}
    with site() as server, Peer(out / 'ssh', browser_port=server.server_port) as peer:
        config = peer.config([peer.profile('Shared SSH account')], out)
        endpoint = f'http://browser-fixture.test:{server.server_port}'
        dropped = peer.root / 'sync-offline'
        home = peer.root / 'home' / config['config']['servers'][0]['username']
        shared = home / '.arcnix' / 'desktop.json'

        def remote():
            return json.loads(shared.read_text(encoding='utf-8')) if shared.exists() else {}

        with Desktop(out / 'a', config, binary=binaries[0], extra_env=env) as a, Desktop(
            out / 'b', config, binary=binaries[1], extra_env=env, runtime=a.runtime,
        ) as b:
            a.connect()
            b.connect()
            first, second = browser(a), browser(b)
            visit(first, endpoint + '/sync/start', 'Session cookie created')
            visit(second, endpoint + '/sync/check', 'No session cookie')
            first.get_by_role('button', name='Toggle bookmark', exact=True).click()
            eventually(lambda: any(v['url'].endswith('/sync/start') for v in remote().get('browser_bookmarks', [])))
            second.get_by_role('button', name='Bookmarks', exact=True).click()
            expect(second.locator('.browser-bookmark-panel').get_by_role(
                'button', name='Session cookie created', exact=True,
            )).to_be_visible(timeout=25000)
            eventually(lambda: len(list((home / '.arcnix/browser/profiles').glob('*/cookies.enc'))) == 2)
            before = {p: p.read_bytes() for p in (home / '.arcnix/browser/profiles').glob('*/cookies.enc')}
            assert all(data.startswith(b'ARCNIX-COOKIE-DPAPI-V1\0') for data in before.values())
            assert all(b'arcnix_sync' not in data and b'synthetic-cookie-secret' not in data for data in before.values())

            # A failed SSH request must leave an on-disk outbox, including when the app exits.
            dropped.touch()
            visit(first, endpoint + '/sync/update', 'Session cookie updated')
            a_settings = settings(a, 'Desktop')
            a_settings.get_by_role('button', name='Dark', exact=True).click()
            b_settings = settings(b, 'General')
            b_settings.get_by_role('button', name='Every 5 s', exact=True).click()
            expect(a_settings.get_by_text('Pending sync', exact=True)).to_be_visible()
            expect(b_settings.get_by_text('Pending sync', exact=True)).to_be_visible()
            eventually(lambda: json.loads(pending(a, 'cookies.json').read_text())['pending']['dirty'])
            assert pending(a, 'desktop.json') and pending(b, 'desktop.json')
            assert remote().get('theme') == 'System'

            # Both clients now retry against the same remote account.
            dropped.unlink()
            eventually(lambda: remote().get('theme') == 'Dark' and remote().get('resource_interval_seconds') == 5,
                       timeout=30)
            eventually(lambda: pending(a, 'desktop.json') is None and pending(b, 'desktop.json') is None)
            eventually(lambda: any(p.read_bytes() != old for p, old in before.items()))
            assert sum(p.read_bytes() != old for p, old in before.items()) == 1

            # Same-field edits require a selection while distinct changes survive.
            b.page.locator('.desktop-settings-page').get_by_role('button', name='Desktop', exact=True).click()
            expect(b_settings.get_by_role('button', name='Dark', exact=True)).to_have_attribute(
                'aria-pressed', 'true', timeout=25000,
            )
            dropped.touch()
            a_settings.get_by_role('button', name='Light', exact=True).click()
            b_settings.get_by_role('button', name='Follow application', exact=True).click()
            expect(a_settings.get_by_text('Pending sync', exact=True)).to_be_visible()
            expect(b_settings.get_by_text('Pending sync', exact=True)).to_be_visible()
            dropped.unlink()
            conflict = eventually(lambda: next((view for view in (a_settings, b_settings)
                if view.get_by_role('button', name='Keep local changes', exact=True).is_visible()), None), timeout=30)
            conflict.get_by_role('button', name='Keep server changes', exact=True).click()
            eventually(lambda: pending(a, 'desktop.json') is None and pending(b, 'desktop.json') is None)
            assert remote()['resource_interval_seconds'] == 5
            assert any(v['url'].endswith('/sync/start') for v in remote()['browser_bookmarks'])

            # Save an edit while SSH is unavailable, and exit before it can be acknowledged.
            dropped.touch()
            b_settings.get_by_role('button', name='Dark', exact=True).click()
            expect(b_settings.get_by_text('Pending sync', exact=True)).to_be_visible()
            assert pending(b, 'desktop.json')

        dropped.unlink()
        # Restore on the same Windows user/device with a fresh WebView cache
        # and only the persisted Arcnix profile ID from the original client.
        old = next((out / 'a' / 'sync-outbox').glob('*/browser-client-id'))
        restored = out / 'restored'
        target = restored / 'sync-outbox' / old.parent.name / old.name
        target.parent.mkdir(parents=True)
        shutil.copyfile(old, target)
        with Desktop(restored, config, binary=binaries[0], extra_env=env) as a, Desktop(
            out / 'b', None, binary=binaries[1], extra_env=env, runtime=a.runtime,
        ) as b:
            a.connect()
            visit(browser(a), endpoint + '/sync/check', 'Updated session cookie restored')
            b.connect()
            eventually(lambda: remote().get('theme') == 'Dark' and pending(b, 'desktop.json') is None, timeout=30)
            visit(browser(b), endpoint + '/sync/check', 'No session cookie')
    print('Desktop and bookmark sharing, conflict resolution, restart retry, encrypted Cookie restore and isolation passed.')


if __name__ == '__main__':
    main()
