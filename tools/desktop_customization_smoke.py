"""Desktop layout, wallpaper and Dock backed by the connected user's SSH home."""
import base64
import hashlib
import json
import time

from desktop_fixture import Desktop, Peer, ROOT, expect, eventually


def main():
    out = ROOT / 'artifacts/desktop-customization' / f'run-{time.time_ns()}'
    local = out / 'local'
    local.mkdir(parents=True)
    local_image = local / 'photo.png'
    local_image.write_bytes(base64.b64decode(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jR3sAAAAASUVORK5CYII='))
    with Peer(out / 'peer') as peer:
        home = peer.root / 'home/arcnix'
        desktop = home / 'Desktop'
        (desktop / 'Projects').mkdir(parents=True, exist_ok=True)
        (desktop / 'Projects' / 'plan.txt').write_text('sample plan', encoding='utf-8')
        (desktop / 'notes.txt').write_text('desktop notes', encoding='utf-8')
        (desktop / 'pattern.svg').write_text(
            '<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="720">'
            '<rect width="1280" height="720" fill="#215978"/></svg>', encoding='utf-8')
        (peer.root / 'Desktop').mkdir(exist_ok=True)
        (peer.root / 'Desktop' / 'outside-home.txt').write_text('not in the login home')
        settings_file = home / '.arcnix/desktop.json'

        def preferences():
            return json.loads(settings_file.read_text())['appearance']

        with Desktop(out / 'desktop', peer.config([peer.profile('Customization fixture')], local)) as app:
            p = app.page
            errors = []
            p.on('pageerror', lambda error: errors.append(str(error)))

            def desktop_button(label):
                return p.locator('.desktop-shortcuts').get_by_role('button', name=label, exact=True)

            def dock_button(label):
                return p.locator('.desktop-dock').get_by_role('button', name=label, exact=True)

            def settings():
                return p.get_by_role('dialog', name='Desktop settings', exact=True)

            def saved():
                expect(settings().get_by_text('Saved on server', exact=True)).to_be_visible()

            def close_settings():
                settings().get_by_role('button', name='Close dialog', exact=True).click()

            def menu(label):
                return p.locator('.desktop-context-menu').get_by_role('button', name=label, exact=True)

            def background_menu():
                p.locator('.desktop-canvas').click(button='right', position={'x': 565, 'y': 188})

            def wallpaper_loaded(width):
                return p.locator('.desktop-wallpaper-image').evaluate('(img) => img.complete && img.naturalWidth') == width

            app.connect()
            app.workspace('Desktop')
            expect(desktop_button('Projects')).to_be_visible()
            expect(desktop_button('outside-home.txt')).to_have_count(0)
            expect(p.locator('.desktop-wallpaper-address')).to_contain_text('127.0.0.1:')
            desktop_button('Software store').click()
            expect(p.locator('.desktop-window:not(.hidden)')).to_be_visible()
            expect(desktop_button('Terminal')).to_be_visible()
            assert p.locator('.desktop-window').evaluate('el => el.offsetLeft') == 22
            dock_button('Show desktop').click()
            desktop_button('Projects').click()
            expect(app.file('plan.txt')).to_be_visible()
            dock_button('Show desktop').click()

            notes = desktop_button('notes.txt')
            notes.drag_to(desktop_button('Terminal'), target_position={'x': 8, 'y': 30})
            eventually(lambda: preferences()['order'][0] == {'kind': 'file', 'id': 'notes.txt'})
            expect(p.locator('.desktop-shortcut').first).to_have_attribute('aria-label', 'notes.txt')
            notes.click()
            expect(p.locator('.remote-editor-content')).to_have_value('desktop notes')
            p.locator('.window-close').click()

            desktop_button('Monitor').click(button='right')
            menu('Hide desktop shortcut').click()
            expect(desktop_button('Monitor')).to_have_count(0)
            background_menu()
            menu('Desktop settings').click()
            settings().get_by_role('button', name='Show server info', exact=True).click()
            saved()
            expect(p.locator('.desktop-wallpaper-copy')).to_have_count(0)
            settings().get_by_role('button', name='Show Arcnix shortcuts', exact=True).click()
            saved()
            expect(desktop_button('Terminal')).to_have_count(0)
            expect(notes).to_be_visible()
            settings().get_by_role('button', name='Reset desktop icons', exact=True).click()
            saved()
            expect(desktop_button('Monitor')).to_be_visible()
            settings().get_by_role('button', name='Wallpaper', exact=True).click()
            settings().get_by_role('button', name='Aurora', exact=True).click()
            saved()
            expect(p.locator('.desktop-wallpaper-art')).to_have_class('desktop-wallpaper-art wallpaper-aurora')
            settings().get_by_role('textbox', name='Server image path').fill('~/Desktop/pattern.svg')
            settings().get_by_role('button', name='Apply', exact=True).click()
            eventually(lambda: preferences()['wallpaper'] == {'remote': '/home/arcnix/Desktop/pattern.svg'})
            eventually(lambda: wallpaper_loaded(1280))
            settings().get_by_role('textbox', name='Local image path').fill(str(local_image))
            settings().get_by_role('button', name='Upload & apply', exact=True).click()
            upload_name = hashlib.sha256(local_image.read_bytes()).hexdigest() + '.png'
            remote_image = f'/home/arcnix/.arcnix/wallpapers/{upload_name}'
            eventually(lambda: preferences()['wallpaper'] == {'remote': remote_image})
            eventually(lambda: wallpaper_loaded(1))
            assert (home / '.arcnix/wallpapers' / upload_name).read_bytes() == local_image.read_bytes()
            assert not (home / 'arcnix/.arcnix').exists()
            close_settings()
            desktop_button('Projects').drag_to(desktop_button('Terminal'), target_position={'x': 8, 'y': 30})
            eventually(lambda: preferences()['order'][:1] == [{'kind': 'file', 'id': 'Projects'}])
            dock_button('Terminal').click(button='right')
            menu('Dock settings').click()
            settings().get_by_role('button', name='Large', exact=True).click()
            saved()
            expect(p.locator('.desktop-dock')).to_have_class('desktop-dock dock-large dock-magnify')
            settings().get_by_role('button', name='Magnify Dock icons', exact=True).click()
            saved()
            settings().get_by_role('button', name='Automatically hide Dock', exact=True).click()
            saved()
            close_settings()
            canvas = p.locator('.desktop-canvas').bounding_box()
            p.mouse.move(canvas['x'] + canvas['width'] / 2, canvas['y'] + canvas['height'] - 4)
            eventually(lambda: p.locator('.desktop-dock').bounding_box()['y'] < canvas['y'] + canvas['height'] - 30)
            dock_button('Editor').drag_to(dock_button('Terminal'), target_position={'x': 5, 'y': 22})
            eventually(lambda: preferences()['dock']['order'][:1] == ['editor'])
            dock_button('Monitor').click(button='right')
            menu('Remove from Dock').click()
            eventually(lambda: 'monitor' in preferences()['dock']['unpinned'])
            expect(dock_button('Monitor')).to_have_count(0)

            app.stop()
            app.launch()
            p = app.page
            p.on('pageerror', lambda error: errors.append(str(error)))
            app.connect()
            app.workspace('Desktop')
            eventually(lambda: wallpaper_loaded(1))
            expect(p.locator('.desktop-wallpaper-copy')).to_have_count(0)
            expect(desktop_button('Projects')).to_be_visible()
            expect(p.locator('.desktop-shortcut').first).to_have_attribute('aria-label', 'Projects')
            expect(p.locator('.desktop-dock-app').first).to_have_attribute('aria-label', 'Editor')
            expect(p.locator('.desktop-dock')).to_have_class('desktop-dock dock-large dock-auto-hide')
            expect(dock_button('Monitor')).to_have_count(0)
            assert not errors, errors
            app.screenshot('restored.png')
    print(json.dumps(dict(output=str(out), checks=[
        'A default floating window uses the standard inset and can cover shortcuts; login HOME files open in Explorer or Editor',
        'Desktop icon ordering, visibility toggles and reset use the remote preference file',
        'Built-in wallpaper, remote SVG and uploaded local PNG render through the WebView',
        'Dock reorder, size, auto-hide, magnification and unpin survive restart',
    ]), indent=2))


if __name__ == '__main__':
    main()
