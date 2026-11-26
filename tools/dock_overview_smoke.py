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
                            const root = document.documentElement;
                            if (!document.querySelector('.window-overview')) {
                                const active = document.querySelector('.desktop-window.active:not(.hidden)');
                                const name = active.style.viewTransitionName;
                                const style = part => getComputedStyle(root, `::view-transition-${part}(${name})`);
                                motion.restored = {
                                    group: motion.snapshots.includes(`::view-transition-group(${name})`),
                                    oldOpacity: style('old').opacity,
                                    newOpacity: style('new').opacity,
                                    newAnimation: style('new').animationName,
                                    blend: style('new').mixBlendMode,
                                };
                                if (active.classList.contains('app-monitor')) {
                                    const animations = document.getAnimations().filter(a =>
                                        a.effect?.pseudoElement?.startsWith('::view-transition-'));
                                    const backdrop = part => getComputedStyle(root,
                                        `::view-transition-${part}(arcnix-desktop-overview)`);
                                    const behind = [...active.parentElement.children].filter(el =>
                                        el !== active && el.matches('.desktop-window:not(.hidden)'));
                                    motion.foregroundZ = style('group').zIndex;
                                    motion.backgrounds = behind.map(el => getComputedStyle(el).viewTransitionName);
                                    motion.layers = [0, 110, 219, 280, 340].map(time => {
                                        for (const a of animations) { a.pause(); a.currentTime = time; }
                                        return [Number(style('new').opacity), Number(backdrop('old').opacity),
                                                Number(backdrop('new').opacity)];
                                    });
                                    for (const a of animations) { a.currentTime = 0; a.play(); }
                                }
                            }
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
                if 'restored' in motion:
                    assert_opaque(motion)
                else:
                    assert sum('::view-transition-new(arcnix-overview-files-window-' in part for part in snapshots) >= windows, motion

            def assert_opaque(motion):
                assert motion['restored'] == dict(group=True, oldOpacity='0', newOpacity='1',
                                                  newAnimation='none', blend='normal'), motion

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

            monitor = dock.get_by_role('button', name='Monitor', exact=True)
            monitor.click()
            monitor_window = p.locator('.desktop-window.app-monitor:not(.hidden)')
            expect(monitor_window).to_be_visible()
            eventually(lambda: not p.locator('html').evaluate("el => el.classList.contains('desktop-motion')"))
            normal = monitor_window.bounding_box()
            monitor.click()
            expect(tiles).to_have_count(1)
            eventually(lambda: not p.locator('html').evaluate("el => el.classList.contains('desktop-motion')"))
            thumbnail = monitor_window.bounding_box()
            index = p.evaluate('overviewMotions.length')
            monitor.click()
            eventually(lambda: p.evaluate('(i) => overviewMotions[i]?.finished', index))
            restored = p.evaluate('(i) => overviewMotions[i]', index)
            assert_opaque(restored)
            assert restored['foregroundZ'] == '1', restored
            assert restored['backgrounds'] and set(restored['backgrounds']) == {'none'}, restored
            assert restored['layers'][:3] == [[1, 1, 0]] * 3, restored
            foreground, old_layer, new_layer = restored['layers'][3]
            assert foreground == 1 and 0 < old_layer < 1 and 0 < new_layer < 1, restored
            assert restored['layers'][4] == [1, 0, 1], restored
            assert monitor_window.bounding_box() == normal and normal != thumbnail
            settings.click()

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
                    if (this.matches('.desktop-window-layer') && !this.matches('.window-overview'))
                        window.backdropTiming = [options.delay, options.duration];
                    if (this.matches('.desktop-window') && options.delay && frames[0].opacity === 0)
                        window.backgroundTiming = [options.delay, options.duration, options.fill];
                    if (this.matches('.app-settings.desktop-window') && !this.closest('.window-overview'))
                        window.restoreFrames = frames;
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
            frames = p.evaluate('window.restoreFrames')
            assert all('opacity' not in frame for frame in frames), frames
            assert 'translate(' in frames[0]['transform'] and 'scale(' in frames[0]['transform'], frames
            assert frames[-1]['transform'] == 'none', frames
            assert p.evaluate('window.backdropTiming') == [220, 120]
            assert p.evaluate('window.backgroundTiming') == [220, 120, 'backwards']
        print(out)


if __name__ == '__main__':
    main()
