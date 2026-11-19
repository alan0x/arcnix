"""Exercise WebView2's actual media controls over a latency-injected SFTP peer."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, executable, expect


def make_video(out, target):
    segment = out / 'zoom.mp4'
    ffmpeg = ['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error', '-y']
    subprocess.run(ffmpeg + [
        '-f', 'lavfi', '-i', 'mandelbrot=size=640x360:rate=24:end_pts=240',
        '-f', 'lavfi', '-i', 'anullsrc=r=44100:cl=stereo', '-t', '10',
        '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-g', '24', '-preset', 'veryfast',
        '-b:v', '3M', '-c:a', 'aac', str(segment),
    ], check=True)
    # Default MP4 muxing puts the index at EOF. A seek must skip the middle.
    subprocess.run(ffmpeg + ['-stream_loop', '15', '-i', str(segment), '-c', 'copy', str(target)], check=True)


def media_control(page, session, control):
    video = page.locator('.remote-player video')
    video.hover()
    # WebView2 puts its native controls inside a user-agent shadow root.
    root = session.send('DOM.getDocument', {'depth': 0})['root']['nodeId']
    node = session.send('DOM.querySelector', {'nodeId': root, 'selector': '.remote-player video'})['nodeId']
    tree = session.send('DOM.describeNode', {'nodeId': node, 'depth': -1, 'pierce': True})['node']
    def walk(item):
        attrs = item.get('attributes', [])
        if '-webkit-media-controls-' + control in attrs:
            return item['backendNodeId']
        for child in item.get('children', []) + item.get('shadowRoots', []):
            result = walk(child)
            if result:
                return result
    node = walk(tree)
    assert node, f'WebView2 {control} control missing'
    box = session.send('DOM.getBoxModel', {'backendNodeId': node})['model']['border']
    return box[0], (box[1] + box[5]) / 2, box[2] - box[0]

def click_play_pause(page, session):
    left, y, width = media_control(page, session, 'play-button')
    page.mouse.click(left + width / 2, y)


def check_geometry(page, window, native_surface_matches=None):
    def drag(handle, dx, dy):
        box = window.locator(handle).bounding_box()
        x, y = box['x'] + box['width'] / 2, box['y'] + box['height'] / 2
        page.mouse.move(x, y)
        page.mouse.down()
        page.mouse.move(x + dx, y + dy, steps=6)
        page.mouse.up()

    def near(a, b):
        return all(abs(a[k] - value) <= 2 for k, value in b.items())

    initial = window.bounding_box()
    drag('.edge-se', -100, -75)
    eventually(lambda: near(window.bounding_box(), dict(width=initial['width'] - 100, height=initial['height'] - 75)))
    drag('.desktop-window-title', 56, 37)
    eventually(lambda: near(window.bounding_box(), dict(x=initial['x'] + 56, y=initial['y'] + 37)))
    positioned = window.bounding_box()
    if native_surface_matches:
        eventually(native_surface_matches)
    def maximized_inside_desktop():
        bounds = window.bounding_box()
        canvas = page.locator('.desktop-canvas').bounding_box()
        dock = page.locator('.desktop-dock-zone').bounding_box()
        return (near(bounds, {key: canvas[key] for key in ('x', 'y', 'width')})
                and bounds['y'] + bounds['height'] <= dock['y'] + 1
                and page.evaluate('!document.fullscreenElement && !document.pictureInPictureElement'))

    window.locator('.window-maximize').click()
    eventually(maximized_inside_desktop)
    if native_surface_matches:
        eventually(native_surface_matches)
        window.locator('.window-maximize').click()
        eventually(lambda: near(window.bounding_box(), positioned))
        eventually(native_surface_matches)
        return positioned
    screen = window.locator('.remote-player-screen').bounding_box()
    assert near(window.locator('video').bounding_box(), screen)
    window.locator('.window-maximize').click()
    eventually(lambda: near(window.bounding_box(), positioned))
    video = window.locator('video')
    assert video.evaluate("v => v.controlsList.contains('nofullscreen') && v.disablePictureInPicture")
    video.dblclick(position={'x': 120, 'y': 70})
    eventually(maximized_inside_desktop)
    video.press('f')
    eventually(lambda: near(window.bounding_box(), positioned))
    return positioned


def run(preview, binary=None):
    out = ROOT / 'artifacts' / ('manual-preview' if preview else 'video-smoke') / f'video-{time.time_ns()}'
    out.mkdir(parents=True)
    binary = binary or executable()
    if preview:
        binary = shutil.copy2(binary, out / binary.name)
    local = out / 'local'
    local.mkdir()
    with Peer(out / 'ssh', io_delay=.003) as peer:
        media = peer.root / 'Mandelbrot.MP4'
        (peer.root / 'folder' / 'nested.txt').write_text('Remote directory listing')
        make_video(out, media)
        trace = peer.root.parent / 'video-reads.txt'
        trace.touch()
        def reads():
            return [tuple(map(int, line.split())) for line in trace.read_text().splitlines() if line]
        size = media.stat().st_size
        config = peer.config([peer.profile('视频播放演示' if preview else 'Video preview')], local)
        if preview:
            config['config']['settings']['language'] = 'SimplifiedChinese'
        with Desktop(out / 'app', config, binary=binary, viewport=None) as app:
            page = app.page
            session = page.context.new_cdp_session(page)
            if preview:
                page.locator('.host-select').click()
                page.locator('.overview-transfer').click()
                page.get_by_role('dialog').get_by_role('button', name='连接', exact=True).click()
                expect(page.locator('.connection-state')).to_have_text('SSH 已连接')
            else:
                app.connect()
                app.workspace('Files')
            app.file(media.name).dblclick()
            video = page.locator('.remote-player video')
            expect(video).to_be_visible()
            eventually(lambda: video.evaluate('v => v.currentTime > .3'), timeout=25)
            assert video.evaluate('v => v.videoWidth === 640 && v.duration > 150 && v.getVideoPlaybackQuality().totalVideoFrames > 0')
            first = sum(length for _, length in reads())
            assert first < size / 4
            window = page.locator('.desktop-window.app-video')
            expect(window.locator('.desktop-window-title')).to_have_text(media.name)
            if preview:
                eventually(lambda: video.evaluate('v => v.currentTime > 2'))
                click_play_pause(page, session)
                eventually(lambda: video.evaluate('v => v.paused'))
                app.screenshot('player.png')
                print(f'Preview ready: {app.process.pid} {out}', flush=True)
                return app.process.wait()
            # Minimize the playing video, use SFTP, then return through the Dock.
            media_element = video.element_handle()
            playing_at = video.evaluate('v => v.currentTime')
            window.locator('.window-minimize').click()
            expect(window).to_be_hidden()
            app.workspace('Transfer files')
            started = time.monotonic()
            app.file('folder').click(button='right')
            app.action('Open folder')
            expect(app.file('nested.txt')).to_be_visible(timeout=2500)
            browse = time.monotonic() - started
            eventually(lambda: media_element.evaluate('(v,t) => !v.paused && v.currentTime > t + .25', playing_at))
            app.application('Video player')
            expect(window).to_be_visible()
            assert video.evaluate('(v, previous) => v === previous', media_element)
            click_play_pause(page, session)
            eventually(lambda: video.evaluate('v => v.paused'))
            stopped = video.evaluate('v => v.currentTime')
            positioned = check_geometry(page, window)
            window.locator('.window-minimize').click()
            expect(window).to_be_hidden()
            app.application('Video player')
            page.wait_for_timeout(800)
            assert video.evaluate('v => v.currentTime') == stopped
            before = len(reads())
            left, y, width = media_control(page, session, 'timeline')
            assert width > 80
            page.mouse.move(left + 5, y)
            page.mouse.down()
            page.mouse.move(left + width * .80, y, steps=5)
            page.mouse.up()
            eventually(lambda: video.evaluate('v => v.currentTime > v.duration * .7 && !v.seeking && v.readyState >= 2'), timeout=20)
            at = video.evaluate('v => v.currentTime')
            assert any(size * .65 < offset < size * .9 for offset, _ in reads()[before:])
            click_play_pause(page, session)
            eventually(lambda: video.evaluate('(v, at) => v.currentTime > at + .5', at))
            total = sum(n for _, n in reads())
            assert total < size / 2 and not list(local.iterdir())
            window.locator('.window-close').click()
            expect(video).to_have_count(0)
            expect(page.locator('.desktop-dock .app-video')).to_have_count(0)
            page.wait_for_timeout(500)
            settled = len(reads())
            page.wait_for_timeout(600)
            assert len(reads()) == settled
            # The same action must also work inside the server desktop's explorer.
            app.application('File explorer')
            page.locator('.explorer-sidebar').get_by_role('button', name='Filesystem', exact=True).click()
            app.file(media.name).dblclick()
            eventually(lambda: video.evaluate('v => v.currentTime > .3'), timeout=25)
            window.locator('.window-close').click()
            expect(video).to_have_count(0)
            app.file(media.name).click(button='right')
            app.action('Play')
            eventually(lambda: video.evaluate('v => v.currentTime > .3'), timeout=25)
            restored = window.bounding_box()
            assert all(abs(restored[k] - v) <= 2 for k, v in positioned.items()), (restored, positioned)
            app.workspace('Files')
            page.get_by_role('button', name='Disconnect', exact=True).click()
            expect(video).to_have_count(0)
            print(json.dumps(dict(file_bytes=size, bytes_at_playback=first, bytes_after_seek=total,
                                  browse_seconds=round(browse, 3), seek_seconds=round(at, 2))))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--preview', action='store_true')
    parser.add_argument('--binary', type=Path, help='Test an already packaged app')
    args = parser.parse_args()
    run(args.preview, args.binary)
