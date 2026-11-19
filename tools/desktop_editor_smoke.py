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
            app.application('Code')
            code = app.code()
            code.get_by_role('treeitem', name='plain.txt', exact=True).dblclick()
            expect(code.locator('.view-lines')).to_contain_text('plain text from SSH')
            app.replace_code('unsaved plain draft')
            p.locator('.desktop-window.active .window-minimize').click()
            p.locator('.desktop-dock-home').click()
            shortcuts.get_by_role('button', name='Notes.md', exact=True).click()
            expect(code.locator('.view-lines:visible').first).to_contain_text('Bold')
            code.get_by_role('button',name='Markdown: Open Preview to the Side').click()
            preview = code.locator('.arcnix-preview')
            expect(preview.locator('strong')).to_have_text('Bold text')
            expect(preview.locator('td').last).to_have_text('22')
            expect(preview.locator('pre code')).to_contain_text('ls -l')
            expect(preview.locator('img')).to_have_count(0)
            expect(preview).to_contain_text('<img src=')
            code.screenshot(path=str(out/'markdown-preview.png'))
            code.locator('.tab').filter(has_text='plain.txt').click()
            expect(code.locator('.view-lines:visible').first).to_contain_text('unsaved plain draft')
            window = p.locator('.desktop-window.app-code')
            window.locator('.window-close').click()
            keep = window.get_by_role('button',name='Keep editing')
            keep.click()
            expect(keep).to_be_hidden()
            expect(code.locator('.view-lines:visible').first).to_contain_text('unsaved plain draft')
            window.locator('.window-close').click()
            window.get_by_role('button',name='Discard changes').click()
            expect(window).to_have_count(0)
            assert (peer.root/'plain.txt').read_text() == 'plain text from SSH'
            assert not errors, errors
    print(json.dumps(dict(output=str(out),checks=['Desktop icon columns', 'Desktop file opens alongside working tree tab', 'Markdown formatting and escaped HTML', 'Dirty draft survives minimize and tab switch', 'Discard confirmation']),indent=2))


if __name__ == '__main__':
    main()
