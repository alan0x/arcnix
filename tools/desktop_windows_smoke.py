"""Window geometry and Explorer preferences through the real WebView and SSH peer."""
import re
import ctypes
from ctypes import wintypes
import json
import time
from desktop_fixture import Desktop, Peer, ROOT, expect, eventually


def native_window(pid):
    user32 = ctypes.windll.user32
    handles = []
    callback = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    @callback
    def collect(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(wintypes.HWND(hwnd)):
            title = ctypes.create_unicode_buffer(128)
            user32.GetWindowTextW(wintypes.HWND(hwnd), title, len(title))
            if title.value.startswith('Arcnix'):
                handles.append(hwnd)
        return True

    user32.EnumWindows(collect, 0)
    assert len(handles) == 1, handles
    return wintypes.HWND(handles[0])


def native_bounds(hwnd):
    rect = wintypes.RECT()
    assert ctypes.windll.user32.GetWindowRect(hwnd, ctypes.byref(rect))
    return tuple(getattr(rect, edge) for edge in ('left', 'top', 'right', 'bottom'))


def monitor_bounds(hwnd):
    class MonitorInfo(ctypes.Structure):
        _fields_ = [('size', wintypes.DWORD), ('monitor', wintypes.RECT),
                    ('work', wintypes.RECT), ('flags', wintypes.DWORD)]

    user32 = ctypes.windll.user32
    user32.MonitorFromWindow.restype = wintypes.HANDLE
    monitor = wintypes.HANDLE(user32.MonitorFromWindow(hwnd, 2))
    info = MonitorInfo(ctypes.sizeof(MonitorInfo))
    assert user32.GetMonitorInfoW(monitor, ctypes.byref(info))
    return tuple(getattr(info.monitor, edge) for edge in ('left', 'top', 'right', 'bottom'))


def main():
    out = ROOT / 'artifacts/desktop-windows' / f'run-{time.time_ns()}'
    out.mkdir(parents=True)
    local = out / 'local'
    local.mkdir()
    with Peer(out / 'peer') as peer:
        home = peer.root / 'home/arcnix'
        desktop_path = '/home/arcnix/Desktop'
        for directory in ('Desktop', 'Documents', 'Downloads'):
            (home / directory).mkdir(parents=True, exist_ok=True)
        (peer.root / 'Work space 中文').mkdir()
        (home / 'Desktop/desktop-file.txt').write_text('desktop file')
        remote = home / '.arcnix/desktop.json'
        remote.parent.mkdir(parents=True)
        disk_layout = dict(x=38, y=51, width=720, height=570)
        remote.write_text(json.dumps(dict(version=1, theme='Dark', windows={'disks': {'bounds': disk_layout}})))
        with Desktop(out / 'desktop', peer.config([peer.profile('Window preferences')], local)) as app:
            p = app.page
            errors = []
            p.on('pageerror', lambda error: errors.append(str(error)))

            def prefs():
                return json.loads(remote.read_text())

            def settled():
                eventually(lambda: not p.locator('html').evaluate('el=>el.classList.contains("desktop-motion")'))

            def bounds():
                return p.locator('.desktop-window.active').bounding_box()

            def drag(locator, dx, dy, cancel=False):
                box = locator.bounding_box()
                x, y = box['x'] + box['width'] / 2, box['y'] + box['height'] / 2
                p.mouse.move(x, y)
                hit = p.evaluate('([x,y])=>document.elementFromPoint(x,y)?.className',[x,y])
                if 'edge-' in locator.evaluate('node=>node.className'):
                    assert 'desktop-resize-handle' in hit, (box,hit,bounds())
                p.mouse.down()
                p.mouse.move(x + dx, y + dy, steps=6)
                if cancel:
                    p.keyboard.press('Escape')
                p.mouse.up()

            def same(a, b):
                return all(abs(a[key] - b[key]) <= 2 for key in ('x', 'y', 'width', 'height'))

            app.connect()
            app.application('File explorer')
            settled()
            expect(p.locator('.desktop-resize-handle:visible')).to_have_count(8)
            original = bounds()
            drag(p.locator('.desktop-window.active .edge-se'), -250, -170)
            assert abs(bounds()['width'] - (original['width'] - 250)) <= 2, (bounds(),original,errors)
            assert abs(bounds()['height'] - (original['height'] - 170)) <= 2
            shrunk = bounds()
            titlebar = p.locator('.desktop-window.active .desktop-window-titlebar')
            expect(titlebar).to_have_css('cursor', 'default')
            drag(titlebar.locator('.desktop-window-mark'), 90, 45)
            moved = bounds()
            assert abs(moved['x'] - shrunk['x'] - 90) <= 2
            assert abs(moved['y'] - shrunk['y'] - 45) <= 2
            drag(p.locator('.desktop-window.active .edge-nw'), -14, -12)
            positioned = bounds()
            assert abs(positioned['x'] - moved['x'] + 14) <= 2
            assert abs(positioned['y'] - moved['y'] + 12) <= 2
            assert abs(positioned['width'] - moved['width'] - 14) <= 2
            drag(p.locator('.desktop-window.active .edge-e'), 50, 0, cancel=True)
            assert same(positioned, bounds())
            eventually(lambda: prefs().get('windows', {}).get('explorer', {}).get('bounds'))
            eventually(lambda: abs(prefs()['windows']['explorer']['bounds']['width'] - positioned['width']) <= 2)
            geometry = prefs()['windows']['explorer']['bounds']

            p.locator('.desktop-window.active .window-maximize').click()
            settled()
            expect(p.locator('.desktop-window.active')).to_have_class(re.compile(r'\bmaximized\b'))
            expect(p.locator('.desktop-resize-handle:visible')).to_have_count(0)
            app.application('Monitor')
            settled()
            expect(p.locator('.desktop-window.active')).not_to_have_class(re.compile(r'\bmaximized\b'))
            monitor_before = bounds()
            assert abs(monitor_before['width'] - disk_layout['width']) <= 2, monitor_before
            assert abs(monitor_before['height'] - disk_layout['height']) <= 2, monitor_before
            drag(p.locator('.desktop-window.active .edge-s'), 0, -65)
            assert abs(bounds()['height']-monitor_before['height']+65)<=2, (bounds(),monitor_before)
            eventually(lambda: prefs()['windows'].get('monitor', {}).get('bounds', {}).get('height') == 505)
            app.application('File explorer')
            settled()
            expect(p.locator('.desktop-window.active')).to_have_class(re.compile(r'\bmaximized\b'))
            p.locator('.desktop-window.active .window-maximize').click()
            settled()
            assert same(positioned, bounds())
            p.set_viewport_size(dict(width=760, height=650))
            eventually(lambda: bounds()['width'] < positioned['width'])
            assert bounds()['x'] >= p.locator('.desktop-canvas').bounding_box()['x'] - 1
            assert prefs()['windows']['explorer']['bounds'] == geometry
            p.set_viewport_size(dict(width=1440, height=980))
            eventually(lambda: same(positioned, bounds()))

            hwnd = native_window(app.process.pid)
            native_before = native_bounds(hwnd)
            screen = monitor_bounds(hwnd)
            assert native_before != screen
            p.locator('.desktop-fullscreen-toggle').click()
            eventually(lambda: native_bounds(hwnd) == screen)
            p.keyboard.press('Escape')
            eventually(lambda: native_bounds(hwnd) == native_before)
            assert same(positioned, bounds())
            assert prefs()['windows']['explorer']['bounds'] == geometry

            sidebar = p.locator('.explorer-sidebar')
            sidebar.get_by_role('button', name='Desktop', exact=True).click()
            expect(app.file('desktop-file.txt')).to_be_visible()
            pane = p.locator('.explorer-pane')
            address = p.locator('.explorer-titlebar .pathbar')
            expect(p.locator('.explorer-pane .pathbar, .explorer-titlebar .path-edit')).to_have_count(0)
            root_crumb = address.locator('.breadcrumb[aria-label="/"]')
            root_crumb.dblclick()
            expect(address.locator('.path-input')).to_be_focused()
            expect(address.locator('.path-input')).to_have_value(desktop_path)
            p.keyboard.press('Escape')
            expect(address.locator('.breadcrumb.current')).to_have_text('Desktop')
            root_crumb.focus()
            p.keyboard.press('Control+l')
            expect(address.locator('.path-input')).to_be_focused()
            p.keyboard.press('Escape')
            star = address.locator('.path-location + .explorer-favorite')
            star.click()
            expect(star).to_have_attribute('aria-pressed', 'true')
            expect(sidebar.get_by_role('button', name=f'Open favorite {desktop_path}', exact=True)).to_be_visible()
            favorites = sidebar.locator('.explorer-favorites')
            folder_favorite = favorites.locator(f'.explorer-favorite-row[aria-label="{desktop_path}"]')
            file_path = f'{desktop_path}/desktop-file.txt'
            file_row = pane.locator(f'.file-row[aria-label="{file_path}"]')
            file_row.drag_to(folder_favorite, target_position={'x': 40, 'y': 3})
            eventually(lambda: prefs()['explorer']['favorites'] == [file_path, desktop_path])
            file_favorite = favorites.locator(f'.explorer-favorite-row[aria-label="{file_path}"]')
            folder_favorite.drag_to(file_favorite, target_position={'x': 40, 'y': 3})
            eventually(lambda: prefs()['explorer']['favorites'] == [desktop_path, file_path])
            file_row.drag_to(folder_favorite, target_position={'x': 40, 'y': 3})
            eventually(lambda: prefs()['explorer']['favorites'] == [file_path, desktop_path])

            file_icon = file_row.locator('.file-entry-icon')
            file_color = file_icon.evaluate('icon => getComputedStyle(icon).color')
            app.file('desktop-file.txt').click(button='right')
            app.action('File color…')
            p.get_by_role('dialog', name='File color', exact=True).get_by_role('button', name='Blue', exact=True).click()
            expect(file_row.locator('.folder-color-dot')).to_have_css('background-color', 'rgb(91, 153, 236)')
            expect(file_icon).to_have_css('color', file_color)
            root_crumb.click()
            expect(address.locator('.breadcrumb.current')).to_have_attribute('aria-label', '/')
            sidebar.get_by_role('button', name='Home', exact=True).click()
            app.file('Desktop').click(button='right')
            app.action('Folder color…')
            color = p.get_by_role('dialog', name='Folder color', exact=True)
            color.get_by_role('button', name='Purple', exact=True).click()
            expect(folder_favorite.locator('.tag-purple')).to_be_visible()
            folder_row = pane.locator(f'.file-row[aria-label="{desktop_path}"]')
            folder_icon = folder_row.locator('.file-entry-icon.glyph-folder')
            expect(folder_row.locator('.folder-color-dot')).to_have_css('background-color', 'rgb(165, 128, 235)')
            expect(folder_icon).to_have_css('color', 'rgb(165, 128, 235)')
            address.locator('.view-grid').click()
            expect(pane).to_have_class(re.compile(r'\bicons-view\b'))
            expect(folder_icon).to_have_css('color', 'rgb(165, 128, 235)')
            address.locator('.view-list').click()
            sidebar.get_by_role('button', name='Blue', exact=True).click()
            tags = p.locator('.explorer-tags-view')
            expect(pane).to_be_hidden()
            expect(tags.locator('.explorer-tag-item')).to_have_count(1)
            tags.get_by_role('button', name=file_path, exact=True).click()
            expect(p.locator('.remote-editor-content')).to_have_value('desktop file')
            p.locator('.desktop-window.active .window-close').click()
            app.application('File explorer')
            sidebar.get_by_role('button', name='All Tags', exact=True).click()
            expect(tags.locator('.explorer-tag-item')).to_have_count(2)
            tags.get_by_role('button', name=desktop_path, exact=True).click()
            expect(app.file('desktop-file.txt')).to_be_visible()
            toggle = p.locator('.explorer-sidebar-toggle')
            toggle.click()
            expect(sidebar).to_be_hidden()
            eventually(lambda: prefs().get('explorer', {}).get('sidebar_hidden') is True)

            p.locator('.desktop-window.active .window-close').click()
            settled()
            app.application('File explorer')
            settled()
            assert same(positioned, bounds())
            app.stop()
            app.launch()
            p = app.page
            p.on('pageerror', lambda error: errors.append(str(error)))
            app.connect()
            app.application('File explorer')
            settled()
            eventually(lambda: same(positioned, bounds()))
            expect(p.locator('.explorer-sidebar')).to_be_hidden()
            p.locator('.explorer-sidebar-toggle').click()
            sidebar = p.locator('.explorer-sidebar')
            expect(sidebar.locator(f'.explorer-favorite-row[aria-label="{desktop_path}"] .tag-purple')).to_be_visible()
            expect(sidebar.locator('.explorer-favorite-row').first).to_have_attribute('aria-label', file_path)
            sidebar.get_by_role('button', name=f'Open favorite {desktop_path}', exact=True).click()
            expect(app.file('desktop-file.txt')).to_be_visible()
            app.screenshot('restored-explorer.png')
            sidebar.get_by_role('button', name=f'Remove favorite {desktop_path}', exact=True).click()
            sidebar.get_by_role('button', name=f'Remove favorite {file_path}', exact=True).click()
            eventually(lambda: prefs()['explorer']['favorites'] == [])
            assert (home / 'Desktop/desktop-file.txt').is_file()
            assert prefs()['explorer']['colors'][desktop_path] == 'purple'
            assert prefs()['explorer']['colors'][file_path] == 'blue'
            app.settings('General')
            p.get_by_role('button', name='简体中文', exact=True).click()
            app.servers()
            expect(p.locator('.explorer-sidebar').get_by_role('button', name='下载目录', exact=True)).to_be_visible()
            assert not errors, errors
    print(json.dumps(dict(output=str(out), checks=[
        'Eight resize handles, captured titlebar movement and Escape cancellation',
        'Independent window geometry across apps, reopen, resize, maximize and client restart',
        'Native monitor fullscreen with Escape restoring the original OS window',
        'Breadcrumb double-click editing and single-click navigation',
        'File and folder favorites, drag ordering, tag filters and sidebar persistence over SSH',
    ]), indent=2))


if __name__ == '__main__':
    main()
