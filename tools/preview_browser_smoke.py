"""Open two simultaneous local previews of the same Windows build over SSH."""
import ctypes
from ctypes import wintypes
import os
import time
from unittest.mock import patch

from desktop_browser_smoke import site
from desktop_fixture import Desktop, Peer, ROOT, eventually, executable, expect
from desktop_sync_smoke import browser, visit
from desktop_windows_smoke import native_window


def check_native_resize(app):
    window = native_window(app.process.pid)
    user = ctypes.windll.user32
    for command, maximized in ((3, True), (9, False)):
        user.ShowWindow(window, command)
        eventually(lambda: bool(user.IsZoomed(window)) == maximized)
        client = wintypes.RECT()
        assert user.GetClientRect(window, ctypes.byref(client))
        app.page.wait_for_function(
            '([w,h]) => Math.abs(w-innerWidth*devicePixelRatio)<=2 && Math.abs(h-innerHeight*devicePixelRatio)<=2',
            arg=[client.right, client.bottom],
        )


def main():
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
    run = f'browser-check-{time.time_ns()}'
    manual = ROOT / 'artifacts/manual-preview' / run
    panel = ROOT / 'artifacts/panel-preview' / run
    override = manual / 'inherited-user-data'
    with site() as server, Peer(manual / 'ssh', browser_port=server.server_port, browser_web=False) as peer:
        config = peer.config([peer.profile('Preview browser check')], manual)
        endpoint = f'http://browser-fixture.test:{server.server_port}'
        with patch.dict(os.environ, {'WEBVIEW2_USER_DATA_FOLDER': str(override)}), Desktop(
            manual / 'app', config, viewport=None,
        ) as first, Desktop(panel / 'desktop', config, binary=executable(), runtime=first.runtime) as second:
            first.connect()
            second.connect()
            first_view, second_view = browser(first), browser(second)
            visit(first_view, endpoint + '/first', 'Routed first page')
            visit(second_view, endpoint + '/second', 'Routed second page')
            expect(first_view.locator('.browser-tab.active .browser-tab-label')).to_have_text('Routed first page')
            first_view.locator('.window-close').click()
            expect(first_view).to_have_count(0)
            first.application('Code Editor')
            first.code()
            check_native_resize(first)
            first.page.locator('.desktop-window.app-code .window-close').click()
            expect(first.page.locator('.desktop-window.app-code')).to_have_count(0)
            check_native_resize(first)
            first.application('Player')
            player = first.page.locator('.desktop-window.app-player')
            expect(player.locator('.desktop-window-title')).to_have_text('Player')
            folder = peer.root / 'folder'
            (folder / 'clip.MP4').touch()
            (folder / 'clip.mKv').touch()
            (folder / 'clip.mp4.txt').touch()
            (folder / 'empty.dir').mkdir()
            player.get_by_role('button', name='Browse files…').click()
            picker = first.page.get_by_role('dialog', name='Open video', exact=True)
            expect(picker).to_be_visible()
            picker.get_by_role('button', name='folder', exact=True).click()
            for name in ('clip.MP4', 'clip.mKv'):
                expect(picker.get_by_role('button', name=name, exact=True)).to_be_visible()
            expect(picker.get_by_role('button', name='clip.mp4.txt', exact=True)).to_have_count(0)
            picker.get_by_role('button', name='empty.dir', exact=True).click()
            expect(picker.get_by_text('No playable videos in this folder.', exact=True)).to_be_visible()
            picker.get_by_role('button', name='Parent folder').click()
            expect(picker.get_by_role('button', name='clip.MP4', exact=True)).to_be_visible()
            picker.get_by_role('button', name='Cancel').click()
            expect(picker).to_have_count(0)
            expect(player.locator('.player-idle')).to_be_visible()
            visit(browser(first), endpoint + '/first', 'Routed first page')
            expect(second_view.locator('.browser-tab.active .browser-tab-label')).to_have_text('Routed second page')
            assert not override.exists()
    print('Concurrent manual and panel previews loaded independent pages and reopened the browser.')


if __name__ == '__main__':
    main()
