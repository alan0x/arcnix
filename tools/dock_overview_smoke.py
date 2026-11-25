"""Exercise Dock overview gestures, window restoration and saved preferences."""
import re
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect
from server_apps_fixture import read_desktop_settings


def main():
    out = ROOT / 'artifacts/dock-overview' / str(time.time_ns())
    with Peer(out / 'peer') as peer:
        (peer.root / 'Start' / 'Nested').mkdir(parents=True)
        profile = dict(peer.profile('Window overview'), initial_remote_path='/Start')
        settings_file = peer.root / 'home/arcnix/.arcnix/desktop.json'

        def saved(key, expected):
            eventually(lambda: read_desktop_settings(settings_file).get('appearance', {}).get('dock', {}).get(key) == expected)

        with Desktop(out / 'desktop', peer.config([profile], out)) as app:
            p = app.page
            app.connect()
            app.workspace('Desktop')
            dock = p.locator('.desktop-dock')
            files = dock.get_by_role('button', name='Files', exact=True)
            settings = dock.get_by_role('button', name='Settings', exact=True)
            overview = p.locator('.window-overview')
            active = p.locator('.app-files.desktop-window.active')
            def location(path):
                expect(active.locator('.breadcrumb.current')).to_have_attribute('aria-label', path)
            def options():
                settings.click(button='right')
                p.locator('.desktop-context-menu').get_by_role('button', name='Dock settings').click()

            files.click()
            location('/Start')
            app.file('Nested').click(button='right')
            app.open_in('New window')
            location('/Start/Nested')
            active.locator('.window-maximize').click()
            expect(active).to_have_class(re.compile('maximized'))
            active.locator('.window-minimize').click()
            location('/Start')
            original = active.bounding_box()
            options()
            single = p.get_by_role('switch', name='Click the active app', exact=True)
            expect(single).to_be_checked()
            expect(p.get_by_role('switch', name='Double-click an inactive app')).to_have_count(0)

            # A background single click still raises the window immediately.
            files.click()
            location('/Start')
            files.click()
            tiles = overview.locator('.overview-select')
            expect(tiles).to_have_count(2)
            expect(p.locator('.desktop-window.app-settings')).to_be_hidden()
            first, second = [t.bounding_box() for t in tiles.all()]
            assert first['x'] + first['width'] <= second['x'] and abs(first['y'] - second['y']) < 2
            p.locator('.desktop-canvas').screenshot(path=str(out / 'overview.png'))
            p.keyboard.press('Escape')
            expect(overview).to_have_count(0)
            location('/Start')
            assert active.bounding_box() == original
            expect(p.locator('.desktop-window.app-files.hidden')).to_have_count(1)

            options()
            single.click()
            saved('overview_active_click', False)
            files.dblclick()
            location('/Start')
            expect(overview).to_have_count(0)
            files.click()
            expect(overview).to_have_count(0)

            options()
            single.click()
            saved('overview_active_click', True)
            files.dblclick()
            location('/Start')
            expect(overview).to_have_count(0)
            files.click()
            expect(tiles).to_have_count(2)
            tiles.first.focus()
            p.keyboard.press('ArrowRight')
            expect(tiles.nth(1)).to_be_focused()
            p.keyboard.press('Enter')
            location('/Start/Nested')

            expect(active).to_have_class(re.compile('maximized'))
            settings.click()
            settings.click()
            expect(tiles).to_have_count(1)
            overview.get_by_role('button', name='Exit window overview').click()
            expect(p.locator('.desktop-window.app-settings.active')).to_be_visible()
        print(out)


if __name__ == '__main__':
    main()
