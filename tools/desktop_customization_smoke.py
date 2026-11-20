"""Desktop layout, wallpaper and Dock backed by the connected user's SSH home."""
import base64
import hashlib
import json
import re
import time

from server_apps_fixture import desktop_settings_dir, read_desktop_settings
from desktop_fixture import Desktop, Peer, ROOT, close_dialog, expect, eventually


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
        long_name = '服务器运维配置记录及备份校验结果详细归档-2026-11-18.txt'
        (desktop / long_name).write_text('long shortcut label', encoding='utf-8')
        (desktop / 'pattern.svg').write_text(
            '<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="720">'
            '<rect width="1280" height="720" fill="#215978"/></svg>', encoding='utf-8')
        (peer.root / 'Desktop').mkdir(exist_ok=True)
        (peer.root / 'Desktop' / 'outside-home.txt').write_text('not in the login home')
        settings_file = home / '.arcnix/desktop.json'
        settings_file.parent.mkdir(exist_ok=True)
        settings_file.write_text(json.dumps({
            'version': 1, 'theme': 'System',
            'appearance': {'wallpaper': 'dunes', 'dock': {'magnify': True}},
            'files': {'folder_icons': {'/home/arcnix/Desktop/Projects': 'projects'}},
        }), encoding='utf-8')

        def preferences():
            return read_desktop_settings(settings_file)['appearance']

        with Desktop(out / 'desktop', peer.config([peer.profile('Customization fixture')], local)) as app:
            p = app.page
            errors = []
            p.on('pageerror', lambda error: errors.append(str(error)))

            def desktop_button(label):
                return p.locator('.desktop-shortcuts').get_by_role('button', name=label, exact=True)

            def dock_button(label):
                return p.locator('.desktop-dock').get_by_role('button', name=label, exact=True)

            def check_hover(button, clipped=False):
                button.hover()
                layout = button.evaluate('''el => {
                    const frame = el.getBoundingClientRect();
                    const cell = el.closest('.desktop-icon-slot').getBoundingClientRect();
                    const icon = el.querySelector('.desktop-app-icon').getBoundingClientRect();
                    const label = el.querySelector('.desktop-app-label');
                    const text = label.getBoundingClientRect();
                    return {
                        margins: [icon.top - frame.top, frame.bottom - text.bottom],
                        inCell: frame.bottom <= cell.bottom && frame.width < cell.width,
                        labelInFrame: text.left >= frame.left && text.right <= frame.right,
                        painted: getComputedStyle(el).backgroundColor !== 'rgba(0, 0, 0, 0)',
                        clipped: label.scrollHeight > label.clientHeight,
                    };
                }''')
                top, bottom = layout['margins']
                assert min(top, bottom) >= 3 and abs(top - bottom) <= 2, layout
                assert layout['inCell'] and layout['labelInFrame'] and layout['painted'], layout
                assert layout['clipped'] == clipped, layout

            def settings():
                return p.locator('.desktop-settings-page')

            def shortcut_options(title):
                settings().get_by_role('button', name=title, exact=True).click()
                dialog = p.get_by_role('dialog', name=title, exact=True)
                expect(dialog).to_be_visible()
                return dialog

            def check_dock_corners():
                active = p.locator('.desktop-dock-app.active')
                assert active.evaluate('''button => {
                    const icon = button.querySelector('.desktop-app-icon');
                    const radius = getComputedStyle(icon).borderTopLeftRadius;
                    const ratio = parseFloat(radius) / (radius.endsWith('%') ? 100 : icon.offsetWidth);
                    return ratio >= .15 && ratio <= .3;
                }''')

            def check_dock_selection():
                check_dock_corners()
                active = p.locator('.desktop-dock-app.active')
                assert active.evaluate('''button => {
                    const icon = button.querySelector('.desktop-app-icon').getBoundingClientRect();
                    const tile = button.getBoundingClientRect();
                    const dock = button.closest('.desktop-dock').getBoundingClientRect();
                    const zone = button.closest('.desktop-dock-zone');
                    const indicator = getComputedStyle(button, '::after');
                    const margins = [icon.top - tile.top, icon.left - tile.left,
                        tile.bottom - icon.bottom, tile.right - icon.right];
                    const left = zone.classList.contains('dock-left');
                    const right = zone.classList.contains('dock-right');
                    const center = left ? tile.left + parseFloat(indicator.left)
                        : right ? tile.right - parseFloat(indicator.right)
                        : tile.bottom - parseFloat(indicator.bottom);
                    const target = left ? (dock.left + icon.left) / 2
                        : right ? (dock.right + icon.right) / 2
                        : (dock.bottom + icon.bottom) / 2;
                    const [width, height] = [parseFloat(indicator.width), parseFloat(indicator.height)];
                    return margins.every(margin => Math.abs(margin / icon.width - .12) < .03)
                        && Math.abs(center - target) <= 1
                        && ((left || right) ? height > width : width > height)
                        && indicator.content !== 'none';
                }''')
                assert p.locator('.desktop-dock').evaluate(
                    'el => el.scrollWidth <= el.clientWidth && el.scrollHeight <= el.clientHeight')

            def check_centered_dock(tray):
                active = tray.locator('.desktop-dock-app.active')
                assert active.evaluate('''button => {
                    const icon = button.querySelector('.desktop-app-icon');
                    const bar = button.closest('.desktop-dock').getBoundingClientRect();
                    const glyph = icon.getBoundingClientRect();
                    const top = icon.offsetTop;
                    const bottom = button.clientHeight - top - icon.offsetHeight;
                    return Math.abs(top - bottom) <= 1
                        && Math.abs((bar.top + bar.bottom - glyph.top - glyph.bottom) / 2) <= 1;
                }''')

            def saved():
                expect(settings().locator('.desktop-settings-status')).to_be_empty()

            def close_settings():
                p.locator('.desktop-window.active .window-close').click()
                expect(settings()).to_have_count(0)

            def menu(label):
                return p.locator('.desktop-context-menu').get_by_role('button', name=label, exact=True)

            def background_menu():
                p.locator('.desktop-canvas').click(button='right', position={'x': 565, 'y': 188})

            def wallpaper_loaded(width):
                return p.locator('.desktop-wallpaper-image').evaluate('(img) => img.complete && img.naturalWidth') == width

            app.connect()
            app.workspace('Desktop')
            default_apps = ['Files', 'Terminal', 'Code', 'Web browser', 'Player', 'Monitor', 'App Center', 'Settings', 'Text Editor']
            expect(p.locator('.desktop-shortcuts .desktop-app-label').filter(
                has_text=re.compile('^(' + '|'.join(default_apps) + ')$'),
            )).to_have_text(default_apps)
            expect(p.locator('.desktop-dock-app .desktop-app-label')).to_have_text(
                [label for label in default_apps if label not in ('Player', 'Text Editor')],
            )
            bubble = p.locator('.desktop-context-menu')
            for label in ('Terminal', 'Settings'):
                launcher = dock_button(label)
                launcher.click(button='right')
                expect(bubble).to_have_attribute('data-dock-menu-side', 'above')
                icon_box = launcher.bounding_box()
                bubble_box = bubble.bounding_box()
                assert bubble_box['y'] + bubble_box['height'] < icon_box['y']
                arrow = float(bubble.evaluate("el => el.style.getPropertyValue('--dock-menu-arrow').slice(0, -2)"))
                assert abs(bubble_box['x'] + arrow - icon_box['x'] - icon_box['width'] / 2) < 2
                if label == 'Settings':
                    app.screenshot('dock-launcher-menu.png')
                p.keyboard.press('Escape')
                expect(bubble).to_have_count(0)
            expect(p.locator('.desktop-wallpaper-art')).to_have_class('desktop-wallpaper-art wallpaper-fjord')
            expect(desktop_button('Projects')).to_be_visible()
            project_icon = desktop_button('Projects').locator('.folder-icon-outline-projects')
            expect(project_icon).to_be_visible()
            assert project_icon.evaluate("icon => getComputedStyle(icon, '::before').maskImage.includes('data:image/svg+xml;base64,')")
            expect(desktop_button('outside-home.txt')).to_have_count(0)
            expect(p.locator('.desktop-wallpaper-address')).to_contain_text('127.0.0.1:')
            check_hover(desktop_button('Monitor'))
            check_hover(desktop_button(long_name), clipped=True)
            app.screenshot('shortcut-hover.png')
            p.set_viewport_size(dict(width=900, height=650))
            check_hover(desktop_button(long_name), clipped=True)
            p.set_viewport_size(dict(width=1440, height=980))
            desktop_button('App Center').click()
            expect(p.locator('.desktop-window:not(.hidden)')).to_be_visible()
            title = p.locator('.desktop-window.active .desktop-window-titlebar')
            assert title.evaluate("""element => !element.dispatchEvent(new MouseEvent('contextmenu', {
                bubbles: true, cancelable: true, button: 2,
            }))""")
            title.click(button='right')
            expect(p.locator('.desktop-context-menu')).to_have_count(0)
            expect(desktop_button('Terminal')).to_be_visible()
            assert p.locator('.desktop-window.active').evaluate('el => el.offsetLeft') == 22
            dock_button('Show desktop').click()
            desktop_button('Projects').click()
            expect(app.file('plan.txt')).to_be_visible()
            p.locator('.files-sidebar').get_by_role('button', name='Desktop', exact=True).click()
            expect(p.locator('.desktop-window.app-files.active .pathbar .breadcrumb.current')).to_have_attribute(
                'aria-label', '/home/arcnix/Desktop',
            )
            dock_button('Show desktop').click()

            notes = desktop_button('notes.txt')
            notes.drag_to(desktop_button('Files'), target_position={'x': 8, 'y': 30})
            eventually(lambda: preferences().get('order', [])[:1] == [{'kind': 'file', 'id': 'notes.txt'}])
            expect(p.locator('.desktop-shortcut').first).to_have_attribute('aria-label', 'notes.txt')
            notes.click()
            expect(p.locator('.app-editor.active .remote-editor-content')).to_have_value('desktop notes')
            p.locator('.desktop-window.active .window-close').click()

            desktop_button('Monitor').click(button='right')
            menu('Hide desktop shortcut').click()
            expect(desktop_button('Monitor')).to_have_count(0)
            background_menu()
            menu('Desktop settings').click()
            expect(settings().get_by_role('button', name='Desktop', exact=True)).to_have_attribute('aria-pressed', 'true')
            expect(settings().get_by_role('button', name='Pinned shortcuts', exact=True)).to_have_count(0)
            settings().get_by_role('button', name='Dock', exact=True).click()
            expect(settings().get_by_role('button', name='Visible icons', exact=True)).to_have_count(0)
            pinned = shortcut_options('Pinned shortcuts')
            expect(pinned.locator('.preference-check')).to_have_text(default_apps + ['Home', 'Trash'])
            expect(pinned.get_by_role('checkbox', name='Player', exact=True)).not_to_be_checked()
            expect(pinned.get_by_role('checkbox', name='Text Editor', exact=True)).not_to_be_checked()
            close_dialog(pinned)
            settings().get_by_role('button', name='Desktop', exact=True).click()
            settings().get_by_role('button', name='Show server info', exact=True).click()
            saved()
            expect(p.locator('.desktop-wallpaper-copy')).to_have_count(0)
            shortcuts = shortcut_options('Visible icons')
            monitor = shortcuts.get_by_role('checkbox', name='Monitor', exact=True)
            terminal = shortcuts.get_by_role('checkbox', name='Terminal', exact=True)
            expect(monitor).not_to_be_checked()
            expect(terminal).to_be_checked()
            terminal.uncheck()
            saved()
            close_dialog(shortcuts)
            expect(desktop_button('Terminal')).to_have_count(0)
            expect(desktop_button('Files')).to_be_visible()
            expect(dock_button('Terminal')).to_be_visible()
            expect(notes).to_be_visible()
            eventually(lambda: set(preferences()['hidden']) == {'monitor', 'terminal'})
            shortcut_options('Visible icons')
            monitor.check()
            saved()
            close_dialog(shortcuts)
            expect(desktop_button('Monitor')).to_be_visible()
            expect(desktop_button('Terminal')).to_have_count(0)
            eventually(lambda: preferences()['hidden'] == ['terminal'])
            shortcut_options('Visible icons')
            shortcuts.get_by_role('button', name='Reset desktop icons', exact=True).click()
            saved()
            expect(terminal).to_be_checked()
            close_dialog(shortcuts)
            expect(desktop_button('Monitor')).to_be_visible()
            expect(desktop_button('Terminal')).to_be_visible()
            settings().get_by_role('button', name='Appearance', exact=True).click()
            expect(settings().locator('.theme-preset-select')).to_have_text(['Fjord', 'Aurora', 'Terracotta'])
            terracotta = settings().get_by_role('button', name='Terracotta', exact=True)
            terracotta.click()
            saved()
            art = p.locator('.desktop-wallpaper-art.wallpaper-terracotta')
            expect(art).to_be_visible()
            assert (
                terracotta.locator('..').locator('.theme-wallpaper-swatch').evaluate('el => getComputedStyle(el).backgroundImage')
                == art.evaluate('el => getComputedStyle(el).backgroundImage')
            )
            settings().get_by_role('button', name='Aurora', exact=True).click()
            saved()
            expect(p.locator('.desktop-wallpaper-art')).to_have_class('desktop-wallpaper-art wallpaper-aurora')
            fjord = settings().get_by_role('button', name='Fjord', exact=True)
            fjord.click()
            eventually(lambda: preferences().get('themes', {}).get('selection', {}).get('value') == 'fjord')
            expect(p.locator('.desktop-wallpaper-art')).to_have_class('desktop-wallpaper-art wallpaper-fjord')
            assert fjord.locator('..').locator('.theme-wallpaper-swatch').evaluate("""async el => {
                const url = getComputedStyle(el).backgroundImage;
                if (!url.startsWith('url("data:image/webp;base64,')) return false;
                const image = new Image();
                image.src = url.slice(5, -2);
                await image.decode();
                return image.naturalWidth > 1200;
            }""")
            settings().get_by_role('button', name='Customize', exact=True).click()
            creator = p.get_by_role('dialog', name='Create theme', exact=True)
            creator.get_by_role('textbox', name='Theme name', exact=True).fill('Uploaded wallpaper')
            creator.get_by_role('button', name='Light', exact=True).click()
            creator.get_by_role('button', name='Wallpaper options', exact=True).click()
            creator.get_by_role('textbox', name='Server image path').fill('~/Desktop/pattern.svg')
            creator.get_by_role('button', name='Apply', exact=True).click()
            eventually(lambda: wallpaper_loaded(1280))
            creator.get_by_role('textbox', name='Local image path').fill(str(local_image))
            creator.get_by_role('button', name='Upload & apply', exact=True).click()
            upload_name = hashlib.sha256(local_image.read_bytes()).hexdigest() + '.png'
            remote_image = f'/home/arcnix/.arcnix/wallpapers/{upload_name}'
            eventually(lambda: wallpaper_loaded(1))
            creator.get_by_role('button', name='Create theme', exact=True).click()
            def uploaded_wallpaper_saved():
                themes = preferences().get('themes', {})
                return themes.get('selection') == {'kind': 'saved', 'value': 'Uploaded wallpaper'} and (
                    themes['saved']['Uploaded wallpaper']['overrides']['light']['wallpaper'] == {'remote': remote_image}
                )
            eventually(uploaded_wallpaper_saved)
            assert (home / '.arcnix/wallpapers' / upload_name).read_bytes() == local_image.read_bytes()
            assert not (home / 'arcnix/.arcnix').exists()
            close_settings()
            desktop_button('Projects').drag_to(desktop_button('Files'), target_position={'x': 8, 'y': 30})
            eventually(lambda: preferences()['order'][:1] == [{'kind': 'file', 'id': 'Projects'}])
            dock_button('Terminal').click(button='right')
            menu('Dock settings').click()
            expect(settings().get_by_role('button', name='Dock', exact=True)).to_have_attribute('aria-pressed', 'true')
            settings().locator('.desktop-setting-row').filter(has_text='Dock icon size').get_by_role('button', name='Large', exact=True).click()
            saved()
            expect(p.locator('.desktop-dock')).to_have_class('desktop-dock dock-large')
            expect(settings().get_by_role('button', name='Magnify Dock icons', exact=True)).to_have_count(0)
            settings().get_by_role('button', name='Automatically hide Dock', exact=True).click()
            saved()
            close_settings()
            canvas = p.locator('.desktop-canvas').bounding_box()
            p.mouse.move(canvas['x'] + canvas['width'] / 2, canvas['y'] + canvas['height'] - 4)
            eventually(lambda: p.locator('.desktop-dock').bounding_box()['y'] < canvas['y'] + canvas['height'] - 30)
            dock_button('Code').drag_to(dock_button('Files'), target_position={'x': 5, 'y': 22})
            eventually(lambda: preferences()['dock']['order'][:1] == ['code'])
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
            expect(p.locator('.desktop-dock-app').first).to_have_attribute('aria-label', 'Code')
            expect(p.locator('.desktop-dock')).to_have_class('desktop-dock dock-large dock-auto-hide')
            expect(dock_button('Monitor')).to_have_count(0)
            app.application('Settings')
            settings().get_by_role('button', name='Dock', exact=True).click()
            pinned = shortcut_options('Pinned shortcuts')
            expect(pinned.locator('.preference-check').first).to_have_text('Code')
            close_dialog(pinned)
            settings().get_by_role('button', name='Automatically hide Dock', exact=True).click()
            saved()
            settings().get_by_role('button', name='Panel mode', exact=True).click()
            saved()
            panel = p.locator('.dock-panel-zone > .desktop-dock')
            canvas = p.locator('.desktop-canvas')
            panel_box, canvas_box = panel.bounding_box(), canvas.bounding_box()
            assert abs(panel_box['x'] - canvas_box['x']) < 2
            assert abs(panel_box['width'] - canvas_box['width']) < 2
            assert abs(panel_box['y'] + panel_box['height'] - canvas_box['y'] - canvas_box['height']) < 2
            clock = panel.locator('.desktop-panel-clock')
            expect(clock.locator('.desktop-panel-time')).not_to_be_empty()
            expect(clock.locator('.desktop-panel-date')).not_to_be_empty()
            assert clock.bounding_box()['x'] > panel_box['x'] + panel_box['width'] / 2
            check_dock_selection()

            align_left = settings().get_by_role('button', name='Align left', exact=True)
            center_icons = settings().get_by_role('button', name='Center icons', exact=True)
            leading = panel.locator('.desktop-dock-leading')
            apps = panel.locator('.desktop-dock-apps')
            expect(align_left).to_have_attribute('aria-pressed', 'true')
            leading_box = leading.bounding_box()
            assert 0 < apps.bounding_box()['x'] - leading_box['x'] - leading_box['width'] < 12
            center_icons.click()
            eventually(lambda: preferences()['dock'].get('panel_centered') is True)
            expect(center_icons).to_have_attribute('aria-pressed', 'true')
            apps_box = apps.bounding_box()
            assert abs(apps_box['x'] + apps_box['width'] / 2 - panel_box['x'] - panel_box['width'] / 2) < 2
            expect(apps).to_have_css('border-top-width', '0px')
            expect(apps).to_have_css('background-color', 'rgba(0, 0, 0, 0)')
            expect(apps).to_have_css('box-shadow', 'none')
            assert panel.evaluate('''el => Math.abs(parseFloat(getComputedStyle(el).paddingBlockStart)
                / el.querySelector('.desktop-app-icon').getBoundingClientRect().width - .045) < .01''')
            check_centered_dock(apps)
            assert abs(panel.get_by_role('button', name='Show desktop').bounding_box()['x'] - panel_box['x']) < 15
            p.set_viewport_size(dict(width=760, height=660))
            expect(settings()).to_have_css('flex-direction', 'column')
            assert apps.bounding_box()['x'] + apps.bounding_box()['width'] < clock.bounding_box()['x']
            check_dock_selection()
            check_centered_dock(apps)
            p.set_viewport_size(dict(width=1440, height=980))
            expect(settings()).to_have_css('flex-direction', 'row')

            slider = settings().get_by_role('slider', name='Dock icon size', exact=True)
            dock_size = settings().locator('.desktop-setting-row').filter(has_text='Dock icon size')
            for label, pixels in [('Small', 32), ('Medium', 48), ('Large', 56)]:
                dock_size.get_by_role('button', name=label, exact=True).click()
                saved()
                expect(slider).to_have_value(str(pixels))
                check_dock_selection()
            track = slider.bounding_box()
            for value, x, progress in [(32, 1, '0%'), (64, track['width'] - 1, '100%')]:
                slider.click(position={'x': x, 'y': track['height'] / 2})
                expect(slider).to_have_value(str(value))
                assert slider.evaluate("el => el.style.getPropertyValue('--icon-size-progress')") == progress
                if value == 32:
                    eventually(lambda: preferences()['dock']['size'] == 'small' and 'icon_size' not in preferences()['dock'])
                else:
                    eventually(lambda: preferences()['dock'].get('icon_size') == value)
                check_dock_corners()
            dock_size.get_by_role('button', name='Large', exact=True).click()
            eventually(lambda: preferences()['dock']['size'] == 'large' and 'icon_size' not in preferences()['dock'])
            expect(slider).to_have_value('56')
            track = slider.bounding_box()
            y = track['y'] + track['height'] / 2
            p.mouse.move(track['x'] + track['width'] * .75, y)
            p.mouse.down()
            p.mouse.move(track['x'] + track['width'] * .9, y, steps=6)
            custom_pixels = eventually(lambda: slider.evaluate('''slider => {
                const dock = slider.closest('.desktop-canvas').querySelector('.desktop-dock');
                const size = slider.valueAsNumber;
                return size > 56 && size < 64 && dock.style.getPropertyValue('--dock-preview') === `${size}px`
                    ? size : null;
            }'''))
            assert 'icon_size' not in preferences()['dock']
            p.mouse.up()
            eventually(lambda: preferences()['dock'].get('icon_size') == custom_pixels)
            expect(panel).to_have_class(f'desktop-dock dock-size-{custom_pixels} dock-centered')
            app.screenshot('panel-dock.png')

            saved()
            app.stop()
            layout_path = desktop_settings_dir(settings_file) / 'preferences/desktop.json'
            layout = json.loads(layout_path.read_bytes())
            layout_path.write_text(json.dumps(dict(layout, show_apps=False)), encoding='utf-8')
            app.launch()
            p = app.page
            p.on('pageerror', lambda error: errors.append(str(error)))
            app.connect()
            app.workspace('Desktop')
            expect(p.locator('.dock-panel-zone > .desktop-dock')).to_have_class(f'desktop-dock dock-size-{custom_pixels} dock-centered')
            expect(p.locator('.desktop-panel-time')).not_to_be_empty()
            app.application('Settings')
            settings().get_by_role('button', name='Desktop', exact=True).click()
            expect(desktop_button('Files')).to_have_count(0)
            shortcuts = shortcut_options('Visible icons')
            files_shortcut = shortcuts.get_by_role('checkbox', name='Files', exact=True)
            expect(files_shortcut).not_to_be_checked()
            files_shortcut.check()
            saved()
            close_dialog(shortcuts)
            expect(desktop_button('Files')).to_be_visible()
            expect(desktop_button('Terminal')).to_have_count(0)
            expect(desktop_button('Settings')).to_have_count(0)
            eventually(lambda: {'terminal', 'settings', 'docker'} <= set(preferences()['hidden']))
            settings().get_by_role('button', name='Dock', exact=True).click()
            expect(settings().get_by_role('button', name='Center icons', exact=True)).to_have_attribute('aria-pressed', 'true')
            expect(settings().get_by_role('button', name='Show Dock status bar', exact=True)).to_have_count(0)
            settings().get_by_role('button', name='Align left', exact=True).click()
            eventually(lambda: 'panel_centered' not in preferences()['dock'])
            expect(p.locator('.dock-panel-zone > .desktop-dock')).to_have_class(f'desktop-dock dock-size-{custom_pixels}')
            settings().get_by_role('button', name='Floating Dock', exact=True).click()
            saved()
            expect(p.locator('.dock-panel-zone')).to_have_count(0)
            expect(p.locator('.desktop-panel-time')).to_have_count(0)
            expect(settings().get_by_role('button', name='Center icons', exact=True)).to_have_count(0)
            slider = settings().get_by_role('slider', name='Dock icon size', exact=True)
            expect(slider).to_have_value(str(custom_pixels))
            check_dock_selection()
            check_centered_dock(p.locator('.desktop-dock'))
            status_toggle = settings().get_by_role('button', name='Show Dock status bar', exact=True)
            expect(status_toggle).to_have_attribute('aria-pressed', 'false')
            status_toggle.click()
            saved()
            status = p.locator('.desktop-dock-status')

            def check_clock_inset():
                assert status.evaluate('''el => {
                    const dock = el.closest('.desktop-dock');
                    const rect = node => node.getBoundingClientRect();
                    const frame = rect(dock);
                    const home = rect(dock.querySelector('.desktop-dock-home .action-glyph'));
                    const clock = rect(el.querySelector('.desktop-panel-clock'));
                    const [start, end] = [...dock.querySelectorAll('.desktop-dock-divider')].map(rect);
                    const icons = [...dock.querySelectorAll('.desktop-dock-app .desktop-app-icon')].map(rect);
                    const side = dock.closest('.dock-side-zone') !== null;
                    const [before, after, cross, span] = side
                        ? ['top', 'bottom', 'left', 'width'] : ['left', 'right', 'top', 'height'];
                    const same = (a, b) => Math.abs(a - b) <= 1;
                    const startInset = home[before] - frame[before];
                    const endInset = frame[after] - clock[after];
                    const corner = parseFloat(getComputedStyle(dock).borderTopLeftRadius);
                    return Math.min(startInset, endInset) >= 6
                        && corner < Math.min(frame.width, frame.height) / 3
                        && same(startInset, endInset)
                        && same(start[span], end[span]) && same(start[cross], end[cross])
                        && same(icons[0][before] - start[after], end[before] - icons.at(-1)[after]);
                }''')

            expect(status.locator('.desktop-dock-divider')).to_be_visible()
            check_clock_inset()
            radii = []
            for key, pixels in [('Home', 32), ('End', 64)]:
                slider.press(key)
                expect(slider).to_have_value(str(pixels))
                if pixels == 32:
                    eventually(lambda: preferences()['dock']['size'] == 'small' and 'icon_size' not in preferences()['dock'])
                else:
                    eventually(lambda: preferences()['dock'].get('icon_size') == pixels)
                check_clock_inset()
                radii.append(status.evaluate(
                    'el => parseFloat(getComputedStyle(el.closest(".desktop-dock")).borderTopLeftRadius)'
                ))
            assert radii[0] < radii[1], radii
            expect(status.locator('.action-glyph')).to_have_count(0)
            expect(status).not_to_contain_text(re.compile(r'\b(?:SSH|SFTP)\b'))
            clock_time = status.locator('.desktop-panel-time')
            expect(clock_time).to_have_text(re.compile(r'\b\d{1,2}:\d{2}:\d{2}\b'))
            expect(clock_time).not_to_have_text(clock_time.text_content(), timeout=3000)
            expect(status.locator('.desktop-panel-date')).not_to_be_empty()
            assert status.evaluate('''el => {
                const time = el.querySelector('.desktop-panel-time').getBoundingClientRect();
                const date = el.querySelector('.desktop-panel-date').getBoundingClientRect();
                return date.top > time.top && Math.abs(date.right - time.right) < 1;
            }''')
            assert preferences()['dock']['floating_status'] is True
            active_dock_button = dock_button('Settings')
            dock_icon = active_dock_button.locator('.desktop-app-icon')
            icon_box = dock_icon.bounding_box()
            active_dock_button.hover()
            expect(dock_icon).to_have_css('transform', 'none')
            assert dock_icon.bounding_box() == icon_box
            p.mouse.move(25, 100)
            app.screenshot('floating-dock-status.png')
            settings().get_by_role('button', name='Panel mode', exact=True).click()
            saved()
            expect(status).to_have_count(0)
            expect(p.locator('.desktop-panel-time')).to_have_text(re.compile(r'\b\d{1,2}:\d{2}:\d{2}\b'))
            settings().get_by_role('button', name='Floating Dock', exact=True).click()
            saved()
            expect(status_toggle).to_have_attribute('aria-pressed', 'true')
            expect(status).to_be_visible()
            expect(settings()).to_have_css('flex-direction', 'row')
            p.set_viewport_size(dict(width=680, height=660))
            expect(settings()).to_have_css('flex-direction', 'column')
            check_dock_selection()
            check_clock_inset()
            assert status.evaluate('el => el.scrollWidth <= el.clientWidth')
            p.set_viewport_size(dict(width=760, height=660))
            tabs = settings().locator('.desktop-settings-tabs')
            preferences_pane = settings().locator('.desktop-preferences')
            for label in ('General', 'Network', 'Desktop', 'Dock', 'Appearance', 'Keyboard shortcuts', 'About'):
                tab = tabs.get_by_role('button', name=label, exact=True)
                tab.click()
                expect(tab).to_have_attribute('aria-pressed', 'true')
                expect(preferences_pane.locator('.desktop-settings-pane')).to_be_visible()
                assert preferences_pane.evaluate('el => el.scrollWidth <= el.clientWidth'), label
                assert preferences_pane.evaluate('el => el.scrollTop === 0'), label
                if label == 'Appearance':
                    settings().get_by_role('button', name='Customize', exact=True).click()
                    creator = p.get_by_role('dialog', name='Create theme', exact=True)
                    creator.get_by_role('button', name='Wallpaper options', exact=True).click()
                    expect(creator.get_by_role('textbox', name='Local image path')).to_be_editable()
                    app.screenshot('compact-wallpaper-settings.png')
                    creator.get_by_role('button', name='Cancel', exact=True).click()
            p.set_viewport_size(dict(width=1440, height=980))
            settings().get_by_role('button', name='Dock', exact=True).click()
            expect(settings().get_by_role('button', name='Bottom', exact=True)).to_have_attribute('aria-pressed', 'true')
            p.locator('.app-settings.active .window-maximize').click()

            def maximized_fits_dock(edge):
                return p.locator('.app-settings.maximized').evaluate('''(window, edge) => {
                    const canvas = window.closest('.desktop-canvas');
                    const frame = canvas.getBoundingClientRect();
                    const dock = canvas.querySelector('.desktop-dock-zone').getBoundingClientRect();
                    const box = window.getBoundingClientRect();
                    const near = (a, b) => Math.abs(a - b) <= 1;
                    return near(box.top, frame.top)
                        && near(box.left, edge === 'left' ? dock.right + 8 : frame.left)
                        && near(box.right, edge === 'right' ? dock.left - 8 : frame.right)
                        && near(box.bottom, edge === 'bottom' ? dock.top - 8 : frame.bottom);
                }''', edge)

            eventually(lambda: maximized_fits_dock('bottom'))
            bubble = p.locator('.desktop-context-menu')
            for edge, menu_side in [('left', 'right'), ('right', 'left')]:
                settings().get_by_role('button', name=edge.title() + ' side', exact=True).click()
                zone = p.locator(f'.desktop-dock-zone.dock-{edge}')
                expect(zone).to_be_visible()
                eventually(lambda: preferences()['dock'].get('position') == edge)
                dock = zone.locator('.desktop-dock')
                expect(dock).to_have_css('flex-direction', 'column')
                assert dock.evaluate('''dock => {
                    const canvas = dock.closest('.desktop-canvas').getBoundingClientRect();
                    const box = dock.getBoundingClientRect();
                    return box.height < canvas.height && Math.abs(box.top + box.bottom - canvas.top - canvas.bottom) < 2;
                }''')
                check_dock_selection()
                side_clock = dock.locator('.desktop-panel-clock')
                check_clock_inset()
                expect(side_clock.locator('.desktop-panel-time')).to_have_text(re.compile(r'^\d{2}:\d{2}$'))
                expect(side_clock.locator('.desktop-panel-date')).to_have_text(re.compile(r'^\d{2}/\d{2}$'))
                tooltip = side_clock.locator('.desktop-clock-tooltip')
                expect(tooltip).to_be_hidden()
                side_clock.hover()
                expect(tooltip).to_be_visible()
                expect(tooltip).to_contain_text(re.compile(r'\d{1,2}:\d{2}:\d{2}.*\d{4}.*UTC'))
                app.screenshot(f'{edge}-dock-clock.png')
                dock_button('Settings').click(button='right')
                expect(bubble).to_have_attribute('data-dock-menu-side', menu_side)
                p.keyboard.press('Escape')
                expect(bubble).to_have_count(0)
                eventually(lambda: maximized_fits_dock(edge))
                if edge == 'left':
                    dock_button('Settings').drag_to(dock_button('Code'), target_position={'x': 12, 'y': 4})
                    eventually(lambda: preferences()['dock']['order'][0] == 'settings')
            settings().get_by_role('button', name='Panel mode', exact=True).click()
            align_top = settings().get_by_role('button', name='Align top', exact=True)
            center_icons = settings().get_by_role('button', name='Center icons', exact=True)

            def side_panel_fits(dock, centered):
                return dock.evaluate('''(dock, centered) => {
                    const zone = dock.parentElement;
                    const rect = el => el.getBoundingClientRect();
                    const desktop = rect(zone.closest('.desktop-canvas'));
                    const panel = rect(dock);
                    const leading = rect(dock.querySelector('.desktop-dock-leading'));
                    const apps = rect(dock.querySelector('.desktop-dock-apps'));
                    const clock = rect(dock.querySelector('.desktop-panel-clock'));
                    const style = getComputedStyle(dock);
                    const near = (a, b) => Math.abs(a - b) < 1;
                    const edge = zone.classList.contains('dock-left') ? 'left' : 'right';
                    return near(panel.top, desktop.top) && near(panel.bottom, desktop.bottom)
                        && near(panel[edge], desktop[edge])
                        && near(leading.top - panel.top, parseFloat(style.paddingTop))
                        && near(panel.bottom - clock.bottom, parseFloat(style.paddingBottom))
                        && (centered
                            ? near((apps.top + apps.bottom) / 2, (panel.top + panel.bottom) / 2)
                            : near(apps.top - leading.bottom, parseFloat(style.rowGap)));
                }''', centered)

            for edge in ('left', 'right'):
                settings().get_by_role('button', name=edge.title() + ' side', exact=True).click()
                panel = p.locator(f'.dock-{edge}.dock-panel-zone > .desktop-dock')
                expect(panel.locator('.desktop-panel-time')).to_have_text(re.compile(r'^\d{2}:\d{2}$'))
                expect(align_top).to_have_attribute('aria-pressed', 'true')
                eventually(lambda: side_panel_fits(panel, False))
                if edge == 'left':
                    app.screenshot('left-panel-align-top.png')
                center_icons.click()
                expect(center_icons).to_have_attribute('aria-pressed', 'true')
                eventually(lambda: side_panel_fits(panel, True))
                p.set_viewport_size(dict(width=900, height=590))
                eventually(lambda: side_panel_fits(panel, True))
                eventually(lambda: maximized_fits_dock(edge))
                check_dock_corners()
                align_top.click()
                eventually(lambda: side_panel_fits(panel, False))
                check_dock_corners()
                p.set_viewport_size(dict(width=1440, height=980))
            settings().get_by_role('button', name='Bottom', exact=True).click()
            eventually(lambda: maximized_fits_dock('bottom'))
            auto_hide = settings().get_by_role('button', name='Automatically hide Dock', exact=True)
            auto_hide.click()
            expect(auto_hide).to_have_attribute('aria-pressed', 'true')
            eventually(lambda: maximized_fits_dock('bottom'))
            settings().get_by_role('button', name='Reset Dock', exact=True).click()
            expect(p.locator('.dock-side-zone')).to_have_count(0)
            expect(settings().get_by_role('button', name='Bottom', exact=True)).to_have_attribute('aria-pressed', 'true')
            assert not errors, errors
            app.screenshot('restored.png')
    print(json.dumps(dict(output=str(out), checks=[
        'A default floating window uses the standard inset and can cover shortcuts; login HOME files open in Files or Text Editor',
        'Desktop shortcut visibility is saved per app, supports context-menu hiding and reset, and preserves other hidden apps when showing one from an older all-hidden layout',
        'Shortcut hover surrounds the icon and label at both desktop sizes and clips long names',
        'Built-in wallpaper, remote SVG and uploaded local PNG render through the WebView',
        'Dock reorder, size, auto-hide and unpin survive restart, and hovering leaves icon size unchanged',
        'Full-width panel, server clock, persistent icon alignment, live slider preview and saved custom size',
        'Proportional icon margins, centered activity indicators and a two-line bottom clock that ticks every second',
        'Dock position, compact side clocks with full tooltips, inward menus and maximized window clearance',
        'Floating side Docks stay centered while left and right panels fill the desktop with top and centered icon layouts',
        'The active Dock icon has a visible underline and the separate Desktop and Dock settings fit the compact layout',
    ]), indent=2))


if __name__ == '__main__':
    main()
