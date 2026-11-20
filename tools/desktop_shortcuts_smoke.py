"""Exercise desktop shortcut routing and preference writes in the native WebView."""
import re
import json
import time

from server_apps_fixture import read_desktop_settings
from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def main():
    out = ROOT / 'artifacts' / 'desktop-shortcuts' / f'run-{time.time_ns()}'
    out.mkdir(parents=True)
    with Peer(out / 'peer') as peer, Desktop(
        out / 'desktop', peer.config([peer.profile('Shortcut test')], out),
    ) as app:
        (peer.root / '.shortcut-example').touch()
        (peer.root / 'shortcut-test.txt').write_text('initial', encoding='utf-8')
        page = app.page
        errors = []
        page.on('pageerror', lambda error: errors.append(str(error)))
        preferences = peer.root / 'home/arcnix/.arcnix/desktop.json'

        def bindings():
            return read_desktop_settings(preferences).get('shortcuts', {})

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
        expect(page.locator('.desktop-window.app-monitor')).to_be_hidden()
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
        panel.get_by_role('button', name='Restore defaults', exact=True).click()
        eventually(lambda: not bindings())
        page.keyboard.press('Control+Shift+F')
        expect(exit_fullscreen).to_have_count(0)
        expect(page.get_by_text('Saved on server', exact=True)).to_have_count(0)
        focus = record('Move focus forward', 'Control+Alt+j')
        shortcut_state = page.locator('.desktop-shortcut-state')
        eventually(lambda: json.loads(shortcut_state.text_content())['bindings']['focus_next'] == 'Ctrl+Alt+J')
        expect(focus).to_be_focused()
        page.keyboard.press('Tab')
        expect(focus).to_be_focused()
        page.keyboard.press('Control+Alt+j')
        expect(focus).not_to_be_focused()
        panel.get_by_role('button', name='Restore default: Move focus forward', exact=True).click()
        record('Minimize window', 'Meta+l')
        expect(panel.locator('.desktop-shortcut-error')).to_have_text('This shortcut is reserved by Windows.')
        record('Open tab', 'Control+Shift+w')
        expect(panel.locator('.desktop-shortcut-error')).to_have_text('This shortcut is already in use.')
        record('Open tab', 'Control+Shift+y')
        eventually(lambda: bindings().get('terminal_new_tab') == 'Ctrl+Shift+Y')
        app.application('Terminal')
        terminal_input = page.locator('.terminal-surface .xterm-helper-textarea')
        terminal_input.focus()
        terminal_tabs = page.locator('.terminal-tab')
        count = terminal_tabs.count()
        page.keyboard.press('Control+Shift+y')
        expect(terminal_tabs).to_have_count(count + 1)
        panel = settings()
        panel.get_by_role('button', name='Restore default: Open tab', exact=True).click()
        eventually(lambda: 'terminal_new_tab' not in bindings())
        expect(panel.get_by_role('button', name='Change shortcut: Open tab')).to_contain_text('Ctrl+Shift+T')
        expect(panel.get_by_role('button', name='Restore default: Open tab', exact=True)).to_have_count(0)
        record('Toggle hidden files', 'Control+Shift+h')
        eventually(lambda: bindings().get('files_hidden') == 'Ctrl+Shift+H')
        app.application('Files')
        hidden = app.file('.shortcut-example')
        expect(hidden).to_be_visible()
        page.keyboard.press('Control+Shift+h')
        expect(hidden).to_have_count(0)
        page.keyboard.press('Control+Shift+h')
        expect(hidden).to_be_visible()
        settings()
        record('Save', 'Control+Alt+s')
        eventually(lambda: bindings().get('code_save') == 'Ctrl+Alt+S')
        app.application('Files')
        app.file('shortcut-test.txt').dblclick()
        editor = app.replace_code('saved using a remapped shortcut\n')
        editor.press('Control+s')
        page.wait_for_timeout(300)
        assert (peer.root / 'shortcut-test.txt').read_text(encoding='utf-8') == 'initial'
        editor.press('Control+Alt+s')
        eventually(lambda: (peer.root / 'shortcut-test.txt').read_text(encoding='utf-8') == 'saved using a remapped shortcut\n')
        record_button = settings()
        record_button.get_by_role('button', name='Restore defaults', exact=True).click()
        eventually(lambda: not bindings())
        app.screenshot('desktop-shortcuts.png')
        assert not errors, errors
        print('Desktop About, Ctrl+Tab in maximized and fullscreen modes, terminal focus, recording and persistence passed.')


if __name__ == '__main__':
    main()
