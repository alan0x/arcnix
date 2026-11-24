"""Exercise Files window shortcuts and application launcher menus."""
import json
import time

from server_apps_fixture import read_desktop_settings
from desktop_fixture import Desktop, Peer, ROOT, close_dialog, eventually, expect


def main():
    out = ROOT / 'artifacts/files-launchers' / str(time.time_ns())
    with Peer(out / 'peer') as peer:
        start = peer.root / 'Start'
        (start / 'Nested').mkdir(parents=True)
        (start / 'Nested' / 'marker.txt').write_text('nested', encoding='utf-8')
        home = peer.root / 'home/arcnix'
        (home / 'Music').mkdir(parents=True, exist_ok=True)
        prefs = home / '.arcnix/desktop.json'
        prefs.parent.mkdir(parents=True, exist_ok=True)
        prefs.write_text(json.dumps({'version': 1, 'theme': 'System', 'appearance': {'dock': {
            'order': ['player', 'editor', 'code'],
        }, 'overrides': {'icon_family': 'solid'}}}), encoding='utf-8')
        profile = {**peer.profile('Files launchers'), 'initial_remote_path': '/Start'}

        def dock_preferences():
            return read_desktop_settings(prefs).get('appearance', {}).get('dock', {})

        with Desktop(out / 'desktop', peer.config([profile], out)) as app:
            page = app.page
            app.connect()
            app.workspace('Desktop')
            expect(page.locator('.server-desktop')).to_have_attribute('data-desktop-icon-family', 'color')
            canvas = page.locator('.desktop-canvas')
            windows = page.locator('.desktop-window.app-files')
            active = page.locator('.desktop-window.app-files.active')
            menu = page.locator('.desktop-context-menu')

            def icon(zone, name):
                return page.locator(zone).get_by_role('button', name=name, exact=True)

            def choose(name):
                menu.get_by_role('button', name=name, exact=True).click()

            def location(path):
                expect(active.locator('.breadcrumb.current')).to_have_attribute('aria-label', path)

            dock_files = icon('.desktop-dock', 'Files')
            canvas.focus()
            page.keyboard.press('Control+n')
            expect(windows).to_have_count(0)
            page.keyboard.press('Control+e')
            expect(windows).to_have_count(1)
            location('/Start')

            app.file('Nested').click(button='right')
            app.open_in('New window')
            expect(app.file('marker.txt')).to_be_visible()
            expect(windows).to_have_count(2)
            location('/Start/Nested')
            active.locator('.desktop-window-mark').click()
            expect(canvas).to_be_focused()
            page.keyboard.press('Control+n')
            expect(windows).to_have_count(3)
            location('/Start/Nested')
            expect(active.locator('.files-tab')).to_have_count(1)
            active.locator('.files-sidebar').get_by_role('button', name='Home', exact=True).click()
            location('/home/arcnix')
            active.locator('.window-close').click()
            location('/Start/Nested')

            dock_files.click(button='right')
            choose('New window')
            expect(windows).to_have_count(3)
            location('/Start')
            app.show_desktop()
            desktop_files = icon('.desktop-shortcuts', 'Files')
            desktop_files.focus()
            page.keyboard.press('Shift+F10')
            expect(menu.get_by_role('button', name='New window', exact=True)).to_be_visible()
            choose('Music')
            expect(windows).to_have_count(4)
            location('/home/arcnix/Music')

            dock_files.click(button='right')
            choose('Remove from dock')
            eventually(lambda: 'files' in dock_preferences().get('unpinned', []))
            expect(dock_files).to_be_visible()
            for remaining in range(3, -1, -1):
                expect(dock_files).to_have_count(1)
                dock_files.click(button='right')
                choose('Close window')
                expect(windows).to_have_count(remaining)
            expect(dock_files).to_have_count(0)

            for label in ('Player', 'Text Editor', 'Code Editor'):
                expect(icon('.desktop-shortcuts', label)).to_have_count(0)
                expect(icon('.desktop-dock', label)).to_have_count(0)
            catalog = app.apps()
            search = catalog.get_by_role('textbox', name='Search apps', exact=True)
            expect(search).to_be_focused()
            app_tiles = catalog.locator('.desktop-app-catalog-grid').get_by_role('button')
            expected_apps = ['Files', 'Terminal', 'Code Editor', 'Browser', 'Player', 'Image Viewer',
                             'Office', 'Monitor', 'pgAdmin', 'App Center', 'Settings', 'Text Editor']
            expect(app_tiles).to_have_text(expected_apps)
            search.fill('not an app')
            expect(catalog.get_by_text('No matching apps')).to_be_visible()
            search.fill('player')
            expect(app_tiles).to_have_text(['Player'])
            search.press('Enter')
            expect(catalog).to_have_count(0)
            expect(page.locator('.desktop-window.app-player.active')).to_be_visible()
            player = icon('.desktop-dock', 'Player')
            expect(player).to_be_visible()
            page.locator('.desktop-window.app-player .window-minimize').click()
            player.click()
            expect(page.locator('.desktop-window.app-player.active')).to_be_visible()
            player.click(button='right')
            choose('Keep in dock')
            eventually(lambda: 'player' in dock_preferences().get('extra_pins', []))
            page.locator('.desktop-window.app-player .window-close').click()
            expect(player).to_be_visible()

            app.application('Files')
            active.locator('.files-sidebar').get_by_role('button', name='Home', exact=True).click()
            location('/home/arcnix')
            active.locator('.files-sidebar').get_by_role('button', name='Apps', exact=True).click()
            files_catalog = active.locator('.desktop-app-catalog')
            expect(files_catalog.get_by_role('textbox', name='Search apps')).to_have_attribute('placeholder', 'Search apps')
            files_catalog.locator('.files-apps-toolbar').get_by_role('button', name='App Center', exact=True).click()
            center = page.locator('.desktop-window.active.app-center')
            expect(center).to_be_visible()
            center.locator('.window-close').click()
            expect(files_catalog).to_be_visible()
            expect(files_catalog.locator('.desktop-app-catalog-grid').get_by_role('button')).to_have_text(expected_apps)
            expect(active.locator('.files-tab-select')).to_have_text('Apps')
            files_catalog.get_by_role('textbox', name='Search apps', exact=True).fill('text editor')
            files_catalog.get_by_role('button', name='Text Editor', exact=True).click()
            expect(page.locator('.desktop-window.app-editor.active')).to_be_visible()
            expect(icon('.desktop-dock', 'Text Editor')).to_be_visible()
            page.locator('.desktop-window.app-editor .window-close').click()
            expect(icon('.desktop-dock', 'Text Editor')).to_have_count(0)
            expect(files_catalog).to_be_visible()
            files_catalog.get_by_role('button', name='Back to folder', exact=True).click()
            expect(files_catalog).to_have_count(0)
            location('/home/arcnix')
            sidebar = active.locator('.files-sidebar').bounding_box()
            toolbar = active.locator('.files-window-tools').bounding_box()
            assert abs(toolbar['x'] - sidebar['x'] - sidebar['width']) < 2
            assert abs(toolbar['y'] - sidebar['y']) < 2
            active.locator('.window-close').click()

            page.locator('.desktop-dock').get_by_role('button', name='Launcher', exact=True).click(button='right')
            choose('Dock settings')
            tabs = page.locator('.desktop-settings-tabs')
            expect(tabs.get_by_role('button', name='Dock', exact=True)).to_have_attribute('aria-pressed', 'true')
            settings = page.locator('.desktop-settings-pane')
            settings.get_by_role('switch', name='Show locations', exact=True).click()
            settings.get_by_role('switch', name='Show restart button', exact=True).click()
            eventually(lambda: all(dock_preferences().get(key) for key in ('launcher_locations', 'launcher_restart')))
            app.apps().get_by_role('button', name='Music', exact=True).click()
            location('/home/arcnix/Music')
            def restart_requests():
                queries = (peer.root / 'server-app-queries.jsonl').read_text(encoding='utf-8').splitlines()
                return sum(json.loads(line)['command'].startswith('# arcnix:desktop:restart\n') for line in queries)
            confirm = page.get_by_role('dialog', name='Restart system?', exact=True)
            app.apps().get_by_role('button', name='Restart system…', exact=True).click()
            expect(confirm).to_contain_text('Files launchers')
            confirm.get_by_role('button', name='Cancel', exact=True).click()
            assert restart_requests() == 0
            app.apps().get_by_role('button', name='Restart system…', exact=True).click()
            confirm.get_by_role('button', name='Restart', exact=True).click()
            eventually(lambda: restart_requests() == 1)
            active.locator('.window-close').click()
            app.application('Settings')
            settings.get_by_role('button', name='App icons', exact=True).click()
            pins = page.get_by_role('dialog', name='App icons', exact=True)
            options = pins.locator('.preference-check')
            expect(options.first).to_be_visible()
            initial_order = options.all_text_contents()
            assert initial_order[:3] == ['Home', 'Files', 'Terminal']
            expect(pins.get_by_role('checkbox', name='Player', exact=True)).to_be_checked()
            for label in ('Text Editor', 'Code Editor'):
                expect(pins.get_by_role('checkbox', name=label, exact=True)).not_to_be_checked()
            pins.get_by_role('checkbox', name='Files', exact=True).check()
            eventually(lambda: 'files' not in dock_preferences().get('unpinned', []))
            expect(options).to_have_text(initial_order)
            close_dialog(pins)
            expect(dock_files).to_be_visible()

            dock_terminal = icon('.desktop-dock', 'Terminal')
            dock_terminal.click(button='right')
            choose('Open')
            terminal = page.locator('.desktop-window.app-terminal.active')
            expect(terminal.locator('.terminal-status')).to_have_text('Interactive SSH')
            dock_terminal.click(button='right')
            choose('New tab')
            expect(terminal.locator('.terminal-tab')).to_have_count(2)
            dock_terminal.click(button='right')
            choose('Close application')
            expect(page.locator('.desktop-window.app-terminal')).to_be_hidden()

        with Desktop(out / 'restarted', peer.config([profile], out)) as app:
            app.connect()
            app.workspace('Desktop')
            player = app.page.locator('.desktop-dock').get_by_role('button', name='Player', exact=True)
            player.click()
            player.click(button='right')
            app.page.locator('.desktop-context-menu').get_by_role('button', name='Remove from dock', exact=True).click()
            eventually(lambda: 'player' not in dock_preferences().get('extra_pins', []))
            expect(player).to_be_visible()
            app.page.locator('.desktop-window.app-player .window-close').click()
            expect(player).to_have_count(0)

        print('Files window shortcuts, running apps, launcher menus and persistent Dock pins passed.')


if __name__ == '__main__':
    main()
