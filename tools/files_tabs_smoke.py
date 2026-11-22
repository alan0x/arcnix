"""Verify Files tabs and floating window moves with a real SFTP workspace."""
import json
import re
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def main():
    out = ROOT / 'artifacts/files-tabs' / str(time.time_ns())
    local = out / 'local'
    local.mkdir(parents=True)
    with Peer(out / 'peer') as peer:
        (peer.root / 'Projects').mkdir()
        (peer.root / 'Projects/notes.txt').write_text('original', encoding='utf-8')
        (peer.root / '.private').mkdir()
        (peer.root / '.env').touch()
        with Desktop(out / 'desktop', peer.config([peer.profile('Files tabs')], local)) as app:
            page = app.page
            app.connect()
            app.application('Files')
            windows = page.locator('.app-files.desktop-window:not(.hidden)')
            active = page.locator('.app-files.desktop-window.active')
            root = active.locator('.files-tab-select')
            expect(root).to_have_text('/')
            hidden_dir = app.file('.private')
            dotfile = app.file('.env')
            dotfile.click()
            page.keyboard.press('F2')
            rename = active.locator('.file-inline-name')
            expect(rename).to_be_focused()
            assert rename.evaluate('e => e.selectionStart === 0 && e.selectionEnd === e.value.length')
            rename.press('Escape')
            expect(rename).to_have_count(0)
            expect(dotfile).to_be_focused()
            page.keyboard.press('Control+h')
            expect(dotfile).to_have_count(0)
            expect(hidden_dir).to_have_count(0)
            assert not json.loads(app.config.read_text(encoding='utf-8'))['config']['settings']['show_hidden_files']
            active.locator('.files-new-tab + .files-settings').click()
            settings = page.get_by_role('dialog', name='Files settings')
            expect(settings.get_by_role('radio', name='Hide hidden files', exact=True)).to_be_checked()
            settings.get_by_role('radio', name='Show hidden files', exact=True).check()
            settings.get_by_role('button', name='Done').click()
            expect(dotfile).to_be_visible()
            expect(hidden_dir).to_be_visible()
            assert json.loads(app.config.read_text(encoding='utf-8'))['config']['settings']['show_hidden_files']
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

            preview = page.locator('.window-tab-preview')

            def press_tab(tab):
                box = tab.bounding_box()
                x, y = box['x'] + 4, box['y'] + 5
                page.mouse.move(x, y)
                page.mouse.down()
                return x, y

            def drag(tab, x, y):
                press_tab(tab)
                page.mouse.move(x, y, steps=14)
                page.mouse.up()
                expect(preview).to_have_count(0)

            canvas = page.locator('.desktop-canvas').bounding_box()
            origin = active.locator('.files-tab').last.bounding_box()
            x, y = press_tab(root.last)
            page.mouse.move(x + 2, y)
            expect(preview).to_have_count(0)
            page.mouse.move(x + 55, y + 45, steps=8)
            expect(preview).to_have_text('Projects')
            expect(preview).to_be_visible()
            expect(preview).to_have_css('pointer-events', 'none')
            page.wait_for_function("""([x, y]) => {
                const box = document.querySelector('.window-tab-preview')?.getBoundingClientRect();
                return box && Math.abs(box.left - x) < 2 && Math.abs(box.top - y) < 2;
            }""", arg=[origin['x'] + 55, origin['y'] + 45])
            page.keyboard.press('Escape')
            expect(preview).to_have_count(0)
            page.mouse.up()
            expect(windows).to_have_count(1)
            expect(root).to_have_text(['/', 'Projects'])

            drag(root.last, canvas['x'] + 5, canvas['y'] + 5)
            expect(windows).to_have_count(2)
            expect(active.locator('.files-tab-select')).to_have_text('Projects')
            expect(note).to_be_visible()
            inactive = page.locator('.app-files.desktop-window:not(.hidden):not(.active)')
            expect(inactive.locator('.files-tab-select')).to_have_text('/')
            target = inactive.locator('.files-tab-select').bounding_box()
            press_tab(active.locator('.files-tab-select'))
            page.mouse.move(target['x'] + 6, target['y'] + target['height'] / 2, steps=14)
            slot = inactive.locator('.window-tab-slot')
            expect(slot).to_be_visible()
            assert slot.evaluate("el => el.nextElementSibling.matches('.files-tab')")
            assert 1 <= slot.evaluate("el => parseFloat(getComputedStyle(el, '::before').width)") <= 3
            assert inactive.locator('.files-tab-select').bounding_box()['x'] > target['x'] + 20
            page.mouse.move(canvas['x'] + 5, canvas['y'] + 5)
            expect(page.locator('.window-tab-slot')).to_have_count(0)
            page.mouse.move(target['x'] + 6, target['y'] + target['height'] / 2)
            page.mouse.up()
            expect(preview).to_have_count(0)
            expect(windows).to_have_count(1)
            expect(active.locator('.files-tab-select')).to_have_text(['Projects', '/'])
            expect(note).to_be_visible()

            page.keyboard.press('Control+t')
            expect(active.locator('.files-tab')).to_have_count(3)
            page.keyboard.press('Control+w')
            expect(active.locator('.files-tab')).to_have_count(2)
            active.locator('.files-sidebar').get_by_role('button', name='Trash', exact=True).click()
            expect(active.locator('.trash-panel')).to_be_visible()
            active.locator('.trash-panel').click(button='right')
            expect(page.locator('.file-context-menu')).to_have_count(0)
            expect(page.get_by_role('menu', name='Trash', exact=True)).to_have_count(0)
            active.get_by_role('button', name='Projects', exact=True).click()
            expect(app.file('notes.txt')).to_be_visible()
            app.file('notes.txt').click(button='right')
            app.action('Move to Trash')
            expect(app.file('notes.txt')).to_have_count(0)
            active.locator('.files-sidebar').get_by_role('button', name='Trash', exact=True).click()
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
            active.locator('.files-sidebar').get_by_role('button', name='Filesystem', exact=True).click()
            app.file('Projects').click()
            actions = active.get_by_role('button', name='File view options', exact=True)
            actions.click()
            menu = active.locator('.file-actions-menu')
            expect(menu.get_by_role('button')).to_have_text([
                'Detailed list', 'Icon grid', '24 px', '48 px', '64 px', '96 px',
                'Name', 'Size', 'Modified', 'Descending', 'Visible columns',
            ])
            search = active.get_by_role('button', name='Search files', exact=True)
            assert search.evaluate("e => e.nextElementSibling.matches('.path-refresh')")
            menu.get_by_role('button', name='96 px').click()
            icon = active.locator('.file-list .file-entry-icon').first
            def icon_size(pixels):
                eventually(lambda: abs(icon.evaluate("e => parseFloat(getComputedStyle(e).width)") - pixels) < .1)
            icon_size(96)
            slider = menu.get_by_role('slider', name='File icon size')
            slider.focus()
            slider.press('ArrowLeft')
            icon_size(95)
            slider.press('Escape')
            expect(menu).not_to_be_visible()
            pane = active.locator('.files-pane')
            pane.hover()
            page.keyboard.down('Control')
            try:
                for size in (64, 48, 24):
                    page.mouse.wheel(0, 100)
                    icon_size(size)
                page.mouse.wheel(0, 100)
                expect(pane).to_have_class(re.compile(r'\bdetails-view\b'))
                page.mouse.wheel(0, -100)
                icon_size(24)
            finally:
                page.keyboard.up('Control')
            page.mouse.wheel(0, -100)
            icon_size(24)
            original = active.locator('.files-tab-select[aria-pressed=true]').inner_text()
            page.keyboard.press('Control+t')
            actions.click()
            menu.get_by_role('button', name='64 px').click()
            menu.get_by_role('button', name='Size', exact=True).click()
            menu.get_by_role('button', name='Descending').click()
            expect(menu.get_by_role('button', name='Descending')).to_have_attribute('aria-pressed', 'true')
            page.keyboard.press('Escape')
            active.locator('.files-tab-select').filter(has_text=original).first.click()
            icon_size(24)
            actions.click()
            expect(menu.get_by_role('button', name='Name', exact=True)).to_have_attribute('aria-pressed', 'true')
            expect(menu.get_by_role('button', name='Descending')).to_have_attribute('aria-pressed', 'false')
    print('Files tab navigation, detach, merge, shortcuts, hidden files settings, Trash menus and restore passed.')


if __name__ == '__main__':
    main()
