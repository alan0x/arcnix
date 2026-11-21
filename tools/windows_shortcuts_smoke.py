"""Exercise shell shortcuts through Windows input with the fixture in the foreground."""
import ctypes
import re
import time
from ctypes import wintypes

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect

CTRL, ALT, F4 = 0xA2, 0xA4, 0x73


def window_process(hwnd):
    process = wintypes.DWORD()
    ctypes.windll.user32.GetWindowThreadProcessId(wintypes.HWND(hwnd), ctypes.byref(process))
    return process.value


def activate_fixture(app):
    # CDP can operate a background WebView. Physical input needs its real HWND.
    user32 = ctypes.windll.user32
    user32.GetForegroundWindow.restype = wintypes.HWND
    if window_process(user32.GetForegroundWindow()) == app.process.pid:
        return
    owned = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def find_window(hwnd, _):
        if window_process(hwnd) == app.process.pid and user32.IsWindowVisible(hwnd):
            title = ctypes.create_unicode_buffer(256)
            user32.GetWindowTextW(hwnd, title, len(title))
            if title.value.startswith('Arcnix'):
                owned.append(hwnd)
        return True

    user32.EnumWindows(find_window, 0)
    assert len(owned) == 1, 'Could not identify the fixture window'
    hwnd = wintypes.HWND(owned[0])
    user32.ShowWindow(hwnd, 9)
    current = ctypes.windll.kernel32.GetCurrentThreadId()
    foreground = user32.GetWindowThreadProcessId(user32.GetForegroundWindow(), None)
    attached = user32.AttachThreadInput(current, foreground, True)
    try:
        user32.SetForegroundWindow(hwnd)
    finally:
        if attached:
            user32.AttachThreadInput(current, foreground, False)
    if window_process(user32.GetForegroundWindow()) == app.process.pid:
        return
    old = wintypes.POINT()
    user32.GetCursorPos(ctypes.byref(old))
    # HWND_TOPMOST and HWND_NOTOPMOST are negative pointer-sized handles.
    user32.SetWindowPos.argtypes = [wintypes.HWND, wintypes.HWND, ctypes.c_int, ctypes.c_int,
                                    ctypes.c_int, ctypes.c_int, wintypes.UINT]
    try:
        if not user32.SetWindowPos(hwnd, -1, 100, 100, 0, 0, 1):
            raise ctypes.WinError()
        user32.WindowFromPoint.argtypes = [wintypes.POINT]
        user32.WindowFromPoint.restype = wintypes.HWND
        user32.GetAncestor.restype = wintypes.HWND

        def visible_titlebar_point():
            rect = wintypes.RECT()
            user32.GetWindowRect(hwnd, ctypes.byref(rect))
            for x in (rect.left + 52, (rect.left + rect.right) // 2, rect.right - 180):
                point = wintypes.POINT(x, rect.top + 14)
                if user32.GetAncestor(wintypes.HWND(user32.WindowFromPoint(point)), 2) == hwnd.value:
                    return point

        point = eventually(visible_titlebar_point)
        click_owned_point(hwnd, point)
        eventually(lambda: window_process(user32.GetForegroundWindow()) == app.process.pid)
    finally:
        user32.SetWindowPos(hwnd, -2, 0, 0, 0, 0, 3)
        user32.SetCursorPos(old.x, old.y)


def click_owned_point(hwnd, point):
    user32 = ctypes.windll.user32
    user32.SetCursorPos(point.x, point.y)
    actual = wintypes.POINT()
    assert user32.GetCursorPos(ctypes.byref(actual))
    assert (actual.x, actual.y) == (point.x, point.y), 'The mouse is confined outside the fixture'
    assert user32.GetAncestor(wintypes.HWND(user32.WindowFromPoint(actual)), 2) == hwnd.value
    user32.mouse_event(2, 0, 0, 0, 0)
    user32.mouse_event(4, 0, 0, 0, 0)


def click_in_main_webview(app, field):
    # Give the outer WebView native keyboard focus after using a child WebView.
    user32 = ctypes.windll.user32
    hwnd = wintypes.HWND(user32.GetForegroundWindow())
    assert window_process(hwnd.value) == app.process.pid
    x, y = field.evaluate("""field => {
        const rect = field.getBoundingClientRect();
        return [(rect.x + rect.width / 2) * devicePixelRatio, (rect.y + rect.height / 2) * devicePixelRatio];
    }""")
    point = wintypes.POINT(round(x), round(y))
    assert user32.ClientToScreen(hwnd, ctypes.byref(point))
    user32.WindowFromPoint.argtypes = [wintypes.POINT]
    user32.WindowFromPoint.restype = user32.GetAncestor.restype = wintypes.HWND
    old = wintypes.POINT()
    user32.GetCursorPos(ctypes.byref(old))
    try:
        click_owned_point(hwnd, point)
        expect(field).to_be_focused()
    finally:
        user32.SetCursorPos(old.x, old.y)


def press(app, *keys):
    user32 = ctypes.windll.user32
    assert window_process(user32.GetForegroundWindow()) == app.process.pid, 'The test window must own keyboard input'
    down = []
    try:
        for key in keys:
            # Extended scancodes for the Windows and arrow keys.
            extended = int(key in (0x5B, 0x5C, 0x23, 0x26, 0x28))
            scan = user32.MapVirtualKeyW(key, 0)
            user32.keybd_event(key, scan, extended, 0)
            down.append((key, scan, extended))
            time.sleep(.025)
    finally:
        for key, scan, extended in reversed(down):
            user32.keybd_event(key, scan, extended | 2, 0)
            time.sleep(.025)
    app.page.wait_for_timeout(300)
    foreground = user32.GetForegroundWindow()
    if window_process(foreground) != app.process.pid:
        name = ctypes.create_unicode_buffer(256)
        user32.GetClassNameW(wintypes.HWND(foreground), name, len(name))
        raise AssertionError(f'The foreground moved to {name.value!r} after {keys}')


def check_files(app, peer, marker):
    app.application('Files')
    files = app.page.locator('.desktop-window.app-files.active')
    tabs = files.locator('.files-tab')
    expect(tabs).to_have_count(1)
    press(app, CTRL, ord('T'))
    expect(tabs).to_have_count(2)
    app.file('shortcut.txt').click()
    press(app, CTRL, ord('C'))
    app.file('destination').dblclick()
    files.locator('.files-pane .file-list-container').focus()
    press(app, CTRL, ord('V'))
    expect(app.file('shortcut.txt')).to_be_visible()
    assert (peer.root / 'destination/shortcut.txt').read_text() == marker
    press(app, CTRL, F4)
    expect(tabs).to_have_count(1)
    expect(app.file('destination')).to_be_visible()


def check_editor(app, marker):
    app.application('Text Editor')
    editor = app.page.locator('.desktop-window.app-editor.active')
    editor.get_by_role('button', name='Open file…', exact=True).first.click()
    picker = app.page.get_by_role('dialog', name='Open file', exact=True)
    expect(picker).to_be_visible()
    press(app, ALT, F4)
    expect(picker).to_be_visible()
    picker.get_by_role('button', name='Filesystem', exact=True).click()
    picker.get_by_role('button', name='shortcut.txt', exact=True).dblclick()
    field = editor.get_by_role('textbox', name='File contents', exact=True)
    expect(field).to_have_value(marker)
    field.focus()
    press(app, CTRL, ord('A'))
    press(app, CTRL, ord('C'))
    app.page.keyboard.insert_text('draft')
    press(app, CTRL, ord('A'))
    press(app, CTRL, ord('V'))
    expect(field).to_have_value(marker)
    press(app, CTRL, ord('A'))
    press(app, CTRL, ord('X'))
    expect(field).to_have_value('')
    press(app, CTRL, ord('Z'))
    expect(field).to_have_value(marker)
    press(app, CTRL, ord('Y'))
    expect(field).to_have_value('')
    press(app, CTRL, F4)
    expect(editor.get_by_text('Discard unsaved changes?', exact=True)).to_be_visible()
    press(app, ALT, F4)
    expect(editor.get_by_text('Discard unsaved changes?', exact=True)).to_be_visible()
    editor.get_by_role('button', name='Keep editing', exact=True).click()
    press(app, ALT, F4)
    expect(editor.get_by_text('Discard unsaved changes in all open files?', exact=True)).to_be_visible()
    editor.get_by_role('button', name='Discard changes', exact=True).click()
    expect(editor).to_have_count(0)


def check_terminal(app, marker):
    app.application('Terminal')
    surface = app.page.locator('.terminal-surface')
    keyboard = surface.locator('.xterm-helper-textarea')
    reader = surface.locator('.xterm-accessibility-tree')
    expect(app.page.locator('.terminal-status')).to_have_text('Interactive SSH')
    keyboard.focus()
    app.page.keyboard.type('echo ')
    press(app, CTRL, ord('V'))
    expect(reader).to_contain_text('echo ' + marker)
    press(app, CTRL, ord('C'))
    expect(reader).to_contain_text('^C')
    tabs = app.page.locator('.terminal-tab')
    before = tabs.count()
    press(app, CTRL, 0xA0, ord('T'))
    expect(tabs).to_have_count(before + 1)
    press(app, CTRL, F4)
    expect(tabs).to_have_count(before)


def check_code(app, peer):
    app.application('Files')
    app.file('sample.rs').dblclick()
    code = app.code()
    field = code.locator('.monaco-editor textarea.inputarea').first
    # Use the workbench entry point that moves native focus to its child WebView.
    # A CDP click alone can focus the textarea while the parent HWND still owns input.
    app.page.locator('.desktop-window.app-code.active .code-quick-open').click()
    expect(code.locator('.quick-input-widget')).to_be_visible()
    code.keyboard.press('Escape')
    code.locator('.monaco-editor .view-lines').first.click()
    expect(field).to_be_focused()
    press(app, CTRL, ord('A'))
    press(app, CTRL, ord('C'))
    code.keyboard.insert_text('temporary')
    press(app, CTRL, ord('A'))
    press(app, CTRL, ord('V'))
    press(app, CTRL, 0x23)  # End
    code.keyboard.insert_text(' saved')
    press(app, CTRL, ord('S'))
    eventually(lambda: (peer.root / 'sample.rs').read_text() == 'code sample saved')
    press(app, CTRL, F4)
    expect(field).to_have_count(0)
    expect(app.page.locator('.desktop-window.app-code.active')).to_be_visible()
    press(app, ALT, F4)
    expect(app.page.locator('.desktop-window.app-code')).to_have_count(0)


def main():
    out = ROOT / 'artifacts/windows-shortcuts' / str(time.time_ns())
    with Peer(out / 'peer') as peer, Desktop(
        out / 'desktop', peer.config([peer.profile('Windows shortcuts')], out,
                                     file_associations={'rs': {'default': 'code'}}),
    ) as app:
        marker = 'Arcnix shortcut sample'
        (peer.root / 'shortcut.txt').write_text(marker)
        (peer.root / 'sample.rs').write_text('code sample')
        (peer.root / 'destination').mkdir()
        app.connect()
        app.application('Terminal')
        app.application('Monitor')
        title = app.page.locator('.desktop-window.active .desktop-window-title')
        expect(title).to_have_text('Monitor')
        activate_fixture(app)
        app.page.wait_for_timeout(650)
        press(app, 0xA4, 0x09)
        expect(title).to_have_text('Terminal')
        press(app, 0xA4, 0xA0, 0x09)
        expect(title).to_have_text('Monitor')
        press(app, 0x5B, 0x45)
        expect(title).to_have_text('Files')
        files = app.page.locator('.desktop-window.app-files.active')
        for _ in range(2):
            press(app, 0x5B, 0x26)
            expect(files).to_have_class(re.compile(r'\bmaximized\b'))
        press(app, 0x5B, 0x28)
        expect(files).to_have_count(0)
        expect(title).to_have_text('Monitor')
        press(app, 0x5B, 0x44)
        expect(app.page.locator('.desktop-window:visible')).to_have_count(0)
        press(app, 0x5C, 0x44)
        expect(title).to_have_text('Monitor')
        expect(app.page.locator('.desktop-window.app-terminal')).to_be_visible()
        expect(app.page.locator('.desktop-window.app-files')).to_be_hidden()
        press(app, CTRL, F4)
        expect(title).to_have_text('Monitor')
        press(app, ALT, F4)
        expect(title).to_have_text('Terminal')
        check_files(app, peer, marker)
        check_editor(app, marker)
        check_terminal(app, marker)
        check_code(app, peer)
        app.application('Browser')
        browser = app.page.locator('.desktop-window.app-browser.active')
        browser.get_by_role('button', name='New web tab', exact=True).last.click()
        expect(browser.locator('.browser-tab')).to_have_count(2)
        address = browser.locator('.browser-address')
        click_in_main_webview(app, address)
        address.fill(marker)
        press(app, CTRL, ord('A'))
        press(app, CTRL, ord('C'))
        app.page.keyboard.insert_text('draft')
        press(app, CTRL, ord('A'))
        press(app, CTRL, ord('V'))
        expect(address).to_have_value(marker)
        press(app, CTRL, F4)
        expect(browser.locator('.browser-tab')).to_have_count(1)
        press(app, 0x5B, ord('D'))
        press(app, ALT, F4)
        expect(app.page.locator('.desktop-window:visible')).to_have_count(0)
    print('Foreground Windows shortcuts passed.')


if __name__ == '__main__':
    main()
