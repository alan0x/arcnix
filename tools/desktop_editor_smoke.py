"""Open and edit remote text and Markdown in the Windows desktop workspace."""
import json
import time

from desktop_fixture import Desktop, Peer, ROOT, expect, eventually


def main():
    out = ROOT / 'artifacts/desktop-editor' / f'run-{time.time_ns()}'
    local = out / 'local'
    local.mkdir(parents=True)
    with Peer(out / 'peer', io_delay=.06) as peer:
        home_desktop = peer.root / 'home/arcnix/Desktop'
        home_desktop.mkdir(parents=True, exist_ok=True)
        notes = home_desktop / 'Notes.md'
        source = ('# Notes\n\n**Bold text**\n\n'
                  '| Name | Value |\n| --- | --- |\n| port | 22 |\n\n'
                  '```sh\nls -l\n```\n\n'
                  '<img src="https://invalid.example/image.png">\n')
        notes.write_bytes(b'\xef\xbb\xbf' + source.replace('\n', '\r\n').encode())
        for number in range(28):
            (home_desktop / f'sample-{number:02}.txt').write_text('sample', encoding='utf-8')
        (peer.root / 'plain.txt').write_text('plain text from SSH', encoding='utf-8')
        with Desktop(out / 'desktop', peer.config([peer.profile('Editor fixture')], local)) as app:
            p = app.page
            errors = []
            p.on('pageerror', lambda error: errors.append(str(error)))
            app.connect()
            app.workspace('Desktop')
            p.set_viewport_size(dict(width=900, height=620))
            shortcuts = p.locator('.desktop-shortcuts')
            expect(shortcuts.get_by_role('button', name='sample-27.txt', exact=True)).to_have_count(1)
            layout = shortcuts.evaluate('''grid => {
                const slots = [...grid.querySelectorAll('.desktop-icon-slot')];
                const next = slots.find(slot => slot.offsetLeft > slots[0].offsetLeft);
                return {
                    columns: new Set(slots.map(slot => slot.offsetLeft)).size,
                    firstDown: slots[1].offsetTop > slots[0].offsetTop,
                    nextStartsAtTop: next?.offsetTop === slots[0].offsetTop,
                    verticalOverflow: grid.scrollHeight - grid.clientHeight,
                    horizontalOverflow: grid.scrollWidth - grid.clientWidth,
                    scrollbar: getComputedStyle(grid).scrollbarWidth,
                };
            }''')
            assert layout['columns'] > 2 and layout['firstDown'] and layout['nextStartsAtTop'], layout
            assert layout['verticalOverflow'] <= 1 and layout['horizontalOverflow'] > 0 and layout['scrollbar'] == 'none', layout
            shortcuts.hover(position={'x': 180, 'y': 30})
            p.mouse.wheel(0, 450)
            eventually(lambda: shortcuts.evaluate('el => el.scrollLeft') > 0)
            shortcuts.evaluate('el => el.scrollLeft = 0')
            app.screenshot('desktop-icon-columns.png')
            p.set_viewport_size(dict(width=1440, height=980))
            app.application('Editor')
            editor = p.locator('.desktop-editor')
            editor.get_by_role('button', name='Open file…', exact=True).click()
            app.file('plain.txt').dblclick()
            contents = editor.get_by_role('textbox', name='File contents', exact=True)
            expect(contents).to_have_value('plain text from SSH')
            p.locator('.desktop-window.active .window-minimize').click()
            p.locator('.desktop-dock-home').click()
            p.locator('.desktop-shortcuts').get_by_role('button', name='Notes.md', exact=True).click()
            expect(p.locator('.desktop-window.active .desktop-window-title')).to_have_text('Editor')
            preview = editor.locator('.markdown-preview')
            strip = editor.locator('.editor-tab-strip')
            plain_tab = strip.get_by_role('button', name='/plain.txt', exact=True)
            notes_tab = strip.get_by_role('button', name='/home/arcnix/Desktop/Notes.md', exact=True)
            expect(strip.locator('.editor-tab')).to_have_count(2)
            expect(preview.get_by_role('heading', name='Notes')).to_be_visible()
            expect(preview.locator('strong')).to_have_text('Bold text')
            expect(preview.locator('td').last).to_have_text('22')
            expect(preview.locator('pre code')).to_have_text('ls -l\n')
            expect(preview.locator('img')).to_have_count(0)
            expect(preview).to_contain_text('<img src=')
            app.screenshot('markdown-preview.png')

            editor.get_by_role('button', name='Edit source', exact=True).click()
            expect(contents).to_have_value(source)
            edited = source.replace('# Notes', '# Saved notes')
            contents.fill(edited)
            editor.get_by_role('button', name='Preview', exact=True).click()
            expect(preview.get_by_role('heading', name='Saved notes')).to_be_visible()
            plain_tab.click()
            expect(contents).to_have_value('plain text from SSH')
            contents.fill('unsaved plain draft')
            notes_tab.click()
            expect(preview.get_by_role('heading', name='Saved notes')).to_be_visible()
            p.locator('.desktop-window.active .window-minimize').click()
            app.application('Editor')
            expect(preview.get_by_role('heading', name='Saved notes')).to_be_visible()
            editor.get_by_role('button', name='Edit source', exact=True).click()
            contents.press('Control+s')
            plain_tab.click()
            expect(contents).to_have_value('unsaved plain draft')
            eventually(lambda: notes.read_bytes() == b'\xef\xbb\xbf' + edited.replace('\n', '\r\n').encode())
            notes_tab.click()
            expect(editor.locator('.editor-feedback')).to_have_text('Remote file saved')

            editor.get_by_role('button', name='Open file…', exact=True).click()
            app.file('plain.txt').dblclick()
            expect(strip.locator('.editor-tab')).to_have_count(2)
            expect(contents).to_have_value('unsaved plain draft')
            notes_tab.click()
            close_plain = strip.get_by_role('button', name='Close file tab plain.txt', exact=True)
            close_plain.click()
            expect(editor.get_by_text('Discard unsaved changes?')).to_be_visible()
            editor.get_by_role('button', name='Keep editing', exact=True).click()
            expect(contents).to_have_value('unsaved plain draft')
            close_plain.click()
            editor.get_by_role('button', name='Discard changes', exact=True).click()
            expect(plain_tab).to_have_count(0)
            expect(notes_tab).to_have_attribute('aria-pressed', 'true')
            assert (peer.root / 'plain.txt').read_text(encoding='utf-8') == 'plain text from SSH'

            contents.fill('# Unsaved draft\n')
            p.locator('.desktop-window.active .window-close').click()
            expect(editor.get_by_text('Discard unsaved changes in all open files?')).to_be_visible()
            editor.get_by_role('button', name='Keep editing', exact=True).click()
            expect(contents).to_have_value('# Unsaved draft\n')
            p.locator('.desktop-window.active .window-close').click()
            editor.get_by_role('button', name='Discard changes', exact=True).click()
            expect(p.locator('.desktop-window.app-editor')).to_have_count(0)
            assert b'Unsaved draft' not in notes.read_bytes()
            assert not errors, errors
    print(json.dumps(dict(output=str(out), checks=[
        'Desktop icons fill a column before growing right, with no vertical scroll or visible scrollbar',
        'Editor launcher, Explorer and Desktop file icons open independent tabs without duplicating a file',
        'Markdown renders headings, emphasis, tables and code offline with raw HTML shown as text',
        'Source editing, preview, minimize and Ctrl+S retain the draft and source encoding',
        'Switching tabs preserves each draft and preview mode, and saving targets the original tab',
        'Closing a dirty tab or the editor window prompts before discarding',
    ]), indent=2))


if __name__ == '__main__':
    main()
