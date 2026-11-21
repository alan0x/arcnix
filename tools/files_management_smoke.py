"""Exercise Files mutations and real drag/drop between desktop windows."""
import json
import time
from zipfile import ZipFile

from server_apps_fixture import read_desktop_settings
from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def main():
    out = ROOT / 'artifacts/files-management' / str(time.time_ns())
    with Peer(out / 'peer') as peer:
        root = peer.root
        notes = root / 'notes.txt'
        notes.write_bytes(b'ordinary remote file')
        inbox = root / 'Inbox'
        inbox.mkdir()
        (inbox / 'empty').mkdir()
        for name in ('move.txt', 'pin.txt', 'tag.txt', 'trash.txt'):
            (root / name).write_text(name, encoding='utf-8')
        home = root / 'home/arcnix'
        home.mkdir(parents=True, exist_ok=True)
        prefs_file = home / '.arcnix/desktop.json'
        prefs_file.parent.mkdir(exist_ok=True)
        prefs_file.write_text(json.dumps({
            'version': 1, 'theme': 'Dark', 'files': {'folder_icons': {'/Inbox': 'cloud'}},
        }), encoding='utf-8')

        def prefs():
            return read_desktop_settings(prefs_file).get('files', {})

        with Desktop(out / 'desktop', peer.config([peer.profile('Files management')], out)) as app:
            page = app.page
            app.connect()
            app.application('Files')

            def action(name, command):
                app.file(name).click(button='right')
                app.action(command)

            action('Inbox', 'Customize folder…')
            picker = page.get_by_role('dialog', name='Customize folder', exact=True)
            search = picker.get_by_role('textbox', name='Search icons', exact=True)
            tiles = picker.locator('.folder-icon-option')
            expect(search).to_be_focused()
            expect(picker.locator('.folder-icon-group')).to_have_count(5)
            expect(tiles).to_have_count(45)
            assert tiles.evaluate_all("buttons => buttons.every(button => !button.textContent.trim() && button.getAttribute('aria-label'))")
            assert tiles.locator('.action-glyph').evaluate_all("icons => icons.every(icon => getComputedStyle(icon).maskImage.startsWith('url('))")
            picker.screenshot(path=str(out / 'folder-icon-picker.png'))
            search.fill('  CLOUD  ')
            expect(tiles).to_have_count(1)
            cloud = picker.get_by_role('button', name='Cloud', exact=True)
            cloud.click()
            expect(cloud).to_have_attribute('aria-pressed', 'true')
            expect(picker.locator('.folder-icon-preview .folder-icon-cloud')).to_be_visible()
            eventually(lambda: prefs().get('folder_icons', {}).get('/Inbox') == 'cloud')
            search.fill('no-icon-has-this-keyword')
            expect(tiles).to_have_count(0)
            expect(picker.get_by_text('No matching icons', exact=True)).to_be_visible()
            search.fill('云盘')
            expect(tiles).to_have_count(1)
            picker.get_by_role('button', name='Done', exact=True).click()
            app.application('Settings')
            panel = page.locator('.desktop-settings-page')
            panel.get_by_role('button', name='Appearance', exact=True).click()
            panel.get_by_role('button', name='Color icons', exact=True).click()
            expect(page.locator('.server-desktop')).to_have_attribute('data-desktop-icon-family', 'color')
            app.application('Files')
            folder_mark = app.file('Inbox').locator('.folder-icon-cloud')
            expect(folder_mark).to_be_visible()
            expect(folder_mark).to_have_css('background-image', 'linear-gradient(145deg, rgb(135, 231, 255), rgb(66, 159, 213) 48%, rgb(49, 101, 186))')
            default_folder = app.file('home').locator('.glyph-folder')
            expect(default_folder).to_have_css('mask-image', 'none')
            assert default_folder.evaluate("icon => getComputedStyle(icon).backgroundImage.startsWith('url(')")
            action('Inbox', 'Customize folder…')
            expect(search).to_have_value('')
            expect(cloud.locator('.folder-icon-cloud')).to_be_visible()
            assert tiles.locator('.action-glyph').evaluate_all('''icons => icons.every(icon => {
                const style = getComputedStyle(icon);
                return style.maskImage === style.getPropertyValue('--folder-color-mask')
                    && style.backgroundImage.startsWith('linear-gradient(');
            })''')
            picker.screenshot(path=str(out / 'folder-icon-picker-color.png'))
            eventually(lambda: prefs().get('folder_icons', {}).get('/Inbox') == 'cloud')
            expect(cloud).to_have_attribute('aria-pressed', 'true')
            picker.get_by_role('button', name='Done', exact=True).click()

            action('notes.txt', 'Duplicate')
            expect(app.file('notes copy.txt')).to_be_visible()
            assert (root / 'notes copy.txt').read_bytes() == notes.read_bytes()
            action('Inbox', 'Duplicate')
            expect(app.file('Inbox copy')).to_be_visible()
            assert (root / 'Inbox copy/empty').is_dir()
            action('Inbox', 'Compress')
            expect(app.file('Inbox.zip')).to_be_visible()
            with ZipFile(root / 'Inbox.zip') as archive:
                assert archive.namelist() == ['Inbox/', 'Inbox/empty/']
            action('notes.txt', 'Compress')
            expect(app.file('notes.txt.zip')).to_be_visible()
            with ZipFile(root / 'notes.txt.zip') as archive:
                assert archive.read('notes.txt') == notes.read_bytes()

            action('Inbox', 'Make alias')
            expect(app.file('Inbox alias')).to_be_visible()
            action('Inbox alias', 'Open alias')
            expect(app.file('empty')).to_be_visible()
            active = page.locator('.desktop-window.app-files.active')
            active.locator('.files-sidebar').get_by_role('button', name='Filesystem', exact=True).click()
            action('notes.txt', 'Copy')
            action('Inbox', 'Open in new tab')
            expect(active.locator('.breadcrumb.current')).to_have_attribute('aria-label', '/Inbox')
            expect(app.file('empty')).to_be_visible()
            active.locator('.files-pane .file-list-container').press('Control+v')
            eventually(lambda: (inbox / 'notes.txt').is_file())
            expect(app.file('notes.txt')).to_be_visible()
            assert (inbox / 'notes.txt').read_bytes() == notes.read_bytes()

            def move_pointer(source, x, y, *, copy=False, position=(20, 10)):
                box = source.bounding_box()
                start_x = box['x'] + min(box['width'] / 2, position[0])
                start_y = box['y'] + min(box['height'] / 2, position[1])
                if copy:
                    page.keyboard.down('Control')
                try:
                    page.mouse.move(start_x, start_y)
                    page.mouse.down()
                    page.mouse.move(start_x + 9, start_y + 2, steps=4)
                    page.mouse.move(x, y, steps=16)
                    page.mouse.move(x + 1, y, steps=1)
                    page.mouse.up()
                finally:
                    if copy:
                        page.keyboard.up('Control')

            canvas = page.locator('.desktop-canvas').bounding_box()
            move_pointer(active.locator('.files-tab-select').last, canvas['x'] + 5, canvas['y'] + 5)
            windows = page.locator('.desktop-window.app-files:not(.hidden)')
            expect(windows).to_have_count(2)
            expect(windows.locator('.files-sidebar')).to_have_count(2)
            inactive = page.locator('.desktop-window.app-files:not(.active):not(.hidden)')
            title = active.locator('.desktop-window-title')
            title_box = title.bounding_box()
            move_pointer(title, title_box['x'] + 550, title_box['y'] + 10)

            def drop(source, destination, *, copy=False, x=24, y=10):
                box = destination.bounding_box()
                move_pointer(source, box['x'] + x, box['y'] + y, copy=copy)

            source = inactive.locator('.files-inactive-entry[aria-label="/move.txt"]')
            target = active.locator('.files-pane')
            drop(source, target, x=180, y=170)
            expect(app.file('move.txt')).to_be_visible()
            expect(source).to_have_count(0)
            assert (inbox / 'move.txt').read_text() == 'move.txt'
            assert not (root / 'move.txt').exists()

            move = active.locator('.file-row[aria-label="/Inbox/move.txt"]')
            home_target = inactive.locator('.files-sidebar').get_by_role('button', name='Home', exact=True)
            drop(move, home_target, copy=True)
            eventually(lambda: (home / 'move.txt').exists())
            assert (inbox / 'move.txt').read_text() == (home / 'move.txt').read_text()

            pin = inactive.locator('.files-inactive-entry[aria-label="/pin.txt"]')
            favorites = active.locator('.files-favorites')
            drop(pin, favorites)
            eventually(lambda: '/pin.txt' in prefs().get('favorites', []))
            assert (root / 'pin.txt').exists()

            tagged = inactive.locator('.files-inactive-entry[aria-label="/tag.txt"]')
            blue = active.locator('.files-sidebar').get_by_role('button', name='Blue', exact=True)
            drop(tagged, blue)
            eventually(lambda: prefs().get('colors', {}).get('/tag.txt') == 'blue')

            trash_source = inactive.locator('.files-inactive-entry[aria-label="/trash.txt"]')
            trash = active.locator('.files-sidebar').get_by_role('button', name='Trash', exact=True)
            drop(trash_source, trash)
            expect(trash_source).to_have_count(0)
            assert not (root / 'trash.txt').exists()
            expect(windows.locator('.files-sidebar')).to_have_count(2)
            print('Duplicate, ZIP, alias, clipboard, cross-window move/copy and sidebar drops passed.')


if __name__ == '__main__':
    main()
