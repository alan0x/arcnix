"""Exercise the shared tab gesture against live PTYs, WebViews and unsaved documents."""
import shutil
import time

from desktop_browser_smoke import native_regions, site
from desktop_fixture import Desktop, Peer, ROOT, eventually, executable, expect


def round_trip(page, app, check_split):
    windows = page.locator(f'.desktop-window.app-{app}:not(.hidden)')
    active = windows.and_(page.locator('.active'))
    items = active.locator('.window-tab-item')
    expect(items).to_have_count(2)
    ids = items.locator('.window-tab-id').all_text_contents()

    def start(tab):
        tab.hover()
        page.mouse.down()

    start(items.last.locator('.window-tab-select'))
    desktop = page.locator('.desktop-canvas').bounding_box()
    page.mouse.move(desktop['x'] + 5, desktop['y'] + 5, steps=12)
    page.mouse.up()
    expect(windows).to_have_count(2)
    expect(active.locator('.window-tab-id')).to_have_text(ids[1])
    other = windows.and_(page.locator(':not(.active)'))
    expect(other.locator('.window-tab-id')).to_have_text(ids[0])
    check_split(active, other)
    titlebar = other.locator('.desktop-window-titlebar')
    target = titlebar.bounding_box()
    start(active.locator('.window-tab-select'))
    page.mouse.move(target['x'] + 9, target['y'] + 9, steps=12)
    slot = other.locator('.window-tab-slot')
    expect(slot).to_be_visible()
    assert slot.evaluate('el => el.nextElementSibling === null')
    expect(titlebar).to_have_css('box-shadow', 'none')
    page.mouse.up()
    expect(windows).to_have_count(1)
    expect(items.locator('.window-tab-id')).to_have_text(ids)
    expect(active.locator('.window-tab-select[aria-pressed=true]').locator('xpath=..').locator('.window-tab-id')).to_have_text(ids[1])
    return active


