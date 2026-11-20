"""Verify Files tabs and floating window moves with a real SFTP workspace."""
import re
import time

from desktop_fixture import Desktop, Peer, ROOT, expect


def main():
    out = ROOT / 'artifacts/files-tabs' / str(time.time_ns())
    local = out / 'local'
    local.mkdir(parents=True)
    with Peer(out / 'peer') as peer:
        (peer.root / 'Projects').mkdir()
        (peer.root / 'Projects/notes.txt').write_text('original', encoding='utf-8')
        with Desktop(out / 'desktop', peer.config([peer.profile('Files tabs')], local)) as app:
            page = app.page
            app.connect()
            app.application('Files')
            windows = page.locator('.app-explorer.desktop-window:not(.hidden)')
            active = page.locator('.app-explorer.desktop-window.active')
            root = active.locator('.files-tab-select')
            expect(root).to_have_text('/')
            active.get_by_role('button', name='Projects', exact=True).click(button='right')
            app.action('Open in new tab')
            expect(root).to_have_text(['/', 'Projects'])
            note = app.file('notes.txt')
            expect(note).to_be_visible()
            note.click()
            root.first.click()
            expect(app.file('Projects')).to_be_visible()
            root.last.click()
            expect(note).to_be_visible()
            expect(note.locator('..')).to_have_class(re.compile('row-selected'))

            preview = page.locator('[data-gly-drag-preview]')

            def press_tab(tab):
                box = tab.bounding_box()
                x, y = box['x'] + box['width'] / 2, box['y'] + box['height'] / 2
                page.mouse.move(x, y)
                page.mouse.down()
                return x, y

            def drag(tab, x, y):
                press_tab(tab)
                page.mouse.move(x, y, steps=14)
                page.mouse.up()
                expect(preview).to_have_count(0)

            canvas = page.locator('.desktop-canvas').bounding_box()
            x, y = press_tab(root.last)
            page.mouse.move(x + 2, y)
            expect(preview).to_have_count(0)
            page.mouse.move(x + 55, y + 45, steps=8)
            expect(preview).to_have_text('Projects')
            expect(preview).to_be_visible()
            expect(preview).to_have_css('pointer-events', 'none')
            page.wait_for_function("""([x, y]) => {
                const box = document.querySelector('[data-gly-drag-preview]')?.getBoundingClientRect();
                return box && Math.abs(box.left - (x + 14)) < 2 && Math.abs(box.top - (y + 16)) < 2;
            }""", arg=[x + 55, y + 45])
            page.keyboard.press('Escape')
            expect(preview).to_have_count(0)
            page.mouse.up()
            expect(windows).to_have_count(1)
            expect(root).to_have_text(['/', 'Projects'])

            drag(root.last, canvas['x'] + 5, canvas['y'] + 5)
            expect(windows).to_have_count(2)
            expect(active.locator('.files-tab-select')).to_have_text('Projects')
            expect(note).to_be_visible()
            inactive = page.locator('.app-explorer.desktop-window:not(.hidden):not(.active)')
            expect(inactive.locator('.files-tab-select')).to_have_text('/')
            target = inactive.locator('.files-tab-select').bounding_box()
            drag(active.locator('.files-tab-select'), target['x'] + 6, target['y'] + target['height'] / 2)
            expect(windows).to_have_count(1)
            expect(active.locator('.files-tab-select')).to_have_text(['Projects', '/'])
            expect(note).to_be_visible()

            page.keyboard.press('Control+t')
            expect(active.locator('.files-tab')).to_have_count(3)
            page.keyboard.press('Control+w')
            expect(active.locator('.files-tab')).to_have_count(2)
            active.locator('.explorer-sidebar').get_by_role('button', name='Trash', exact=True).click()
            expect(active.locator('.trash-panel')).to_be_visible()
            active.locator('.trash-panel').click(button='right')
            expect(page.locator('.file-context-menu')).to_have_count(0)
            expect(page.get_by_role('menu', name='Trash', exact=True)).to_have_count(0)
            active.get_by_role('button', name='Projects', exact=True).click()
            expect(app.file('notes.txt')).to_be_visible()
            app.file('notes.txt').click(button='right')
            app.action('Move to Trash')
            expect(app.file('notes.txt')).to_have_count(0)
            active.locator('.explorer-sidebar').get_by_role('button', name='Trash', exact=True).click()
            row = active.locator('.trash-item').filter(has_text='/Projects/notes.txt')
            menu = page.get_by_role('menu', name='Trash', exact=True)
            target = row.get_by_role('group', name='/Projects/notes.txt', exact=True)
            target.press('Shift+F10')
            expect(menu.get_by_role('menuitem')).to_have_text(['Restore', 'Delete permanently'])
            expect(menu.get_by_role('menuitem', name='Restore', exact=True)).to_be_focused()
            page.keyboard.press('ArrowDown')
            expect(menu.get_by_role('menuitem', name='Delete permanently', exact=True)).to_be_focused()
            page.keyboard.press('Escape')
            expect(menu).not_to_be_visible()
            expect(target).to_be_focused()
            row.click(button='right')
            menu.get_by_role('menuitem', name='Restore', exact=True).click()
            expect(row).to_have_count(0)
            assert (peer.root / 'Projects/notes.txt').read_text(encoding='utf-8') == 'original'
            active.locator('.explorer-sidebar').get_by_role('button', name='Filesystem', exact=True).click()
            actions = active.get_by_role('button', name='Remote file actions', exact=True)
            actions.click()
            menu = active.locator('.file-actions-menu')
            expect(menu.get_by_role('button', name='Transfer defaults')).to_have_count(0)
            expect(menu.locator('.menu-divider')).to_have_count(3)
    print('Files tab navigation, detach, merge, shortcuts, Trash menus and restore passed.')


if __name__ == '__main__':
    main()
