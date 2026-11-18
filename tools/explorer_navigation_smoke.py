"""Verify Explorer locations, committed navigation history and inline search."""
from datetime import datetime, timedelta
import json
import os
import time

from desktop_fixture import Desktop, Peer, ROOT, expect


def main():
    out = ROOT / 'artifacts/explorer-navigation' / f'run-{time.time_ns()}'
    out.mkdir(parents=True)
    with Peer(out / 'peer') as peer:
        home = peer.root / 'home/arcnix'
        desktop = home / 'Desktop'
        desktop.mkdir(parents=True, exist_ok=True)
        (home / 'Documents').mkdir(exist_ok=True)
        (desktop / 'notes-today.txt').write_text('sample notes')
        (desktop / 'notes-old.PDF').write_text('sample document')
        (desktop / 'floorplan.png').write_text('sample picture')
        noon = datetime.now().replace(hour=12, minute=0, second=0, microsecond=0)
        for filename, day in [('notes-today.txt', noon), ('notes-old.PDF', noon - timedelta(days=1))]:
            timestamp = day.timestamp()
            os.utime(desktop / filename, (timestamp, timestamp))
        with Desktop(out / 'desktop', peer.config([peer.profile('File navigation')], out)) as app:
            page = app.page
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            app.connect()
            app.application('File explorer')
            explorer = page.locator('.desktop-window.app-explorer.active')
            sidebar = explorer.locator('.explorer-sidebar')
            pathbar = explorer.locator('.pathbar')
            crumb = pathbar.locator('.breadcrumb.current')
            back = pathbar.get_by_role('button', name='Back', exact=True)
            forward = pathbar.get_by_role('button', name='Forward', exact=True)

            def go(label, path):
                sidebar.get_by_role('button', name=label, exact=True).click()
                expect(crumb).to_have_attribute('aria-label', path)

            go('Filesystem', '/')
            expect(back).to_be_disabled()
            go('Home', '/home/arcnix')
            go('Desktop', '/home/arcnix/Desktop')
            separators = pathbar.locator('.breadcrumbs .glyph-chevron-right')
            expect(separators).to_have_count(3)
            separator = separators.nth(1)
            size = separator.evaluate('el => [el.offsetWidth, el.offsetHeight]')
            assert size == [18, 18], size
            assert 'data:image/svg+xml' in separator.evaluate('el => getComputedStyle(el).maskImage')
            home_crumb = pathbar.get_by_role('button', name='/home', exact=True)
            expect(home_crumb).to_have_css('font-size', '14px')
            background = home_crumb.evaluate('el => getComputedStyle(el).backgroundColor')
            separator_color = separator.evaluate('el => getComputedStyle(el).color')
            home_crumb.hover()
            expect(home_crumb).not_to_have_css('background-color', background)
            expect(separator).to_have_css('color', separator_color)
            box = separator.bounding_box()
            page.mouse.move(box['x'] + box['width'] / 2, box['y'] + box['height'] / 2)
            expect(home_crumb).to_have_css('background-color', background)
            back.click()
            expect(crumb).to_have_attribute('aria-label', '/home/arcnix')
            back.click()
            expect(crumb).to_have_attribute('aria-label', '/')
            expect(back).to_be_disabled()
            forward.click()
            expect(crumb).to_have_attribute('aria-label', '/home/arcnix')
            forward.click()
            expect(crumb).to_have_attribute('aria-label', '/home/arcnix/Desktop')
            expect(forward).to_be_disabled()
            pathbar.get_by_role('button', name='Refresh remote directory', exact=True).click()
            expect(back).to_be_enabled()
            crumb.press('Control+l')
            edit_path = pathbar.locator('.path-input:visible')
            edit_path.fill('/not-a-real-directory')
            edit_path.press('Enter')
            expect(crumb).to_have_attribute('aria-label', '/home/arcnix/Desktop')
            page.get_by_role('button', name='Dismiss notification', exact=True).click()
            back.click()
            expect(crumb).to_have_attribute('aria-label', '/home/arcnix')
            go('Documents', '/home/arcnix/Documents')
            expect(forward).to_be_disabled()
            go('Desktop', '/home/arcnix/Desktop')

            expect(pathbar.get_by_role('button', name='Search files', exact=True)).to_be_enabled()
            crumb.press('Control+f')
            query = pathbar.get_by_role('textbox', name='Search current folder', exact=True)
            expect(query).to_be_focused()
            expect(pathbar.locator('.breadcrumbs')).to_be_hidden()
            query.fill('notes')
            expect(app.file('notes-today.txt')).to_be_visible()
            expect(app.file('notes-old.PDF')).to_be_visible()
            expect(app.file('floorplan.png')).to_have_count(0)
            pathbar.get_by_role('button', name='Search filters', exact=True).click()
            options = explorer.locator('.explorer-search-options')
            options.get_by_role('button', name='PDF', exact=True).click()
            expect(app.file('notes-today.txt')).to_have_count(0)
            expect(app.file('notes-old.PDF')).to_be_visible()
            options.get_by_role('button', name='Today', exact=True).click()
            expect(explorer.locator('.file-empty')).to_contain_text('No matching items')
            options.get_by_role('button', name='Yesterday', exact=True).click()
            expect(app.file('notes-old.PDF')).to_be_visible()
            options.get_by_role('button', name='Reset filters', exact=True).click()
            expect(app.file('notes-today.txt')).to_be_visible()
            expect(query).to_have_value('notes')

            page.set_viewport_size(dict(width=900, height=650))
            popup, window = options.bounding_box(), explorer.bounding_box()
            assert popup['x'] >= window['x'] and popup['x'] + popup['width'] <= window['x'] + window['width']
            app.screenshot('explorer-search-filters.png')
            width = query.bounding_box()['width']
            assert width >= 100, width
            page.keyboard.press('Escape')
            expect(options).to_have_count(0)
            expect(query).to_be_visible()
            page.keyboard.press('Escape')
            expect(query).to_have_count(0)
            expect(crumb).to_have_attribute('aria-label', '/home/arcnix/Desktop')
            expect(app.file('floorplan.png')).to_be_visible()

            page.set_viewport_size(dict(width=1440, height=980))
            go('Home', '/home/arcnix')
            app.file('Documents').click(button='right')
            menu = explorer.locator('.file-context-menu')
            expect(menu.get_by_role('button', name='Folder color…', exact=True)).to_be_visible()
            expect(menu.get_by_role('button', name='Terminal here', exact=True)).to_be_enabled()
            menu.evaluate("""element => {
                window.explorerMenuChanges = [];
                const observer = new MutationObserver(records => {
                    for (const record of records) {
                        for (const node of [...record.addedNodes, ...record.removedNodes]) {
                            if (node.textContent.trim()) window.explorerMenuChanges.push(node.textContent);
                        }
                    }
                });
                observer.observe(element, {childList:true, subtree:true});
            }""")
            # Confirm a real periodic settings read occurred while the menu was open.
            settings = home / '.arcnix/desktop.json'
            settings.parent.mkdir(exist_ok=True)
            preferences = json.loads(settings.read_text(encoding='utf-8')) if settings.exists() else {
                'version': 1, 'theme': 'System',
            }
            preferences.setdefault('explorer', {})['favorites'] = ['/home/arcnix/Documents']
            settings.write_text(json.dumps(preferences), encoding='utf-8')
            favorite = sidebar.locator('.explorer-favorite-row button[aria-label="Open favorite /home/arcnix/Documents"]')
            expect(favorite).to_be_visible(timeout=22000)
            expect(menu.get_by_role('button', name='Folder color…', exact=True)).to_be_enabled()
            assert page.evaluate('window.explorerMenuChanges') == []
            page.keyboard.press('Escape')
            expect(menu).to_have_count(0)
            assert not errors, errors
    print('Explorer navigation, search filters and menu stability during settings sync passed.')


if __name__ == '__main__':
    main()