def main():
    out = ROOT / 'artifacts/window-tabs' / str(time.time_ns())
    local = out / 'local'
    local.mkdir(parents=True)
    with site() as server, Peer(out / 'peer', browser_port=server.server_port) as peer:
        binary = shutil.copy2(executable(), out / 'arcnix.exe')
        with Desktop(out / 'desktop', peer.config([peer.profile('Window tabs')], local),
                     binary=binary, extra_env={'WEBVIEW2_USER_DATA_FOLDER': ''}) as desktop:
            p = desktop.page
            p.emulate_media(reduced_motion='reduce')
            errors = []
            p.on('pageerror', lambda error: errors.append(str(error)))
            desktop.connect()
            desktop.application('Terminal')
            active = p.locator('.desktop-window.active')
            expect(active.locator('.xterm-accessibility-tree')).to_contain_text('Arcnix terminal fixture')
            active.locator('.terminal-new-tab').click()
            keyboard = active.locator('.xterm-helper-textarea')
            keyboard.press_sequentially('echo tab-move-check')
            keyboard.press('Enter')
            reader = active.locator('.xterm-accessibility-tree')
            expect(reader).to_contain_text('tab-move-check')
            def check_terminal(moved, original):
                expect(moved.locator('.xterm-accessibility-tree')).to_contain_text('tab-move-check')
                expect(original.locator('.xterm-accessibility-tree')).to_contain_text('Arcnix terminal fixture')
            round_trip(p, 'terminal', check_terminal)
            expect(reader).to_contain_text('tab-move-check')
            # Open beyond the old eight-session cap. Jump directly to both ends of the strip.
            for count in range(3, 13):
                active.locator('.terminal-new-tab').click()
                expect(active.locator('.terminal-tab')).to_have_count(count)
            strip = active.locator('.terminal-tab-list')
            picker = active.get_by_role('button', name='All tabs', exact=True)
            expect(picker).to_have_attribute('data-tab-count', '12')
            eventually(lambda: strip.evaluate('el => el.scrollLeft > 0'))
            for key, index in [('Home', 0), ('End', 11)]:
                picker.click()
                menu = active.get_by_role('group', name='All tabs')
                expect(menu.get_by_role('button')).to_have_count(12)
                p.keyboard.press(key)
                p.keyboard.press('Enter')
                tab = active.locator('.terminal-tab').nth(index)
                expect(tab).to_have_attribute('aria-pressed', 'true')
                eventually(lambda: tab.evaluate('''el => {
                    const bounds = el.closest('.window-tab-strip').getBoundingClientRect();
                    const tab = el.closest('.window-tab-item').getBoundingClientRect();
                    return tab.left >= bounds.left - 1 && tab.right <= bounds.right + 1;
                }'''))
            expect(reader).to_contain_text('Arcnix terminal fixture')
            active.locator('.window-close').click()

            desktop.application('Browser')
            for path, title in [('first', 'Routed first page'), ('second', 'Routed second page')]:
                if path == 'second':
                    active.get_by_role('button', name='New web tab', exact=True).last.click()
                url = f'http://browser-fixture.test:{server.server_port}/{path}'
                address = active.locator('.browser-address')
                address.fill(url)
                address.press('Enter')
                expect(active.locator('.browser-tab.active')).to_contain_text(title, timeout=20000)
            def check_browser(moved, original):
                expect(moved.locator('.browser-tab.active')).to_contain_text('Routed second page')
                expect(original.locator('.browser-tab.active')).to_contain_text('Routed first page')
                eventually(lambda: len(native_regions(desktop.process.pid)) == 2)
            round_trip(p, 'browser', check_browser)
            expect(active.locator('.browser-address')).to_have_value(url)
            eventually(lambda: len(native_regions(desktop.process.pid)) == 1)
            active.locator('.window-close').click()

            desktop.application('Text Editor')
            for draft in ['first unsaved draft', 'second unsaved draft']:
                active.get_by_role('button', name='File', exact=True).click()
                active.locator('.editor-menu-popup:popover-open').get_by_role('button', name='New document', exact=True).click()
                active.get_by_role('textbox', name='File contents', exact=True).fill(draft)
            def check_editor(moved, original):
                expect(moved.get_by_role('textbox', name='File contents')).to_have_value('second unsaved draft')
                expect(original.get_by_role('textbox', name='File contents')).to_have_value('first unsaved draft')
            round_trip(p, 'editor', check_editor)
            active.get_by_role('button', name='File', exact=True).click()
            active.locator('.editor-menu-popup:popover-open').get_by_role('button', name='New document', exact=True).click()
            items = active.locator('.window-tab-item')
            expect(items).to_have_count(3)
            ids = items.locator('.window-tab-id').all_text_contents()
            selected = items.last.locator('.window-tab-select')
            selected.click()
            expect(selected).to_have_css('outline-style', 'none')
            p.keyboard.press('Tab')
            expect(items.last.locator('.editor-tab-close')).to_be_focused()
            p.keyboard.press('Shift+Tab')
            expect(selected).to_be_focused()
            expect(selected).to_have_css('outline-style', 'none')
            expect(items.last).to_have_css('outline-style', 'solid')

            p.emulate_media(reduced_motion='no-preference')
            # Move the last document between its neighbours; cancel once, then commit.
            for cancel in [True, False]:
                middle = items.nth(1).bounding_box()
                selected.hover(position=dict(x=5, y=5))
                p.mouse.down()
                p.mouse.move(middle['x'] + 3, middle['y'] + middle['height'] / 2, steps=12)
                slot = active.locator('.window-tab-slot')
                expect(slot).to_be_visible()
                assert slot.evaluate("e => e.nextElementSibling.querySelector('.window-tab-id').textContent") == ids[1]
                gap = slot.bounding_box()
                p.mouse.move(gap['x'] + gap['width'] / 2, gap['y'] + gap['height'] / 2)
                if cancel:
                    p.keyboard.press('Escape')
                p.mouse.up()
                expect(slot).to_have_count(0)
                expect(items.locator('.window-tab-id')).to_have_text(ids if cancel else [ids[0], ids[2], ids[1]])
            items.nth(1).locator('.editor-tab-close').click()
            expect(items).to_have_count(2)
            # Closing a detached dirty document must only discard that window's draft.
            tab = active.locator('.window-tab-select').last
            tab.hover()
            p.mouse.down()
            canvas = p.locator('.desktop-canvas').bounding_box()
            p.mouse.move(canvas['x'] + 5, canvas['y'] + 5, steps=12)
            p.mouse.up()
            editors = p.locator('.desktop-window.app-editor:not(.hidden)')
            expect(editors).to_have_count(2)
            active.locator('.window-close').click()
            active.get_by_role('button', name='Keep editing', exact=True).click()
            expect(active.get_by_role('textbox', name='File contents')).to_have_value('second unsaved draft')
            active.locator('.window-close').click()
            active.get_by_role('button', name='Discard changes', exact=True).click()
            expect(editors).to_have_count(1)
            expect(active.get_by_role('textbox', name='File contents')).to_have_value('first unsaved draft')
            assert not errors, errors
    print('Terminal sessions, browser pages and unsaved editor documents survived detach and merge.')


if __name__ == '__main__':
    main()
