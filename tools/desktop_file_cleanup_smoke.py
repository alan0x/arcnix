"""Clean up desktop metadata without touching files or recoverable Trash entries."""
import json
import time

from server_apps_fixture import desktop_settings_dir, read_desktop_settings
from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def main():
    out = ROOT / 'artifacts/desktop-file-cleanup' / str(time.time_ns())
    with Peer(out / 'peer') as peer:
        for name in ('keep', 'delete', 'trashed'):
            (peer.root / name).mkdir()
        (peer.root / 'delete/inside').write_text('tracked child', encoding='utf-8')
        prefs = peer.root / 'home/arcnix/.arcnix/desktop.json'
        prefs.parent.mkdir(parents=True, exist_ok=True)
        paths = ['/keep', '/missing', '/delete', '/delete/inside', '/trashed']
        prefs.write_text(json.dumps({
            'version': 1, 'theme': 'System',
            'files': {'favorites': paths, 'folder_icons': {p: 'cloud' for p in paths if p != '/delete/inside'},
                      'colors': {p: 'blue' for p in paths}},
        }, separators=(',', ':')), encoding='utf-8')

        def recorded_paths():
            files = read_desktop_settings(prefs)['files']
            return set(files.get('favorites', [])) | files.get('colors', {}).keys() | files.get('folder_icons', {}).keys()

        with Desktop(out / 'desktop', peer.config([peer.profile('Metadata cleanup')], out)) as app:
            page = app.page
            app.connect()
            app.application('Files')

            def action(name, label):
                app.file(name).click(button='right')
                menu = page.locator('.file-context-menu:visible')
                menu.get_by_role('button', name=label, exact=True).press('Enter')

            action('trashed', 'Move to Trash')
            expect(app.file('trashed')).to_have_count(0)
            assert '/trashed' in recorded_paths()

            app.application('Settings')
            page.locator('.desktop-settings-tabs').get_by_role('button', name='Desktop', exact=True).click()
            page.get_by_role('button', name='Clean up missing entries', exact=True).click()
            expect(page.locator('.files-cleanup-status')).to_have_text('Cleaned up 1 missing Files paths')
            eventually(lambda: recorded_paths() == set(paths) - {'/missing'})
            assert '\n  "version": 1,\n' in (desktop_settings_dir(prefs) / 'system.json').read_text(encoding='utf-8')
            assert (peer.root / 'keep').is_dir()

            app.application('Files')
            action('delete', 'Delete permanently')
            page.get_by_role('dialog', name='Delete permanently', exact=True).get_by_role(
                'button', name='Delete permanently', exact=True).click()
            eventually(lambda: recorded_paths() == {'/keep', '/trashed'})
            assert not (peer.root / 'delete').exists()

            active = page.locator('.desktop-window.app-files.active')
            active.get_by_role('button', name='Trash', exact=True).click()
            row = active.locator('.trash-item').filter(has_text='/trashed')
            row.click(button='right')
            active.get_by_role('menuitem', name='Delete permanently', exact=True).click()
            active.locator('.trash-confirm').get_by_role('button', name='Delete permanently', exact=True).click()
            eventually(lambda: recorded_paths() == {'/keep'})
            print('Manual and automatic metadata cleanup, Trash retention and pretty desktop JSON passed.')


if __name__ == '__main__':
    main()
