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
            p.emulate_media(reduced_motion='no-preference')
            app.connect()
            app.workspace('Desktop')
            p.evaluate('''() => {
                window.overviewMotions = [];
                const start = document.startViewTransition.bind(document);
                document.startViewTransition = callback => {
                    const transition = start(callback);
                    if (document.documentElement.classList.contains('desktop-motion-overview')) {
                        const motion = {};
                        overviewMotions.push(motion);
                        transition.ready.then(() => {
                            motion.snapshots = [...new Set(document.getAnimations().map(animation =>
                                animation.effect?.pseudoElement || ''))];
                        }).catch(() => {});
                        transition.finished.catch(() => {}).then(() => motion.finished = true);
                    }
                    return transition;
                };
            }''')
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
            def animated(index, windows):
                eventually(lambda: p.evaluate('(i) => overviewMotions[i]?.finished', index))
                motion = p.evaluate('(i) => overviewMotions[i]', index)
                snapshots = motion.get('snapshots', [])
                assert any('arcnix-desktop-overview)' in part for part in snapshots), motion
                assert sum('::view-transition-new(arcnix-overview-files-window-' in part for part in snapshots) >= windows, motion

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
            animated(0, 2)
            expect(p.locator('.desktop-window.app-settings')).to_be_hidden()
            first, second = [t.bounding_box() for t in tiles.all()]
            assert first['x'] + first['width'] <= second['x'] and abs(first['y'] - second['y']) < 2
            p.locator('.desktop-canvas').screenshot(path=str(out / 'overview.png'))

            expect(overview).to_have_css('overflow-x', 'hidden')
            assert overview.evaluate('el => el.clientHeight === el.offsetHeight')
            normal_viewport = p.viewport_size
            p.set_viewport_size(dict(width=680, height=385))
            eventually(lambda: overview.evaluate('el => el.scrollHeight > el.clientHeight'))
            overview.hover()
            p.mouse.wheel(180, 240)
            eventually(lambda: overview.evaluate('el => el.scrollTop > 0'))
            assert overview.evaluate('el => el.scrollLeft === 0 && el.clientHeight === el.offsetHeight')
            p.set_viewport_size(normal_viewport)
            eventually(lambda: overview.evaluate('el => el.scrollTop === 0'))

            p.keyboard.press('Escape')
            expect(overview).to_have_count(0)
            animated(1, 1)
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

            p.emulate_media(reduced_motion='reduce')
            before = p.evaluate('overviewMotions.length')
            settings.click()
            expect(tiles).to_have_count(1)
            settings.click()
            expect(overview).to_have_count(0)
            assert p.evaluate('overviewMotions.length') == before

            p.emulate_media(reduced_motion='no-preference')
            p.evaluate('''() => {
                document.startViewTransition = undefined;
                window.overviewFallback = [];
                const animate = Element.prototype.animate;
                Element.prototype.animate = function(frames, options) {
                    if (this.matches('.window-overview,.overview-tile,.app-settings.desktop-window'))
                        overviewFallback.push(this.classList.contains('overview-tile') ? 'tile' :
                            this.classList.contains('window-overview') ? 'layer' : 'window');
                    return animate.call(this, frames, options);
                };
            }''')
            settings.click()
            expect(tiles).to_have_count(1)
            eventually(lambda: p.evaluate('overviewFallback.includes("layer") && overviewFallback.includes("tile")'))
            settings.click()
            expect(overview).to_have_count(0)
            eventually(lambda: p.evaluate('overviewFallback.includes("window")'))
        print(out)


if __name__ == '__main__':
    main()
