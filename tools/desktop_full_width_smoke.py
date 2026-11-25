"""Restore an offset, desktop-wide Settings window and resize the real WebView."""
import json
import time

from desktop_fixture import Desktop, ROOT, expect


def main():
    out = ROOT / 'artifacts' / 'desktop-full-width' / str(time.time_ns())
    prefs = out / 'self-demo-server' / 'local-windows' / 'desktop.json'
    prefs.parent.mkdir(parents=True)
    saved = dict(x=275, y=284, width=1096, height=711,
                 reference=dict(width=1096, height=847))
    prefs.write_text(json.dumps(dict(version=1, windows={'settings': {'bounds': saved}})))
    config = dict(version=1, config=dict(servers=[], groups=[], trusted_hosts=[]))
    with Desktop(out, config, viewport=(1840, 1100)) as app:
        page = app.page
        page.emulate_media(reduced_motion='reduce')
        page.get_by_role('button', name='Try the local desktop', exact=True).click()
        page.get_by_role('button', name='Read-only mode', exact=True).click()
        app.application('Settings')
        settings = page.locator('.desktop-window.app-settings[data-positioned]')
        expect(settings).to_be_visible()
        layer = page.locator('.desktop-window-layer')
        chrome = round(1840 - layer.bounding_box()['width'])

        def resize(width):
            page.set_viewport_size(dict(width=width + chrome, height=1100))
            page.evaluate('() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done)))')
            return settings.bounding_box(), layer.bounding_box()

        sweep = [1440, 1371, 1358, 1240, 1114, 1097, 1096, 1095, 960]
        positions = {}
        for width in sweep + list(reversed(sweep)):
            rect, desktop = resize(width)
            x = rect['x'] - desktop['x']
            assert abs(desktop['width'] - width) <= 1, desktop
            assert abs(rect['width'] - min(width, saved['width'])) <= 1, rect
            assert abs(x - max(0, width - saved['width'])) <= 1, (width, x)
            if width in positions:
                assert abs(x - positions[width]) <= 1
            positions[width] = x
        assert json.loads(prefs.read_text())['windows']['settings']['bounds'] == saved

        # Manually moving a full-width window may clip it at the current size.
        # Shrinking or reversing a resize should gradually recover that edge.
        rect, desktop = resize(1096)
        title = settings.locator('.desktop-window-mark').bounding_box()
        x, y = title['x'] + title['width'] / 2, title['y'] + title['height'] / 2
        page.mouse.move(x, y)
        page.mouse.down()
        page.mouse.move(x + 90, y, steps=5)
        page.mouse.up()
        previous_clip = settings.bounding_box()['x'] - desktop['x']
        assert abs(previous_clip - 90) <= 2, previous_clip
        for width in (1086, 1076, 1086, 1096, 1156):
            rect, desktop = resize(width)
            clip = max(0, rect['x'] + rect['width'] - desktop['x'] - desktop['width'])
            assert clip < previous_clip, (width, clip, previous_clip)
            previous_clip = clip
        assert previous_clip <= 1
    print('Settings left offsets across desktop widths:', positions)


if __name__ == '__main__':
    main()
