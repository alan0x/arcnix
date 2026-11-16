"""Remote Terminal here workflows on an owned WebView and deterministic SSH peer."""
import json
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def main():
    output = ROOT / 'artifacts/terminal-here-ui' / f'run-{time.time_ns()}'
    output.mkdir(parents=True)
    local = output / 'local'
    local.mkdir()
    (local / 'local.txt').write_bytes(b'local context fixture')
    checks = []

    with Peer(output / 'peer') as peer:
        folder = peer.root / 'terminal-folder'
        folder.mkdir()
        (folder / 'nested.txt').write_bytes(b'nested context fixture')
        (peer.root / 'other.txt').write_bytes(b'root context fixture')
        literal_name = "literal ' 中文;$(touch PWNED)"
        (peer.root / literal_name).mkdir()
        missing = peer.root / 'gone-directory'
        missing.mkdir()
        config = peer.config([peer.profile('Terminal here')], local)
        with Desktop(output / 'desktop', config) as app:
            page = app.page
            app.connect()
            menu = page.locator('.file-context-menu')
            screen = page.locator('.terminal-surface')
            reader = screen.locator('.xterm-accessibility-tree')
            keyboard = screen.locator('.xterm-helper-textarea')
            tabs = page.locator('.terminal-tabs .terminal-tab')
            status = page.locator('.terminal-status')

            def terminal_running():
                expect(status).to_have_text('Interactive SSH')
                expect(reader).to_contain_text('Arcnix terminal fixture')

            def command(value):
                expect(screen).to_be_visible()
                keyboard.focus()
                expect(keyboard).to_be_focused()
                page.keyboard.type(value)
                page.keyboard.press('Enter')

            def files():
                page.get_by_role('button', name='Files', exact=True).click()

            def here(name=None):
                if name is None:
                    listing = page.locator('.remote-slot .file-list-container')
                    box = listing.bounding_box()
                    assert box
                    listing.click(button='right', position=dict(x=box['width'] - 8, y=box['height'] - 8))
                else:
                    app.file(name).click()
                    app.file(name).click(button='right')
                expect(menu).to_be_visible()
                action = menu.get_by_role('button', name='Terminal here', exact=True)
                expect(action).to_be_enabled()
                action.click()
                expect(menu).to_have_count(0)
                expect(screen).to_be_visible()

            # Leave a partially entered command in the existing shell. Opening
            # a directory must not send a cd into that foreground input stream.
            page.get_by_role('button', name='Terminal', exact=True).click()
            terminal_running()
            command('echo ORIGINAL_SESSION_MARKER')
            expect(reader).to_contain_text('ORIGINAL_SESSION_MARKER')
            keyboard.focus()
            page.keyboard.type('echo ORIGINAL_PENDING-')
            expect(reader).to_contain_text('echo ORIGINAL_PENDING-')
            files()

            app.file('local.txt', remote=False).click(button='right')
            expect(menu).to_be_visible()
            expect(menu.get_by_role('button', name='Terminal here', exact=True)).to_have_count(0)
            page.keyboard.press('Escape')
            app.file('terminal-folder').click()
            app.file('other.txt').click(modifiers=['Control'])
            app.file('terminal-folder').click(button='right')
            expect(menu.get_by_role('button', name='Terminal here', exact=True)).to_be_disabled()
            page.keyboard.press('Escape')
            checks.append('Terminal here is remote-only and refuses ambiguous multiple selections')

            app.file('terminal-folder').click()
            app.file('terminal-folder').click(button='right')
            app.screenshot('folder-menu.png')
            menu.get_by_role('button', name='Terminal here', exact=True).click()
            terminal_running()
            expect(tabs).to_have_count(2)
            command('pwd')
            expect(reader).to_contain_text('/terminal-folder')
            app.screenshot('folder-terminal.png')
            checks.append('The remote folder menu opens a new SSH terminal directly in that directory')

            tabs.first.click()
            terminal_running()
            expect(reader).to_contain_text('ORIGINAL_SESSION_MARKER')
            keyboard.focus()
            page.keyboard.type('RESTORED')
            page.keyboard.press('Enter')
            expect(reader).to_contain_text('ORIGINAL_PENDING-RESTORED')
            command('pwd')
            expect(reader).to_contain_text('/fixture')
            checks.append('The original terminal keeps its history, live session and partially entered command while another directory opens')

            files()
            page.locator('.remote-slot').get_by_role('button', name='Expand terminal-folder', exact=True).click()
            expect(app.file('nested.txt')).to_be_visible()
            here('nested.txt')
            terminal_running()
            expect(tabs).to_have_count(3)
            command('pwd')
            expect(reader).to_contain_text('/terminal-folder')
            checks.append('An expanded nested file opens its actual parent directory rather than the pane root')

            files()
            here()
            terminal_running()
            expect(tabs).to_have_count(4)
            command('pwd')
            eventually(lambda: any(line.strip() == '/' for line in reader.inner_text().splitlines()))
            checks.append('A blank-space menu opens the current remote directory')

            files()
            here(literal_name)
            terminal_running()
            expect(tabs).to_have_count(5)
            command('pwd')
            expect(reader).to_contain_text('/' + literal_name)
            requests = [json.loads(line) for line in (peer.root / 'terminal-here-requests.jsonl').read_text(encoding='utf-8').splitlines()]
            assert requests[-1]['directory'] == '/' + literal_name, requests[-1]
            assert not (peer.root / 'PWNED').exists()
            app.screenshot('literal-directory.png')
            checks.append('Unicode, spaces, apostrophes, semicolons and shell-substitution text stay one literal directory argument')

            files()
            expect(app.file('gone-directory')).to_be_visible()
            missing.rmdir()
            here('gone-directory')
            expect(tabs).to_have_count(6)
            eventually(lambda: 'exited' in status.inner_text().lower() or 'failed' in status.inner_text().lower())
            expect(reader).to_contain_text('directory does not exist')
            expect(reader).not_to_contain_text('Arcnix terminal fixture')
            app.screenshot('missing-directory.png')
            active = page.locator('.terminal-tab-item').filter(has=page.locator('.terminal-tab.active'))
            active.locator('.terminal-tab-close').click()
            expect(tabs).to_have_count(5)
            terminal_running()
            command('pwd')
            expect(reader).to_contain_text('/' + literal_name)
            checks.append('A disappeared directory exits with an error instead of falling back to home; closing that tab leaves other terminals usable')

            page.get_by_role('button', name='Disconnect', exact=True).click()
            app.connect()
            page.get_by_role('button', name='Terminal', exact=True).click()
            terminal_running()
            expect(tabs).to_have_count(1)
            expect(reader).not_to_contain_text('ORIGINAL_SESSION_MARKER')
            command('pwd')
            expect(reader).to_contain_text('/fixture')
            checks.append('Disconnecting clears every old terminal tab and reconnecting starts a clean shell without stale history')

    result = dict(passed=len(checks), checks=checks, output=str(output))
    (output / 'results.json').write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps(result, ensure_ascii=False), flush=True)


if __name__ == '__main__':
    main()
