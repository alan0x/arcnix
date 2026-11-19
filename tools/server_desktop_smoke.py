"""Exercise server Home/Desktop/Transfer tabs against owned loopback SSH/SFTP."""
import json
import re
import time

from desktop_fixture import Desktop, Peer, ROOT, expect, eventually


def main():
    output = ROOT / 'artifacts/server-desktop' / f'run-{time.time_ns()}'
    output.mkdir(parents=True)
    local = output / 'local'
    local.mkdir()
    (local / 'desktop-transfer.txt').write_text('Server desktop transfer check\n')
    checks = []
    with Peer(output / 'peer') as peer:
        (peer.root / 'work').mkdir()
        (peer.root / 'work' / 'example.txt').write_text('remote file')
        (peer.root / 'metrics-fixture.json').write_text('{"trace": true}')
        config = peer.config([peer.profile('Development'), dict(peer.profile('Offline'), port=9)], local)
        with Desktop(output / 'desktop', config) as app:
            page = app.page
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            tabs = page.locator('.workbench-tabs')

            def tab(label):
                tabs.get_by_role('button', name=label, exact=True).click()

            def launch(label):
                page.locator('.desktop-dock').get_by_role('button', name=label, exact=True).click()

            def home():
                tab('Home')
                expect(page.locator('.server-overview')).to_be_visible()

            page.locator('.host-select').first.click()
            expect(tabs.locator('.workbench-tab-select')).to_have_text(['Home'])
            expect(page.locator('.overview-connect')).to_have_accessible_name('Desktop')
            expect(page.locator('.overview-transfer')).to_have_accessible_name('Transfer files')
            gear = page.locator('.server-overview .overview-edit')
            expect(gear).to_have_accessible_name('Edit server')
            expect(gear.locator('.glyph-settings')).to_have_count(1)
            card = page.locator('.server-overview').bounding_box()
            button = gear.bounding_box()
            assert button['x'] > card['x'] + card['width'] - 75
            assert button['y'] < card['y'] + 35
            gear.click()
            edit = page.get_by_role('dialog', name='Edit server', exact=True)
            expect(edit).to_be_visible()
            edit.get_by_role('button', name='Cancel', exact=True).click()
            expect(page.locator('.server-overview .home-actions')).to_have_count(0)
            app.screenshot('home-light.png')

            # Either entry can establish SSH; opening files first doesn't open Desktop.
            page.locator('.overview-transfer').click()
            expect(tabs.locator('.workbench-tab-select')).to_have_text(['Home', 'Transfer files'])
            expect(page.locator('.desktop-modal-backdrop')).to_have_count(0)
            expect(page.locator('.connection-state')).to_have_text('SSH connected')
            identity = page.locator('.workspace-header .connection-target')
            expect(identity).to_have_text('arcnix@127.0.0.1')
            expect(identity).to_be_visible()
            expect(page.locator('.files-module')).to_be_visible()
            home()
            page.locator('.overview-connect').click()
            expect(tabs.locator('.workbench-tab-select')).to_have_text(['Home', 'Transfer files', 'Desktop'])
            expect(page.locator('.server-desktop')).to_be_visible()
            expect(page.locator('.workspace-header .workbench-tabs')).to_be_visible()
            expect(page.locator('.workspace-header .glyph-settings')).to_have_count(0)
            expect(identity).to_be_visible()
            expect(page.locator('.desktop-menubar')).to_have_count(0)
            assert page.locator('.workspace-header').bounding_box()['height'] <= 46
            expect(page.locator('.desktop-shortcuts .desktop-app-label')).to_have_text([
                'Terminal', 'Monitor', 'Software store', 'File explorer', 'Code', 'Trash', 'Web browser', 'Settings', 'Player',
            ])
            expect(page.locator('.server-desktop .files-module')).to_have_count(0)
            expect(page.locator('.desktop-window.active')).to_be_hidden()
            app.screenshot('desktop-light.png')
            checks.append('Home is always first; files and desktop open once in activation order and share SSH')
            checks.append('Edit server is an accessible gear in the upper-right of the overview and opens the existing editor')

            def fullscreen():
                tabs.get_by_role('button', name='Fullscreen desktop', exact=True).click()
                expect(page.locator('.navigation-rail')).to_be_hidden()
                expect(tabs).to_be_hidden()
                box = page.locator('.server-desktop').bounding_box()
                assert box['x'] == 0 and box['y'] == 0, box
                viewport = page.viewport_size
                assert abs(box['width'] - viewport['width']) <= 1, box
                assert abs(box['height'] - viewport['height']) <= 1, box

            fullscreen()
            expect(page.locator('.desktop-menubar .connection-target')).to_have_text('arcnix@127.0.0.1')
            expect(page.locator('.desktop-menubar .glyph-settings')).to_have_count(0)
            app.screenshot('desktop-fullscreen-light.png')
            page.get_by_role('button', name='Exit desktop fullscreen', exact=True).click()
            expect(tabs).to_be_visible()
            checks.append('The windowed and fullscreen headers identify the current SSH user and host')

            launch('Terminal')
            expect(page.locator('.terminal-status')).to_have_text('Interactive SSH')
            screen = page.locator('.terminal-surface')
            reader = screen.locator('.xterm-accessibility-tree')
            keyboard = screen.locator('.xterm-helper-textarea')
            expect(reader).to_contain_text('Arcnix terminal fixture')
            keyboard.focus()
            page.keyboard.type('echo DESKTOP_PRESERVED')
            page.keyboard.press('Enter')
            expect(reader).to_contain_text('DESKTOP_PRESERVED')
            surface = page.locator('.terminal-owned-surface').element_handle()
            home()
            page.locator('.overview-connect').click()
            expect(tabs.locator('.workbench-tab-select')).to_have_count(3)
            expect(reader).to_contain_text('DESKTOP_PRESERVED')
            assert surface.evaluate('el => el.isConnected')
            fullscreen()
            expect(reader).to_contain_text('DESKTOP_PRESERVED')
            assert surface.evaluate('el => el.isConnected')
            page.keyboard.press('Escape')
            expect(tabs).to_be_visible()
            page.get_by_role('button', name='Minimize window', exact=True).click()
            expect(page.locator('.desktop-window.active')).to_be_hidden()
            expect(page.locator('.desktop-dock .app-terminal')).to_have_class(re.compile(r'\brunning\b'))
            launch('Terminal')
            expect(reader).to_contain_text('DESKTOP_PRESERVED')
            page.get_by_role('button', name='Toggle maximize', exact=True).click()
            expect(page.locator('.desktop-window.active')).to_have_class(re.compile(r'\bmaximized\b'))
            page.get_by_role('button', name='Toggle maximize', exact=True).click()
            app.screenshot('terminal-light.png')
            checks.append('Terminal DOM and SSH survive Home, minimize, maximize and reopening an existing tab')

            tab('Transfer files')
            app.file('desktop-transfer.txt', remote=False).click()
            page.locator('.local-slot').get_by_role('button', name='Upload', exact=True).click()
            expect(page.locator('.queue-row').last).to_contain_text('Completed')
            assert (peer.root / 'desktop-transfer.txt').read_text() == 'Server desktop transfer check\n'
            app.screenshot('transfers-light.png')
            # Terminal here opens the desktop tab and a new PTY in the selected directory.
            app.file('work').click(button='right')
            page.locator('.file-context-menu').get_by_role('button', name='Terminal here', exact=True).click()
            expect(page.locator('.server-desktop')).to_be_visible()
            expect(page.locator('.terminal-tabs .terminal-tab')).to_have_count(2)
            expect(page.locator('.terminal-status')).to_have_text('Interactive SSH')
            checks.append('SFTP upload completes and Terminal here opens Desktop while preserving existing PTYs')

            launch('File explorer')
            expect(page.locator('.desktop-window.active .desktop-window-title')).to_have_text('File explorer')
            expect(page.locator('.remote-explorer')).to_be_visible()
            expect(page.locator('.remote-slot')).to_have_count(1)
            expect(page.locator('.local-slot')).to_have_count(0)
            expect(page.locator('.explorer-pane .pane-heading, .explorer-pane .pane-tools')).to_have_count(0)
            expect(page.locator('html')).not_to_have_class(re.compile(r'\bdesktop-motion\b'))
            titlebar = page.locator('.explorer-titlebar')
            expect(titlebar.locator('.explorer-sidebar-toggle .glyph-sidebar')).to_be_visible()
            expect(titlebar.get_by_role('button', name='Transfer files', exact=True)).to_have_count(0)
            expect(page.locator('.explorer-pane .pathbar')).to_have_count(0)
            for size in [dict(width=1440,height=980), dict(width=900,height=650)]:
                page.set_viewport_size(size)
                title = titlebar.bounding_box()
                pathbar = titlebar.locator('.pathbar').bounding_box()
                first = page.locator('.explorer-pane .file-row').first.bounding_box()
                assert title['y'] <= pathbar['y'] and pathbar['y'] + pathbar['height'] <= title['y'] + title['height'], (title, pathbar)
                assert first['y'] - title['y'] <= 78, (title, pathbar, first)
                actions = titlebar.get_by_role('button', name='Remote file actions', exact=True)
                expect(actions).to_be_in_viewport()
                assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
                app.screenshot(f'explorer-compact-{size["width"]}.png')
                if size['width'] == 900:
                    expect(titlebar.locator('.view-switch')).to_be_hidden()
                    menu = page.locator('.file-actions-menu')
                    actions.click()
                    menu.get_by_role('button', name='Icon grid', exact=True).click()
                    expect(page.locator('.explorer-pane.icons-view')).to_be_visible()
                    actions.click()
                    menu.get_by_role('button', name='Detailed list', exact=True).click()
                    expect(page.locator('.explorer-pane.details-view')).to_be_visible()
            page.set_viewport_size(dict(width=1440,height=980))
            checks.append('Explorer fits navigation, view actions and icon-only transfer into its titlebar at normal and compact widths')
            app.file('work').dblclick()
            expect(app.file('example.txt')).to_be_visible()
            app.file('example.txt').click(button='right')
            app.action('Rename')
            rename = page.get_by_role('dialog', name='Rename', exact=True)
            rename.get_by_role('textbox', name='New name', exact=True).fill('renamed.txt')
            rename.get_by_role('button', name='Rename', exact=True).click()
            expect(app.file('renamed.txt')).to_be_visible()
            assert (peer.root / 'work' / 'renamed.txt').read_text() == 'remote file'
            app.file('renamed.txt').click(button='right')
            app.action('Download')
            eventually(lambda: (local / 'renamed.txt').read_text() == 'remote file')
            app.file('renamed.txt').dblclick()
            expect(app.code().locator('.view-lines')).to_contain_text('remote file')
            app.replace_code('Edited from desktop Explorer\n').press('Control+s')
            eventually(lambda: (peer.root / 'work' / 'renamed.txt').read_text() == 'Edited from desktop Explorer\n')
            page.locator('.desktop-window.active .window-close').click()
            launch('File explorer')
            titlebar.get_by_role('button', name='Icon grid', exact=True).click()
            titlebar.get_by_role('button', name='Remote file actions', exact=True).click()
            page.locator('.file-actions-menu').get_by_role('button', name='New folder', exact=True).click()
            field = page.locator('.explorer-pane .file-inline-name')
            expect(field).to_be_focused()
            expect(field).to_have_value('New folder')
            assert (peer.root / 'work' / 'New folder').is_dir()
            field.fill('created-in-explorer')
            field.press('Enter')
            expect(app.file('created-in-explorer')).to_be_visible()
            assert (peer.root / 'work' / 'created-in-explorer').is_dir()
            search = titlebar.get_by_role('button', name='Search files', exact=True)
            search.click()
            file_filter = titlebar.get_by_role('textbox', name='Search current folder', exact=True)
            file_filter.fill('renamed')
            expect(app.file('created-in-explorer')).to_have_count(0)
            expect(app.file('renamed.txt')).to_be_visible()
            search.click()
            expect(file_filter).to_have_count(0)
            fullscreen()
            app.file('renamed.txt').click(button='right')
            app.action('Properties')
            properties = page.get_by_role('dialog', name='Properties', exact=True)
            expect(properties).to_contain_text('/work/renamed.txt')
            scrim = page.locator('.desktop-modal-backdrop .modal-scrim')
            assert abs(scrim.bounding_box()['y'] - page.locator('.desktop-canvas').bounding_box()['y']) <= 1
            assert properties.bounding_box()['y'] >= scrim.bounding_box()['y']
            page.keyboard.press('Escape')
            expect(properties).not_to_be_visible()
            expect(tabs).to_be_hidden()
            app.screenshot('explorer-fullscreen-light.png')
            page.keyboard.press('Escape')
            expect(tabs).to_be_visible()
            tab('Transfer files')
            expect(page.locator('.files-module')).to_be_visible()
            expect(app.file('renamed.txt')).to_be_visible()
            tab('Desktop')
            expect(page.locator('.remote-explorer')).to_be_visible()
            expect(app.file('renamed.txt')).to_be_visible()
            app.file('renamed.txt').click(button='right')
            app.action('Terminal here')
            expect(page.locator('.terminal-tabs .terminal-tab')).to_have_count(3)
            expect(page.locator('.terminal-status')).to_have_text('Interactive SSH')
            page.locator('.terminal-tabs .terminal-tab').first.click()
            expect(reader).to_contain_text('DESKTOP_PRESERVED')
            fullscreen()
            keyboard.focus()
            screen.click(button='right')
            expect(page.locator('.terminal-context-menu')).to_be_visible()
            page.keyboard.press('Escape')
            expect(page.locator('.terminal-context-menu')).to_have_count(0)
            expect(tabs).to_be_hidden()
            page.evaluate("""() => {
                window.terminalEscapeForwarded = false;
                document.querySelector('.terminal-input-bridge').addEventListener('input', event => {
                    const packet = JSON.parse(event.target.value);
                    if (packet.kind === 'data' && packet.data.includes(String.fromCharCode(27))) {
                        window.terminalEscapeForwarded = true;
                    }
                });
            }""")
            keyboard.focus()
            expect(keyboard).to_be_focused()
            page.keyboard.press('Escape')
            eventually(lambda: page.evaluate('window.terminalEscapeForwarded'))
            expect(tabs).to_be_hidden()
            page.locator('.desktop-fullscreen-exit').focus()
            page.keyboard.press('Escape')
            expect(tabs).to_be_visible()
            checks.append('Remote-only Explorer browses, renames, edits, creates folders and filters actual SFTP files, downloads correct bytes, shares its directory with Transfer files and opens Terminal here')
            checks.append('Fullscreen Escape closes the menu, reaches the focused remote terminal, or exits fullscreen from desktop controls as appropriate')

            launch('Monitor')
            expect(page.locator('.desktop-window.active .desktop-window-title')).to_have_text('Monitor')
            queries = peer.root / 'metrics-queries.jsonl'
            eventually(lambda: queries.exists() and len(queries.read_text().splitlines()) >= 1)
            home()
            # Allow a request already in flight to settle, then ensure Home does not poll.
            page.wait_for_timeout(700)
            count = len(queries.read_text().splitlines())
            page.wait_for_timeout(3600)
            assert len(queries.read_text().splitlines()) == count
            tab('Desktop')
            expect(page.locator('.desktop-window.active .desktop-window-title')).to_have_text('Monitor')
            launch('Software store')
            expect(page.locator('.software-store')).to_be_visible()
            launch('Monitor')
            sections = page.locator('.monitor-sidebar')
            sections.get_by_role('button', name='Services', exact=True).click()
            expect(page.locator('.operations-page')).to_be_visible()
            sections.get_by_role('button', name='Logs', exact=True).click()
            expect(page.locator('.system-logs-page')).to_be_visible()
            expect(page.locator('.logs-discover .glyph-search')).to_have_count(1)
            expect(page.locator('.logs-refresh .glyph-refresh')).to_have_count(1)
            logs = peer.root / 'system-log-queries.jsonl'

            def queries_since(offset):
                return [json.loads(line)['action'] for line in logs.read_text().splitlines()[offset:]]

            expect(page.locator('.logs-refresh')).to_be_enabled()
            count = len(logs.read_text().splitlines())
            page.locator('.logs-refresh').click()
            eventually(lambda: queries_since(count) == ['read'])
            expect(page.locator('.logs-discover')).to_be_enabled()
            count = len(logs.read_text().splitlines())
            page.locator('.logs-discover').click()
            eventually(lambda: queries_since(count) == ['discover', 'read'])
            page.locator('.desktop-window.app-resources .window-close').click()
            expect(page.locator('.desktop-window.app-resources')).to_have_count(0)
            expect(page.locator('.desktop-dock .app-resources')).not_to_have_class(re.compile(r'\brunning\b'))
            checks.append('Monitor includes services and logs; resource polling pauses on Home; search redetects log sources while refresh only reads logs')

            app.settings('General')
            page.get_by_role('button', name='Dark', exact=True).click()
            app.servers()
            app.screenshot('desktop-dark.png')
            launch('Terminal')
            app.screenshot('terminal-dark.png')
            page.get_by_role('button', name='Close Desktop tab', exact=True).click()
            expect(tabs.locator('.workbench-tab-select')).to_have_text(['Home', 'Transfer files'])
            expect(page.locator('.server-overview')).to_be_visible()
            page.locator('.overview-connect').click()
            expect(page.locator('.terminal-tabs .terminal-tab')).to_have_count(3)
            page.locator('.desktop-window.app-terminal .window-close').click()
            launch('Terminal')
            expect(page.locator('.terminal-tabs .terminal-tab')).to_have_count(3)
            expect(page.locator('.terminal-status')).to_have_text('Interactive SSH')
            checks.append('Closing a workspace tab keeps SSH; reopening Terminal restores the saved tab layout')

            for width in (1100, 900):
                page.set_viewport_size(dict(width=width, height=740))
                page.get_by_role('button', name='Show desktop', exact=True).click()
                assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
                box = page.locator('.desktop-dock').bounding_box()
                assert box and box['x'] >= 0 and box['x'] + box['width'] <= width
                app.screenshot(f'desktop-dark-{width}.png')
                home()
                header = page.locator('.workspace-header').bounding_box()
                assert page.locator('.server-overview').bounding_box()['y'] >= header['y']+header['height']
                app.screenshot(f'home-dark-{width}.png')
                tab('Desktop')
                launch('File explorer')
                fullscreen()
                assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
                app.screenshot(f'explorer-fullscreen-dark-{width}.png')
                page.get_by_role('button', name='Exit desktop fullscreen', exact=True).click()
            page.set_viewport_size(dict(width=1440, height=980))
            app.settings('General')
            page.get_by_role('button', name='简体中文', exact=True).click()
            app.servers()
            expect(tabs.locator('.workbench-tab-select')).to_have_text(['首页', '传输文件', '桌面'])
            expect(tabs.get_by_role('button', name='全屏桌面', exact=True)).to_be_visible()
            expect(page.locator('.desktop-window.active .desktop-window-title')).to_have_text('资源管理器')
            tabs.get_by_role('button', name='全屏桌面', exact=True).click()
            expect(page.get_by_role('button', name='退出桌面全屏', exact=True)).to_be_visible()
            app.screenshot('desktop-chinese-dark.png')
            page.keyboard.press('Escape')
            checks.append('Light/dark themes, live Chinese localization and 900/1100px layouts render without overflow')

            # Disconnect keeps useful tabs; switching servers resets their scope.
            page.locator('.connection-button').click()
            expect(identity).to_be_hidden()
            expect(page.locator('.workspace-reconnect')).to_be_visible()
            # Switching tabs while disconnected neither connects nor prompts for an unencrypted key.
            tabs.locator('.workbench-tab-select').first.click()
            tabs.locator('.workbench-tab-select').last.click()
            expect(page.locator('.workspace-reconnect')).to_be_visible()
            expect(page.get_by_role('dialog')).to_have_count(0)
            expect(page.locator('.connection-state')).to_have_text('SSH 未连接')
            # Its explicit Connect button reconnects without a prompt.
            page.locator('.workspace-reconnect').get_by_role('button', name='连接', exact=True).click()
            expect(page.locator('.connection-state')).to_have_text('SSH 已连接')
            expect(page.get_by_role('dialog')).to_have_count(0)
            page.locator('.connection-button').click()
            expect(page.locator('.workspace-reconnect')).to_be_visible()
            page.locator('.host-select').nth(1).click()
            expect(tabs.locator('.workbench-tab-select')).to_have_text(['首页'])
            expect(page.locator('.server-overview')).to_contain_text('Offline')
            assert not errors, errors
            checks.append('Tab switches never connect by themselves; reconnect is explicit; selecting another server clears its tabs and sessions; no browser errors')
        profile = dict(peer.profile('SSH only'), username='arcnix-nosftp')
        with Desktop(output / 'without-sftp', peer.config([profile], local)) as app:
            app.connect()
            expect(app.page.locator('.workspace-header .connection-target')).to_have_text('arcnix-nosftp@127.0.0.1')
            expect(app.page.locator('.remote-slot .file-empty')).to_contain_text('SFTP is unavailable')
            app.application('Terminal')
            expect(app.page.locator('.terminal-status')).to_have_text('Interactive SSH')
            app.application('File explorer')
            expect(app.page.locator('.remote-explorer .file-empty')).to_contain_text('SFTP is unavailable')
            expect(app.page.locator('.explorer-titlebar').get_by_role('button', name='Download', exact=True)).to_have_count(0)
            checks.append('Desktop and Terminal work when the server has no SFTP subsystem')
    (output / 'results.json').write_text(json.dumps(checks, indent=2))
    print(json.dumps(dict(output=str(output), checks=checks), indent=2))


if __name__ == '__main__':
    main()
