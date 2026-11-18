"""Verify independent floating windows, stacking and focus in the native desktop."""
import time

from desktop_fixture import Desktop, Peer, ROOT, expect


def main():
    out = ROOT / 'artifacts' / 'desktop-stack' / f'run-{time.time_ns()}'
    out.mkdir(parents=True)
    with Peer(out / 'peer') as peer, Desktop(
        out / 'desktop', peer.config([peer.profile('Desktop stack')], out),
    ) as app:
        page = app.page
        errors = []
        page.on('pageerror', lambda error: errors.append(str(error)))
        page.emulate_media(reduced_motion='reduce')
        app.connect()
        app.application('Terminal')
        terminal = page.locator('.desktop-window.app-terminal')
        monitor = page.locator('.desktop-window.app-resources')
        settings = page.locator('.desktop-window.app-settings')
        windows = page.locator('.desktop-window:visible')
        front = page.locator('.desktop-window.active')
        output = terminal.locator('.xterm-accessibility-tree')
        expect(terminal.locator('.terminal-status')).to_have_text('Interactive SSH')
        terminal.locator('.xterm-helper-textarea').focus()
        page.keyboard.type('echo WINDOW_SESSION_KEPT')
        page.keyboard.press('Enter')
        expect(output).to_contain_text('WINDOW_SESSION_KEPT')

        app.application('Monitor')
        app.application('Settings')
        expect(settings.and_(front)).to_be_visible()
        expect(windows).to_have_count(3)
        first, second, third = [window.bounding_box() for window in (terminal, monitor, settings)]
        assert first['x'] < second['x'] < third['x'], (first, second, third)
        assert first['y'] < second['y'] < third['y'], (first, second, third)
        assert settings.evaluate("""el => {
            const box = el.getBoundingClientRect();
            return document.elementFromPoint(box.x + 60, box.y + 65).closest('.desktop-window') === el;
        }""")
        app.screenshot('three-windows.png')

        # Click the strip of the Terminal title bar left exposed by later windows.
        page.mouse.click(first['x'] + 12, first['y'] + 16)
        expect(terminal.and_(front)).to_be_visible()
        expect(windows).to_have_count(3)
        assert monitor.bounding_box() == second
        assert settings.bounding_box() == third
        expect(output).to_contain_text('WINDOW_SESSION_KEPT')

        title = terminal.locator('.desktop-window-titlebar').bounding_box()
        x, y = title['x'] + title['width'] * .55, title['y'] + title['height'] / 2
        page.mouse.move(x, y)
        page.mouse.down()
        page.mouse.move(x + 60, y + 45, steps=8)
        page.mouse.up()
        moved = terminal.bounding_box()
        assert moved['x'] > first['x'] and moved['y'] > first['y'], (first, moved)
        assert monitor.bounding_box() == second
        assert settings.bounding_box() == third

        terminal.locator('.window-maximize').click()
        expect(terminal).to_have_class('desktop-window app-terminal active maximized')
        expect(windows).to_have_count(3)
        terminal.locator('.window-maximize').click()
        expect(terminal).to_have_class('desktop-window app-terminal active')
        assert terminal.bounding_box() == moved

        terminal.locator('.window-minimize').click()
        expect(terminal).to_be_hidden()
        expect(windows).to_have_count(2)
        expect(settings.and_(front)).to_be_visible()
        app.application('Terminal')
        expect(terminal.and_(front)).to_be_visible()
        assert terminal.bounding_box() == moved
        expect(output).to_contain_text('WINDOW_SESSION_KEPT')

        app.application('Settings')
        settings.locator('.window-close').click()
        expect(settings).to_have_count(0)
        expect(terminal.and_(front)).to_be_visible()
        assert monitor.bounding_box() == second
        page.locator('.desktop-dock-home').click()
        expect(windows).to_have_count(0)
        app.application('Monitor')
        expect(monitor.and_(front)).to_be_visible()
        assert monitor.bounding_box() == second
        assert not errors, errors
        print('Independent stacking, click-to-front, maximize, minimize, restore and live Terminal passed.')


if __name__ == '__main__':
    main()
