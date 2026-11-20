"""Exercise Files window shortcuts and application launcher menus."""
import json
import time

from server_apps_fixture import read_desktop_settings
from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def main():
    out = ROOT / 'artifacts/files-launchers' / str(time.time_ns())
    with Peer(out / 'peer') as peer:
        start = peer.root / 'Start'
        (start / 'Nested').mkdir(parents=True)
        (start / 'Nested' / 'marker.txt').write_text('nested', encoding='utf-8')
        home = peer.root / 'home/arcnix'
        (home / 'Music').mkdir(parents=True, exist_ok=True)
        prefs = home / '.arcnix/desktop.json'
        profile = {**peer.profile('Files launchers'), 'initial_remote_path': '/Start'}

        def dock_preferences():
            return read_desktop_settings(prefs).get('appearance', {}).get('dock', {})

        with Desktop(out / 'desktop', peer.config([profile], out)) as app:
            page = app.page
            app.connect()
            app.workspace('Desktop')
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
            app.action('Open in new window')
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
            choose('New Window')
            expect(windows).to_have_count(3)
            location('/Start')
            icon('.desktop-dock', 'Show desktop').click()
            desktop_files = icon('.desktop-shortcuts', 'Files')
            desktop_files.focus()
            page.keyboard.press('Shift+F10')
            expect(menu.get_by_role('button', name='New Window', exact=True)).to_be_visible()
            choose('Music')
            expect(windows).to_have_count(4)
            location('/home/arcnix/Music')

            dock_files.click(button='right')
            choose('Remove from Dock')
            eventually(lambda: 'files' in dock_preferences().get('unpinned', []))
            expect(dock_files).to_be_visible()
            for remaining in range(3, -1, -1):
                dock_files.click(button='right')
                choose('Close window')
                expect(windows).to_have_count(remaining)
            expect(dock_files).to_have_count(0)

            player = icon('.desktop-shortcuts', 'Player')
            dock_player = icon('.desktop-dock', 'Player')
            player.click(button='right')
            choose('Keep in Dock')
            eventually(lambda: 'player' in dock_preferences().get('order', []))
            expect(dock_player).to_be_visible()
            dock_player.click(button='right')
            expect(menu.get_by_role('button', name='Open video', exact=True)).to_be_visible()
            choose('Dock settings')
            settings = page.locator('.desktop-settings-pane')
            settings.get_by_role('button', name='Files', exact=True).click()
            eventually(lambda: 'files' not in dock_preferences().get('unpinned', []))
            expect(dock_files).to_be_visible()
            settings.get_by_role('button', name='Player', exact=True).click()
            eventually(lambda: 'player' not in dock_preferences().get('order', []))
            expect(dock_player).to_have_count(0)

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

            print('Files window shortcuts, launcher menus, folder links and Dock pins passed.')


if __name__ == '__main__':
    main()
