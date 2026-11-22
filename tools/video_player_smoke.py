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


def watch_player_scrollbars(page):
    # Sample layout during opening, buffering, seeking and window transitions.
    page.evaluate("""() => {
        const stats = window.playerLayout = {frames: 0, overflow: null};
        function sample() {
            const player = document.querySelector('.desktop-window.app-player');
            if (player?.getClientRects().length) {
                stats.frames++;
                for (const el of [player, ...player.querySelectorAll('*')]) {
                    if (!el.clientWidth || !el.clientHeight) continue;
                    const css = getComputedStyle(el);
                    if ((/auto|scroll/.test(css.overflowX) && el.scrollWidth > el.clientWidth + 1) ||
                        (/auto|scroll/.test(css.overflowY) && el.scrollHeight > el.clientHeight + 1)) {
                        stats.overflow ??= el.className || el.tagName;
                    }
                }
            }
            requestAnimationFrame(sample);
        }
        sample();
    }""")


def verify_player_scrollbars(page):
    stats = page.evaluate('window.playerLayout')
    assert stats['frames'] > 0 and stats['overflow'] is None, stats
    return stats['frames']


def run(preview, binary=None, read_delay=.003):
    out = ROOT / 'artifacts' / ('manual-preview' if preview else 'video-smoke') / f'video-{time.time_ns()}'
    out.mkdir(parents=True)
    binary = binary or executable()
    if preview:
        binary = shutil.copy2(binary, out / binary.name)
    local = out / 'local'
    local.mkdir()
    with Peer(out / 'ssh', io_delay=read_delay) as peer:
        media = peer.root / 'Mandelbrot.MP4'
        (peer.root / 'folder' / 'nested.txt').write_text('Remote directory listing')
        make_video(out, media)
        clips = peer.root / 'folder' / 'Clips'
        clips.mkdir()
        second_media = clips / 'Second.MP4'
        shutil.copyfile(media, second_media)
        renamed = clips / 'Renamed.MP4.BACKUP'
        renamed.hardlink_to(second_media)
        (clips / 'NoExtension').write_text('An unrelated file without a suffix')
        (peer.root / 'home/arcnix/Documents').mkdir(parents=True, exist_ok=True)
        settings = peer.root / 'home/arcnix/.arcnix/desktop.json'
        settings.parent.mkdir(parents=True, exist_ok=True)
        settings.write_text(json.dumps({'files': {
            'favorites': ['/folder/Clips'],
            'colors': {f'/{media.name}': 'red', '/folder/Clips': 'blue'},
        }}), encoding='utf-8')
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
                app.connect_media_preview()
            else:
                app.connect()
                app.workspace('Files')
            if not preview:
                watch_player_scrollbars(page)
            app.file(media.name).dblclick()
            video = page.locator('.remote-player video')
            expect(video).to_be_visible()
            eventually(lambda: video.evaluate('v => v.currentTime > .3'), timeout=25)
            assert video.evaluate('v => v.videoWidth === 640 && v.duration > 150 && v.getVideoPlaybackQuality().totalVideoFrames > 0')
            first = sum(length for _, length in reads())
            assert first < size / 4
            window = page.locator('.desktop-window.app-player')
            expect(window.locator('.desktop-window-title')).to_have_text(media.name)
            if preview:
                eventually(lambda: video.evaluate('v => v.currentTime > 2'))
                click_play_pause(page, session)
                eventually(lambda: video.evaluate('v => v.paused'))
                app.screenshot('player.png')
                print(f'Preview ready: {app.process.pid} {out}', flush=True)
                return app.process.wait()
            open_video = window.locator('.desktop-window-actions').get_by_role('button', name='Open video', exact=True)
            assert open_video.evaluate("button => button.nextElementSibling.matches('.window-minimize')")
            open_video.click()
            picker = page.get_by_role('dialog', name='Open video', exact=True)
            def picker_button(name):
                return picker.get_by_role('button', name=name, exact=True)
            current_folder = picker.locator('.breadcrumb.current')
            confirm = picker_button('Open')
            formats = picker_button('File types')
            root_video = picker_button(media.name)
            expect(root_video).to_be_visible()
            expect(picker.locator('xpath=ancestor::*[contains(@class, "desktop-modal-backdrop")]')).to_be_visible()
            expect(video).to_be_visible()
            expect(current_folder).to_have_attribute('aria-label', '/')
            expect(picker_button('Parent folder')).to_be_disabled()
            sidebar = picker.locator('.files-sidebar')
            toggle_sidebar = picker_button('Toggle file picker sidebar')
            expect(toggle_sidebar).to_have_attribute('aria-pressed', 'true')
            expect(sidebar).to_be_visible()
            expect(sidebar.get_by_text('Favorites', exact=True)).to_be_visible()
            expect(sidebar.get_by_text('Tags', exact=True)).to_be_visible()
            sidebar.get_by_role('button', name='Open favorite /folder/Clips', exact=True).click()
            expect(picker_button(second_media.name)).to_be_visible()
            sidebar.get_by_role('button', name='Red', exact=True).click()
            expect(root_video).to_be_visible()
            expect(picker_button(second_media.name)).to_have_count(0)
            sidebar.get_by_role('button', name='Trash', exact=True).click()
            expect(picker.locator('.breadcrumb.current')).to_have_text('Trash')
            expect(root_video).to_have_count(0)

            initial = picker.bounding_box()
            def resize(dx, dy):
                box = picker.bounding_box()
                x = box['x'] + box['width'] - 4
                y = box['y'] + box['height'] - 4
                page.mouse.move(x, y)
                page.mouse.down()
                page.mouse.move(x + dx, y + dy, steps=10)
                page.mouse.up()

            resize(100, 85)
            eventually(lambda: picker.bounding_box()['width'] > initial['width'] + 20
                       and picker.bounding_box()['height'] > initial['height'] + 20)
            resize(-350, -280)
            eventually(lambda: abs(picker.bounding_box()['width'] - 440) <= 2
                       and abs(picker.bounding_box()['height'] - 320) <= 2)
            resize(120, 120)
            sidebar.get_by_role('button', name='Documents', exact=True).click()
            expect(current_folder).to_have_attribute('aria-label', '/home/arcnix/Documents')
            sidebar.get_by_role('button', name='Filesystem', exact=True).click()
            expect(current_folder).to_have_attribute('aria-label', '/')
            toggle_sidebar.click()
            expect(sidebar).to_have_count(0)
            expect(confirm).to_be_disabled()
            root_video.click()
            expect(root_video).to_have_attribute('aria-pressed', 'true')
            expect(confirm).to_be_enabled()
            formats.click()
            assert picker.locator('.file-picker-formats').evaluate('''menu => {
                const card = menu.closest('.file-picker-dialog').getBoundingClientRect();
                const box = menu.getBoundingClientRect();
                return box.width >= card.width - 31 && box.left >= card.left && box.right <= card.right;
            }''')
            picker_button('*.webm').click()
            expect(root_video).to_have_count(0)
            expect(confirm).to_be_disabled()
            picker_button('folder').click()
            expect(current_folder).to_have_attribute('aria-label', '/folder')
            expect(formats).to_have_text('*.webm')
            expect(picker_button('nested.txt')).to_have_count(0)
            picker_button('Clips').click()
            expect(picker.get_by_text('No matching items', exact=True)).to_be_visible()
            formats.click()
            picker_button('*.mp4').click()
            picker_button(second_media.name).click()
            expect(confirm).to_be_enabled()
            expect(picker_button(renamed.name)).to_have_count(0)
            formats.click()
            picker_button('All files').click()
            expect(picker_button('NoExtension')).to_be_visible()
            expect(picker_button(renamed.name)).to_be_visible()
            expect(confirm).to_be_disabled()
            formats.click()
            custom = picker.get_by_role('textbox', name='Custom extensions', exact=True)
            custom.fill('*.MP4.BACKUP, .other')
            custom.press('Enter')
            expect(formats).to_have_text('Custom extensions')
            expect(picker_button('NoExtension')).to_have_count(0)
            expect(picker_button(second_media.name)).to_have_count(0)
            picker_button(renamed.name).click()
            expect(confirm).to_be_enabled()
            toggle_sidebar.click()
            expect(sidebar).to_be_visible()
            app.screenshot('video-picker.png')
            picker_button('/folder').click()
            expect(current_folder).to_have_attribute('aria-label', '/folder')
            expect(confirm).to_be_disabled()
            picker_button('/').press('Enter')
            expect(current_folder).to_have_attribute('aria-label', '/')
            expect(picker_button('folder')).to_be_visible()
            formats.click()
            picker_button('All supported videos').click()
            expect(root_video).to_be_visible()
            toggle_sidebar.click()
            page.keyboard.press('Escape')
            expect(picker).to_have_count(0)
            expect(window.locator('.desktop-window-title')).to_have_text(media.name)

            video.click()
            eventually(lambda: video.evaluate('v => v.paused'))
            expect(window.locator('.player-feedback')).to_have_attribute('data-symbol', 'play')
            expect(window.locator('.player-feedback')).to_be_visible()
            video.click()
            eventually(lambda: video.evaluate('v => !v.paused'))
            expect(window.locator('.player-feedback')).to_be_hidden()
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
            app.application('Player')
            expect(window).to_be_visible()
            assert video.evaluate('(v, previous) => v === previous', media_element)
            click_play_pause(page, session)
            eventually(lambda: video.evaluate('v => v.paused'))
            stopped = video.evaluate('v => v.currentTime')
            positioned = check_geometry(page, window)
            window.locator('.window-minimize').click()
            expect(window).to_be_hidden()
            app.application('Player')
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
            expect(page.locator('.desktop-dock .app-player')).to_have_count(0)
            # Already-issued SFTP reads may finish after the UI closes. Wait for
            # the peer to release this handle, then verify that reads stay stopped.
            eventually(lambda: '/' + media.name in trace.with_suffix('.closed').read_text().splitlines())
            settled = len(reads())
            page.wait_for_timeout(600)
            assert len(reads()) == settled
            # The same action must also work inside the server desktop's files.
            app.application('Files')
            page.locator('.files-sidebar').get_by_role('button', name='Filesystem', exact=True).click()
            app.file(media.name).dblclick()
            eventually(lambda: video.evaluate('v => v.currentTime > .3'), timeout=25)
            window.locator('.window-close').click()
            expect(video).to_have_count(0)
            app.file(media.name).click(button='right')
            app.action('Open with'); app.action('Player')
            eventually(lambda: video.evaluate('v => v.currentTime > .3'), timeout=25)
            restored = window.bounding_box()
            assert all(abs(restored[k] - v) <= 2 for k, v in positioned.items()), (restored, positioned)
            open_video.click()
            expect(formats).to_have_text('All supported videos')
            expect(sidebar).to_have_count(0)
            picker_button('folder').click()
            picker_button('Clips').click()
            formats.click()
            picker_button('All files').click()
            picker_button(renamed.name).press('Enter')
            expect(window.locator('.desktop-window-title')).to_have_text(media.name)
            expect(confirm).to_be_enabled()
            confirm.press('Enter')
            expect(picker).to_have_count(0)
            expect(window.locator('.desktop-window-title')).to_have_text(renamed.name)
            eventually(lambda: video.evaluate('v => v.currentTime > .3'), timeout=25)
            wrong_format = clips / 'Wrong format.WEBM'
            wrong_format.hardlink_to(second_media)
            open_video.click()
            expect(current_folder).to_have_attribute('aria-label', '/folder/Clips')
            picker_button(wrong_format.name).click()
            confirm.click()
            expect(picker).to_have_count(0)
            expect(window.locator('.desktop-window-title')).to_have_text(wrong_format.name)
            eventually(lambda: video.evaluate('v => v.currentSrc.endsWith(".mp4") && v.currentTime > .3'), timeout=25)
            app.workspace('Files')
            page.get_by_role('button', name='Disconnect', exact=True).click()
            expect(video).to_have_count(0)
            print(json.dumps(dict(layout_frames=verify_player_scrollbars(page), file_bytes=size, bytes_at_playback=first, bytes_after_seek=total,
                                  browse_seconds=round(browse, 3), seek_seconds=round(at, 2))))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--preview', action='store_true')
    parser.add_argument('--binary', type=Path, help='Test an already packaged app')
    parser.add_argument('--read-delay', type=float, default=.003, help='Fixture latency per SFTP read in seconds (0–0.1)')
    args = parser.parse_args()
    run(args.preview, args.binary, args.read_delay)
