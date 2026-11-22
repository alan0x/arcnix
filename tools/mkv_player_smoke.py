"""Play and seek Matroska in the real Win32 child surface over a delayed SFTP peer."""
import argparse
import ctypes
from ctypes import wintypes
import json
from pathlib import Path
import shutil
import subprocess
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, executable, expect
from libmpv_runtime import stage
from video_player_smoke import check_geometry, make_video, verify_player_scrollbars, watch_player_scrollbars


def native_window(pid):
    user = ctypes.windll.user32
    found = []
    callback = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user.FindWindowExW.restype = wintypes.HWND
    @callback
    def visit(hwnd, _):
        owner = wintypes.DWORD()
        user.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid:
            child = user.FindWindowExW(hwnd, None, 'Static', 'Arcnix video surface')
            if child:
                found.append(child)
        return True
    user.EnumWindows(visit, 0)
    return found[0] if found else None


def click_picture(surface):
    user = ctypes.windll.user32
    user.FindWindowExW.restype = wintypes.HWND
    user.FindWindowExW.argtypes = [wintypes.HWND, wintypes.HWND, wintypes.LPCWSTR, wintypes.LPCWSTR]
    user.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    picture = user.FindWindowExW(surface, None, 'mpv', None)
    assert picture
    bounds = wintypes.RECT()
    assert user.GetClientRect(picture, ctypes.byref(bounds))
    center = ((bounds.bottom // 2) << 16) | (bounds.right // 2)
    assert user.PostMessageW(picture, 0x0201, 1, center)  # WM_LBUTTONDOWN
    assert user.PostMessageW(picture, 0x0202, 0, center)  # WM_LBUTTONUP


def run(preview=False, binary=None, read_delay=.003):
    out = ROOT / 'artifacts' / ('manual-preview' if preview else 'mkv-smoke') / f'mkv-{time.time_ns()}'
    local = out / 'local'
    local.mkdir(parents=True)
    if binary is None:
        binary = executable()
        if preview:
            binary = Path(shutil.copy2(binary, out / binary.name))
        stage(binary.parent)
    with Peer(out/'ssh', io_delay=read_delay) as peer:
        mp4 = out/'source.mp4'
        make_video(out, mp4)
        mkv = peer.root/'Mandelbrot.MKV'
        subprocess.run(['ffmpeg','-loglevel','error','-i',str(mp4),'-map','0','-c:v','copy','-c:a','flac',str(mkv)], check=True)
        trace = peer.root.parent/'video-reads.txt'
        trace.touch()
        def reads():
            return [tuple(map(int,line.split())) for line in trace.read_text().splitlines() if line]
        (peer.root/'folder'/'hello.txt').write_text('SFTP listing')
        if not preview:
            (peer.root/'invalid.mkv').write_bytes(b'not a Matroska container')
        config = peer.config([peer.profile('MKV')], local)
        if preview:
            config['config']['settings']['language'] = 'SimplifiedChinese'
        with Desktop(out/'app', config, binary=binary, viewport=None) as app:
            page = app.page
            if preview:
                app.connect_media_preview()
            else:
                app.connect()
                app.workspace('Files')
            player = page.locator('.desktop-window.app-player')
            if not preview:
                watch_player_scrollbars(page)
                app.file('invalid.mkv').click(button='right')
                app.action('Open with'); app.action('Player')
                expect(player.locator('.remote-player-status')).to_contain_text('Video playback failed', timeout=20000)
                player.locator('.window-close').click()
                expect(player).to_have_count(0)
                app.workspace('Transfer files')
            app.file(mkv.name).dblclick()
            expect(player).to_be_visible()
            def media():
                status = player.locator('.remote-player-status').inner_text()
                if status and status not in ('Opening video…', '正在打开视频…'):
                    raise RuntimeError(status)
                return json.loads(player.locator('.mpv-state').inner_text())['media']
            eventually(lambda: media()['position'] > .5, timeout=25)
            hwnd = eventually(lambda: native_window(app.process.pid))
            user = ctypes.windll.user32
            user.GetParent.restype = wintypes.HWND
            user.MapWindowPoints.argtypes = [wintypes.HWND, wintypes.HWND, ctypes.c_void_p, wintypes.UINT]
            assert user.IsWindowVisible(hwnd)
            assert media()['duration'] > 150
            size = mkv.stat().st_size
            first = sum(length for _, length in reads())
            assert 0 < first < size/4
            if preview:
                player.locator('.player-play').click()
            else:
                click_picture(hwnd)
            eventually(lambda: media()['paused'])
            if preview:
                print(f'MKV preview ready: {app.process.pid} {out}', flush=True)
                return app.process.wait()
            at = media()['position']
            page.wait_for_timeout(600)
            assert media()['position'] == at
            def native_surface_matches():
                area = json.loads(player.locator('.browser-viewport-input').input_value())['area']
                rect = wintypes.RECT()
                user.GetWindowRect(hwnd, ctypes.byref(rect))
                # Convert screen coordinates to the app client coordinates used by the bridge.
                user.MapWindowPoints(None, user.GetParent(hwnd), ctypes.byref(rect), 2)
                return area and all(abs(getattr(rect, side) - value) <= 2 for side, value in area.items())
            check_geometry(page, player, native_surface_matches)
            previous = len(reads())
            def slide(selector, fraction):
                control = player.locator(selector)
                control.hover(position={'x': 6, 'y': 12})
                box = control.bounding_box()
                page.mouse.down()
                page.mouse.move(box['x'] + box['width']*fraction, box['y'] + box['height']/2, steps=5)
                page.mouse.up()
            slide('.player-seek', .80)
            eventually(lambda: media()['position'] > media()['duration'] * .75, timeout=15)
            eventually(lambda: any(size*.70 < offset < size*.90 for offset,_ in reads()[previous:]))
            player.get_by_role('button', name='Settings', exact=True).click()
            settings = page.get_by_role('dialog', name='Settings', exact=True)
            settings.get_by_role('combobox', name='Default volume').select_option(label='60%')
            settings.get_by_role('combobox', name='Playback speed').select_option(label='2×')
            settings.get_by_role('button', name='Close dialog', exact=True).click()
            eventually(lambda: media()['volume'] == 60)
            player.locator('.player-mute').click()
            eventually(lambda: media()['muted'])
            player.locator('.player-mute').click()
            eventually(lambda: not media()['muted'] and media()['volume'] == 60)
            slide('.player-volume', .36)
            eventually(lambda: 25 < media()['volume'] < 50)
            player.locator('.player-play').click()
            eventually(lambda: not media()['paused'])
            seek = media()['position']
            eventually(lambda: media()['position'] > seek + .3)
            # Code has its own native surface; the video must yield overlapping pixels.
            player.locator('.window-maximize').click()
            app.application('Code Editor')
            app.code()
            def clipped():
                region = ctypes.windll.gdi32.CreateRectRgn(0,0,1,1)
                try:
                    kind = user.GetWindowRgn(hwnd,region)
                    r = wintypes.RECT()
                    user.GetClientRect(hwnd,ctypes.byref(r))
                    return not user.IsWindowVisible(hwnd) or (kind > 0 and not ctypes.windll.gdi32.PtInRegion(region,r.right//2,r.bottom//2))
                finally:
                    ctypes.windll.gdi32.DeleteObject(region)
            eventually(clipped)
            app.application('Player')
            player.locator('.window-minimize').click()
            eventually(lambda: not user.IsWindowVisible(hwnd))
            app.workspace('Transfer files')
            started = time.monotonic()
            app.file('folder').dblclick()
            expect(app.file('hello.txt')).to_be_visible(timeout=2500)
            browse = time.monotonic() - started
            app.application('Player')
            eventually(lambda: user.IsWindowVisible(hwnd))
            page.get_by_role('button',name='Disconnect',exact=True).click()
            expect(player).to_have_count(0)
            eventually(lambda: not user.IsWindow(hwnd))
            eventually(lambda: '/' + mkv.name in trace.with_suffix('.closed').read_text().splitlines())
            settled = len(reads())
            time.sleep(.6)
            assert len(reads()) == settled and not list(local.iterdir())
            total = sum(n for _,n in reads())
            assert total < size/2
            print(json.dumps(dict(layout_frames=verify_player_scrollbars(page), size=size, first=first, total=total, seek=seek, browse_seconds=browse)))


if __name__ == '__main__':
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--preview', action='store_true')
    parser.add_argument('--binary', type=Path, help='Test an already packaged app with its bundled DLL')
    parser.add_argument('--read-delay', type=float, default=.003, help='Fixture latency per SFTP read in seconds (0–0.1)')
    args = parser.parse_args()
    run(args.preview, args.binary, args.read_delay)
