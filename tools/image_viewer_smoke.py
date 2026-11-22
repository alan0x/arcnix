"""Exercise the image viewer through the desktop, Files, and the SFTP file picker."""
from pathlib import Path
import tempfile
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def images(folder):
    folder.mkdir(exist_ok=True)
    (folder / '01 海岸.WEBP').write_bytes((ROOT / 'crates/arcnix/assets/fjord.webp').read_bytes())
    (folder / '02.svg').write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" width="2048" height="512">'
        '<rect width="2048" height="512" fill="teal"/></svg>', encoding='utf-8')
    (folder / '03.png').write_bytes(b'\x89PNG\r\n\x1a\ntruncated')
    with (folder / '04.png').open('wb') as oversize:
        oversize.truncate(8 * 1024 * 1024 + 1)
    (folder / 'notes.txt').write_text('Notes', encoding='utf-8')


def main():
    out = ROOT / 'artifacts/image-viewer' / str(time.time_ns())
    with Peer(out / 'peer') as peer:
        images(peer.root / 'Pictures')
        prefs = peer.root / 'home/arcnix/.arcnix/desktop.json'
        prefs.parent.mkdir(parents=True, exist_ok=True)
        prefs.write_text('{"version":1,"theme":"Dark"}', encoding='utf-8')
        profile = {**peer.profile('Pictures'), 'initial_remote_path': '/Pictures'}
        config = peer.config([profile], out, file_associations={'svg': {'default': 'text_editor'}})
        with Desktop(out / 'remote', config) as app:
            p = app.page
            app.connect()
            app.file('01 海岸.WEBP').dblclick()
            window = p.locator('.desktop-window.app-image.active')
            viewer = window.locator('.image-viewer')
            expect(viewer.locator('img')).to_have_attribute('alt', '01 海岸.WEBP')
            expect(viewer.locator('.viewer-status')).to_contain_text('1 / 4')
            expect(viewer.get_by_role('button', name='Previous image')).to_be_disabled()
            viewer.get_by_role('button', name='Next image').click()
            expect(viewer.locator('.viewer-dimensions')).to_have_text('2048 × 512')
            p.mouse.move(0, 0)
            for label in ('Previous image', 'Next image'):
                button = viewer.get_by_role('button', name=label, exact=True)
                glyph = button.locator('.glyph-chevron-right')
                assert glyph.evaluate("e => getComputedStyle(e).maskImage.includes('image/svg+xml')")
                assert glyph.bounding_box()['width'] >= 23.9
                assert button.bounding_box()['width'] >= 35.9
            viewer.locator('.viewer-toolbar').screenshot(path=str(out / 'toolbar.png'))
            viewer.get_by_role('button', name='Actual size').click()
            expect(viewer.locator('.viewer-scale')).to_have_text('100%')
            stage = viewer.locator('.viewer-stage')
            box = stage.bounding_box()
            cx, cy = box['x'] + box['width'] / 2, box['y'] + box['height'] / 2
            p.mouse.move(cx, cy)
            p.mouse.wheel(0, -100)
            expect(viewer.locator('.viewer-scale')).to_have_text('125%')
            viewer.get_by_role('button', name='Zoom out').click()
            expect(viewer.locator('.viewer-scale')).to_have_text('100%')
            p.mouse.move(cx, cy)
            p.mouse.down()
            p.mouse.move(cx + 70, cy)
            p.mouse.up()
            assert viewer.locator('img').bounding_box()['x'] + 1024 > cx + 60
            viewer.get_by_role('button', name='Rotate right').click()
            assert abs(viewer.locator('img').bounding_box()['width'] - 512) < 1
            p.keyboard.press('+')
            expect(viewer.locator('.viewer-scale')).to_have_text('125%')
            viewer.get_by_role('button', name='Fit to window').click()
            expect(viewer.locator('.viewer-fit')).to_have_attribute('aria-pressed', 'true')
            original = window.bounding_box()
            canvas = p.locator('.desktop-canvas')
            enter = viewer.get_by_role('button', name='View fullscreen in Desktop')
            leave = viewer.get_by_role('button', name='Exit image fullscreen')
            def same_bounds(a, b):
                return all(abs(a[k] - b[k]) < 1 for k in a)
            enter.click()
            expect(leave).to_have_attribute('aria-pressed', 'true')
            eventually(lambda: same_bounds(window.bounding_box(), canvas.bounding_box()))
            assert p.evaluate('document.fullscreenElement === null')
            expect(p.locator('.workbench-tabs')).to_be_visible()
            expect(p.locator('.desktop-dock')).to_be_hidden()
            p.keyboard.press('Escape')
            expect(enter).to_have_attribute('aria-pressed', 'false')
            eventually(lambda: same_bounds(window.bounding_box(), original))
            expect(p.locator('.desktop-dock')).to_be_visible()
            p.keyboard.press('f')
            leave.click()
            eventually(lambda: same_bounds(window.bounding_box(), original))
            p.keyboard.press('ArrowRight')
            expect(viewer.locator('.viewer-placeholder')).to_contain_text('Could not decode this image.')
            viewer.get_by_role('button', name='Next image').click()
            expect(viewer.locator('.viewer-placeholder')).to_contain_text('8 MiB')
            expect(viewer.get_by_role('button', name='Next image')).to_be_disabled()
            viewer.get_by_role('button', name='Open image').click()
            picker = p.get_by_role('dialog', name='Open image')
            expect(picker.get_by_role('button', name='notes.txt')).to_have_count(0)
            picker.get_by_role('button', name='01 海岸.WEBP', exact=True).click()
            picker.get_by_role('button', name='Open', exact=True).click()
            expect(viewer.locator('img')).to_have_attribute('alt', '01 海岸.WEBP')
            window.locator('.window-close').click()
            app.application('Image Viewer')
            expect(viewer.locator('.viewer-placeholder')).to_contain_text('Open an image to view it.')
            app.workspace('Transfer files')
            app.file('02.svg').dblclick()
            editor = p.get_by_role('dialog', name='Text Editor', exact=True)
            expect(editor.get_by_role('textbox', name='File contents')).to_have_value(
                (peer.root / 'Pictures/02.svg').read_text(encoding='utf-8'))
            editor.get_by_role('button', name='Close dialog').click()
            app.file('02.svg').click(button='right')
            menu = p.locator('.file-context-menu:visible')
            menu.get_by_role('button', name='Open with', exact=True).hover()
            menu.get_by_role('button', name='Image Viewer', exact=True).click()
            expect(viewer.locator('.viewer-dimensions')).to_have_text('2048 × 512')

    with tempfile.TemporaryDirectory(dir=Path.home()) as path:
        folder = Path(path)
        images(folder)
        cfg = dict(version=1, config=dict(servers=[], groups=[], trusted_hosts=[],
                                           settings=dict(language='English')))
        with Desktop(out / 'local', cfg) as app:
            p = app.page
            p.get_by_role('button', name='Try the local desktop', exact=True).click()
            p.get_by_role('button', name='Read-only mode', exact=True).click()
            expect(p.locator('.connection-state')).to_have_text('Connected locally')
            app.application('Files')
            app.file(folder.name).dblclick()
            app.file('02.svg').dblclick()
            expect(p.locator('.app-image.active .viewer-dimensions')).to_have_text('2048 × 512')


if __name__ == '__main__':
    main()
