"""Exercise desktop shortcut routing and preference writes in the native WebView."""
import re
import json
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def main():
    out = ROOT / 'artifacts' / 'desktop-shortcuts' / f'run-{time.time_ns()}'
    out.mkdir(parents=True)
    with Peer(out / 'peer') as peer, Desktop(
        out / 'desktop', peer.config([peer.profile('Shortcut test')], out),
    ) as app:
        page = app.page
        errors = []
        page.on('pageerror', lambda error: errors.append(str(error)))
        preferences = peer.root / 'home/arcnix/.arcnix/desktop.json'

        def bindings():
            return json.loads(preferences.read_text()).get('shortcuts', {})

        def settings():
            app.application('Settings')
            page.locator('.desktop-settings-tabs').get_by_role(
                'button', name='Keyboard shortcuts', exact=True,
            ).click()
            return page.locator('.desktop-custom-keys')

        def record(action, chord):
            button = page.get_by_role('button', name=f'Change shortcut: {action}', exact=True)
            button.click()
            expect(button).to_have_attribute('aria-pressed', 'true')
            page.keyboard.press(chord)
            expect(button).not_to_have_attribute('aria-pressed', 'true')
            return button

        def title(label):
            expect(page.locator('.desktop-window.active .desktop-window-title')).to_have_text(label)

        app.connect()
        app.application('Terminal')
        terminal_input = page.locator('.terminal-surface .xterm-helper-textarea')
        expect(terminal_input).to_be_visible()
        app.application('Monitor')
        title('Monitor')
        page.locator('.host-select').focus()
        page.keyboard.press('Control+Shift+Tab')
        title('Terminal')
        expect(page.locator('.desktop-canvas')).to_be_focused()
        page.keyboard.press('Control+Tab')
        title('Monitor')
        page.keyboard.press('Control+Shift+ArrowUp')
        expect(page.locator('.desktop-window.active')).to_have_class(re.compile(r'\bmaximized\b'))
        page.keyboard.press('Control+Shift+Tab')
        title('Terminal')
        page.keyboard.press('Control+Tab')
        title('Monitor')
        expect(page.locator('.desktop-window.active')).to_have_class(re.compile(r'\bmaximized\b'))
        page.keyboard.press('Control+Shift+ArrowUp')
        expect(page.locator('.desktop-window.active')).not_to_have_class(re.compile(r'\bmaximized\b'))
        page.keyboard.press('Control+Shift+ArrowDown')
        expect(page.locator('.desktop-window.app-resources')).to_be_hidden()
        title('Terminal')
        page.keyboard.press('Control+Tab')
        title('Monitor')
        page.keyboard.press('Control+Shift+Tab')
        title('Terminal')
        expect(page.locator('.desktop-canvas')).to_be_focused()
        page.keyboard.press('Enter')
        expect(terminal_input).to_be_focused()

        panel = settings()
        tabs = page.locator('.desktop-settings-tabs')
        expect(tabs.get_by_role('button').last).to_have_text('About')
        tabs.get_by_role('button', name='About', exact=True).click()
        about = page.locator('.desktop-settings-page .about-details')
        expect(about).to_contain_text(re.compile(r'Version \d+\.\d+\.\d+'))
        expect(about.get_by_role('button', name='Project on GitHub', exact=True)).to_be_visible()
        expect(page.locator('.desktop-settings-page .update-panel')).to_have_count(0)
        tabs.get_by_role('button', name='Dock', exact=True).click()
        page.get_by_role('slider', name='Dock icon size').focus()
        page.keyboard.press('Control+Tab')
        title('Terminal')
        page.keyboard.press('Control+Shift+Tab')
        title('Settings')
        tabs.get_by_role('button', name='Keyboard shortcuts', exact=True).click()
        fullscreen = 'Toggle desktop fullscreen'
        button = record(fullscreen, 'Control+Tab')
        expect(panel.locator('.desktop-shortcut-error')).to_have_text('This shortcut is already in use.')
        expect(button).to_contain_text('Ctrl+Shift+F')
        button = record(fullscreen, 'Control+l')
        expect(button).to_contain_text('Ctrl+L')
        eventually(lambda: bindings().get('fullscreen') == 'Ctrl+L')
        panel.get_by_role('button', name='Disable shortcut: Switch to previous app', exact=True).click()
        eventually(lambda: 'previous_app' in bindings() and bindings()['previous_app'] is None)
        expect(panel.get_by_role('button', name='Change shortcut: Switch to previous app', exact=True)).to_contain_text('Unassigned')
        page.keyboard.press('Control+Shift+Tab')
        title('Settings')
        page.keyboard.press('Control+l')
        exit_fullscreen = page.locator('.desktop-fullscreen-exit')
        expect(exit_fullscreen).to_be_visible()
        for name in ('Terminal', 'Monitor', 'Settings'):
            page.keyboard.press('Control+Tab')
            title(name)
        expect(exit_fullscreen).to_be_visible()
        record(fullscreen, 'Escape')
        expect(button).to_contain_text('Ctrl+L')
        expect(exit_fullscreen).to_be_visible()
        page.keyboard.press('Control+l')
        expect(exit_fullscreen).to_have_count(0)

        app.application('Terminal')
        expect(page.locator('.terminal-status')).to_have_text('Interactive SSH')
        terminal_input.focus()
        page.keyboard.press('Control+Tab')
        title('Terminal')
        terminal_input.focus()
        page.evaluate("""() => {
            window.remoteControlL = false;
            document.querySelector('.terminal-input-bridge').addEventListener('input', event => {
                const packet = JSON.parse(event.target.value);
                if (packet.kind === 'data' && packet.data.includes(String.fromCharCode(12))) {
                    window.remoteControlL = true;
                }
            });
        }""")
        page.keyboard.press('Control+l')
        eventually(lambda: page.evaluate('window.remoteControlL'))
        expect(exit_fullscreen).to_have_count(0)
        expect(terminal_input).to_be_focused()
        page.locator('.desktop-window.active .desktop-window-mark').click()
        page.keyboard.press('Control+Tab')
        title('Monitor')
        app.application('Terminal')
        page.locator('.desktop-window.active .window-maximize').focus()
        page.keyboard.press('Control+l')
        expect(exit_fullscreen).to_be_visible()
        page.keyboard.press('Control+l')
        expect(exit_fullscreen).to_have_count(0)

        app.stop()
        app.launch()
        page = app.page
        exit_fullscreen = page.locator('.desktop-fullscreen-exit')
        page.on('pageerror', lambda error: errors.append(str(error)))
        app.connect()
        panel = settings()
        expect(panel.get_by_role('button', name=f'Change shortcut: {fullscreen}', exact=True)).to_contain_text('Ctrl+L')
        expect(panel.get_by_role(
            'button', name='Change shortcut: Switch to previous app', exact=True,
        )).to_contain_text('Unassigned')
        page.keyboard.press('Control+l')
        expect(exit_fullscreen).to_be_visible()
        panel.get_by_role('button', name='Restore shortcut defaults', exact=True).click()
        eventually(lambda: not bindings())
        page.keyboard.press('Control+Shift+F')
        expect(exit_fullscreen).to_have_count(0)
        app.screenshot('desktop-shortcuts.png')
        assert not errors, errors
        print('Desktop About, Ctrl+Tab in maximized and fullscreen modes, terminal focus, recording and persistence passed.')


if __name__ == '__main__':
    main()
