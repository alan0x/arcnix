"""Search an unopened remote file from Code and edit the selected match."""
import re
import time
from desktop_fixture import Desktop, Peer, ROOT, expect, eventually


def main():
    out = ROOT / 'artifacts/code-search' / str(time.time_ns())
    out.mkdir(parents=True)
    with Peer(out / 'peer') as peer:
        root = peer.root / 'project'
        nested = root / '文档'
        nested.mkdir(parents=True)
        notes = nested / 'notes.md'
        notes.write_text('one\ntwo\nthree\n端口 🚦 SEARCH_PORT=443\nfive\n', encoding='utf8')
        (root / 'config.ini').write_text('SEARCH_PORT=80\n', encoding='utf8')
        (root / '.gitignore').write_text('ignored/\n')
        (root / 'ignored').mkdir()
        (root / 'ignored/private.txt').write_text('SEARCH_PORT=0')
        (root / 'binary.bin').write_bytes(b'\0SEARCH_PORT')
        (peer.root / 'outside.txt').write_text('SEARCH_PORT=0')
        profile = peer.profile('Search fixture')
        profile['initial_remote_path'] = '/project'
        with Desktop(out / 'app', peer.config([profile], out)) as app:
            app.connect()
            app.application('Code')
            page = app.code()
            expect(page.get_by_role('tab', name=re.compile('^Search'))).to_be_visible()
            expect(page.get_by_role('button', name='Accounts', exact=True)).to_have_count(0)
            expect(page.locator('.statusbar')).not_to_contain_text('SSH:')
            manage = page.get_by_role('button', name='Manage', exact=True)
            manage.click()
            theme = page.get_by_role('menuitem', name=re.compile('Color Theme'))
            expect(theme).to_be_visible()
            expect(page.get_by_role('menu').get_by_role('menuitem')).to_have_count(3)
            theme.click()
            choice = page.locator('.quick-input-list .monaco-list-row').filter(has_text='Dark Modern')
            choice.first.click()
            expect(page.locator('.monaco-workbench.vs-dark')).to_be_visible()
            manage.click()
            page.get_by_role('menuitem', name='File Icon Theme').click()
            page.locator('.quick-input-list .monaco-list-row').filter(has_text=re.compile(r'^None$')).click()
            manage.click()
            page.get_by_role('menuitem', name=re.compile('Command Palette')).click()
            expect(page.locator('.quick-input-widget')).to_be_visible()
            page.keyboard.press('Escape')
            page.get_by_role('tab', name=re.compile('^Search')).click()
            box = page.locator('.search-view .search-widget textarea').first
            box.fill('SEARCH_PORT')
            box.press('Enter')
            results = page.locator('.search-view .results')
            expect(results).to_contain_text('notes.md', timeout=20000)
            expect(results).to_contain_text('config.ini')
            expect(page.locator('.search-view')).to_contain_text('2 results in 2 files')
            expect(results).not_to_contain_text('outside.txt')
            expect(results).not_to_contain_text('private.txt')
            expect(results).not_to_contain_text('binary.bin')
            result = results.locator('.monaco-list-row').filter(has_text='443')
            result.click()
            area = page.locator('.monaco-editor textarea.inputarea').first
            expect(page.locator('.view-lines')).to_contain_text('端口')
            page.screenshot(path=str(out/'search.png'))
            # Replacing only the selected text proves both the target line and UTF-16 columns.
            area.focus()
            page.keyboard.insert_text('LISTEN')
            area.press('Control+s')
            eventually(lambda: '端口 🚦 LISTEN=443' in notes.read_text(encoding='utf8'))
            assert (root / 'config.ini').read_text() == 'SEARCH_PORT=80\n'
            page.get_by_role('tab', name=re.compile('^Explorer')).click()
            area.focus()
            area.press('Control+Shift+f')
            expect(box).to_be_focused()
            page.get_by_role('checkbox', name=re.compile('Use Regular Expression')).click()
            box.fill(r'LISTEN=\d+')
            box.press('Enter')
            expect(results.locator('.monaco-list-row').filter(has_text='LISTEN=443')).to_be_visible()
            box.fill('never-matches-this')
            box.press('Enter')
            expect(results.locator('.monaco-list-row')).to_have_count(0)
            print(out)


if __name__ == '__main__':
    main()
