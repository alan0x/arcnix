"""Exercise the visible Code menus and the embedded SSH terminal over a real connection."""
import json
import re
import time

from desktop_browser_smoke import native_regions
from desktop_fixture import Desktop, Peer, ROOT, eventually, expect
from desktop_windows_smoke import native_bounds, native_window


def menu_item(page, title, command):
    bar = page.get_by_role('menubar')
    button = bar.get_by_role('menuitem', name=title, exact=True)
    if button.is_visible():
        button.click()
    else:
        bar.get_by_role('menuitem', name='More', exact=True).click()
        submenu = page.get_by_role('menu').get_by_role('menuitem', name=title, exact=True)
        submenu.hover()
        submenu.press('ArrowRight')
    item = page.get_by_role('menuitem', name=re.compile('^' + re.escape(command))).last
    expect(item).to_be_visible()
    return item


def invoke(page, title, command):
    item = menu_item(page, title, command)
    expect(item).to_be_enabled()
    # The workbench suppresses accidental mouseup on newly opened menus.
    page.wait_for_timeout(160)
    item.click()


def check_titlebar_drag(app):
    page = app.page
    window = page.locator('.desktop-window.app-code')
    bridge = window.locator('.browser-viewport-input')

    def menu_hit_region():
        viewport = json.loads(bridge.input_value())
        body, menu, command = (viewport[key] for key in ('area', 'menu_slot', 'command_center'))
        y = (body['top'] - command['top']) // 2
        # The first menu is in the child HWND. The blank end of its slot is
        # in the host so it starts a child-window drag.
        points = [(menu['left'] - body['left'] + 8, y), (menu['right'] - body['left'] - 6, y)]
        return native_regions(app.process.pid, points) == [[True, False]]

    eventually(menu_hit_region)
    root = native_window(app.process.pid)
    before, root_before = window.bounding_box(), native_bounds(root)
    slot = window.locator('.code-menu-slot').bounding_box()
    x, y = slot['x'] + slot['width'] - 6, slot['y'] + slot['height'] / 2
    dx, dy = 46, 28
    page.mouse.move(x, y)
    page.mouse.down()
    page.mouse.move(x + dx, y + dy, steps=8)
    page.mouse.up()
    after = window.bounding_box()
    assert abs(after['x'] - before['x'] - dx) <= 2, (before, after)
    assert abs(after['y'] - before['y'] - dy) <= 2, (before, after)
    assert native_bounds(root) == root_before


def main():
    out = ROOT / 'artifacts/code-menus' / str(time.time_ns())
    with Peer(out / 'peer') as peer:
        root = peer.root / 'project'
        root.mkdir()
        notes = root / 'notes.md'
        notes.write_text('# Notes\n', encoding='utf8')
        profile = peer.profile('Code')
        profile['initial_remote_path'] = '/project'
        with Desktop(out / 'app', peer.config([profile], out)) as app:
            app.page.set_viewport_size(dict(width=1760, height=980))
            app.page.emulate_media(reduced_motion='reduce')
            app.connect()
            app.application('Code')
            page = app.code()
            tree = page.get_by_role('treeitem', name='notes.md', exact=True)
            expect(tree).to_be_visible()
            check_titlebar_drag(app)
            expect(menu_item(page, 'File', 'Save')).to_be_disabled()
            page.keyboard.press('Escape')
            invoke(page, 'File', 'New File')
            name = page.locator('.explorer-folders-view input:visible')
            name.fill('from-menu.txt')
            name.press('Enter')
            eventually(lambda: (root / 'from-menu.txt').exists())
            tree.dblclick()
            area = app.replace_code('# 菜单验证\n')
            area.press('Control+Home')
            invoke(page, 'Selection', 'Copy Line Down')
            invoke(page, 'Edit', 'Undo')
            invoke(page, 'File', 'Save')
            eventually(lambda: notes.read_text(encoding='utf8') == '# 菜单验证\n')
            invoke(page, 'Go', 'Go to Line')
            picker = page.locator('.quick-input-widget input:visible').first
            expect(picker).to_be_focused()
            picker.fill(':2')
            picker.press('Enter')
            invoke(page, 'View', 'Open Markdown Preview')
            expect(page.locator('.arcnix-preview h1')).to_have_text('菜单验证')
            invoke(page, 'Terminal', 'New Terminal')
            rows = page.locator('.xterm-rows:visible').last
            expect(rows).to_contain_text('Arcnix terminal fixture')
            def command(text):
                area = page.locator('.xterm-helper-textarea:visible').last
                area.focus()
                page.keyboard.type(text)
                area.press('Enter')
            command('pwd')
            expect(rows).to_contain_text('/project')
            command('unicode')
            expect(rows).to_contain_text('中文终端')
            invoke(page, 'Terminal', 'Toggle Terminal')
            expect(rows).to_have_count(0)
            invoke(page, 'Terminal', 'Toggle Terminal')
            expect(rows).to_contain_text('中文终端')
            invoke(page, 'Terminal', 'New Terminal')
            expect(rows).to_contain_text('Arcnix terminal fixture')
            expect(rows).not_to_contain_text('中文终端')
            invoke(page, 'Terminal', 'Kill Active Terminal')
            expect(rows).to_contain_text('中文终端')
            command('burst')
            expect(rows).to_contain_text('BURST COMPLETE', timeout=45000)
            command('exit')
            expect(rows).to_have_count(0)
            print('Titlebar drag, menus, SFTP save, Markdown preview, SSH session switching, UTF-8 and output flow passed.')


if __name__ == '__main__':
    main()
