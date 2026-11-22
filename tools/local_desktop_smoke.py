"""Exercise the homepage local desktop against a real WSL distribution, without SSH."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from desktop_fixture import Desktop, ROOT, eventually, expect, set_desktop_mode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--distribution', required=True)
    distro = parser.parse_args().distribution

    def linux(*args):
        return subprocess.check_output(['wsl.exe', '-d', distro, '--cd', '/', '--exec', *args])

    def enter(app):
        page = app.page
        entry = page.locator('.server-explorer').get_by_role('button', name='Try the local desktop', exact=True)
        expect(page.locator('.local-desktop-entry + .server-search')).to_be_visible()
        entry.click()
        choice = page.locator('.wsl-distribution').filter(has=page.get_by_text(distro, exact=True))
        expect(choice.or_(page.locator('.desktop-dock:visible'))).to_be_visible(timeout=30000)
        if choice.is_visible():
            choice.click()
        expect(page.locator('.connection-state')).to_have_text('Connected locally', timeout=30000)
        expect(page.locator('.desktop-dock')).to_be_visible()
        expect(page.locator('.desktop-wallpaper-copy')).to_contain_text('Connect to a server for more.')
        expect(entry).to_be_visible()
        for app_class in ('app-code', 'app-player'):
            expect(page.locator(f'.desktop-shortcuts .{app_class}')).to_be_visible()
            expect(page.locator(f'.desktop-dock .{app_class}')).to_be_visible()
        catalog = app.apps()
        for label in ('Code Editor', 'Player'):
            expect(catalog.get_by_role('button', name=label, exact=True)).to_be_visible()
        catalog.get_by_role('button', name='Close dialog', exact=True).click()
        expect(page.locator('.workbench-tabs').get_by_role('button', name='Transfer files')).to_have_count(0)

    home = linux('sh', '-c', 'printf %s "$HOME"').decode()
    directory = linux('mktemp', '-d', home.rstrip('/') + '/arcnix-desktop-XXXXXXXX').decode().strip()
    assert Path(directory).parent.as_posix() == home
    name = "local 桌面's note.md"
    path = directory + '/' + name
    out = Path(tempfile.mkdtemp(prefix='local-desktop-', dir=ROOT / 'artifacts'))
    cfg = dict(version=1, config=dict(servers=[], groups=[], trusted_hosts=[], settings=dict(
        language='English', theme='Light', show_hidden_files=False, resource_interval_seconds=3)))
    try:
        with Desktop(out, cfg) as app:
            enter(app)
            # Keep the shell in English and change just this desktop while Files is mounted.
            folder = directory.rsplit('/', 1)[1]
            owner = linux('stat', '-c', '%U (%u) / %G (%g)', directory).decode().strip()
            app.application('Files')
            for language, region, open_folder, properties, owner_label, close in (
                ('简体中文', 'Region & language', '打开位置', '属性', '所有者 / 组', '关闭对话框'),
                ('English', '区域与语言', 'Open in', 'Properties', 'Owner / Group', 'Close dialog'),
            ):
                page = app.page
                page.locator('.desktop-dock-app.app-settings').click()
                panel = page.locator('.app-settings.active')
                panel.get_by_role('button', name=region, exact=True).click()
                panel.locator('.setting-desktop-language').get_by_role('radio', name=language, exact=True).check()
                page.locator('.desktop-dock-app.app-files').click()
                app.file(folder).click(button='right')
                menu = page.locator('.file-context-menu:visible')
                expect(menu.get_by_role('button', name=open_folder, exact=True)).to_be_visible()
                menu.get_by_role('button', name=properties, exact=True).click()
                dialog = page.get_by_role('dialog', name=properties, exact=True)
                expect(dialog.locator('.property-detail').filter(has_text=owner_label)).to_contain_text(owner)
                expect(dialog.locator('.permission-matrix')).to_contain_text(owner.split(' (', 1)[0])
                dialog.get_by_role('button', name=close, exact=True).click()
            app.application('Files')
            app.file(directory.rsplit('/', 1)[1]).dblclick()
            expect(app.page.get_by_text('This folder is empty', exact=True)).to_be_visible()
            app.page.get_by_text('This folder is empty', exact=True).click(button='right')
            app.action('New file')
            rename = app.page.locator('.app-files.active .file-inline-name')
            expect(rename).to_be_focused()
            rename.fill(name)
            rename.press('Enter')
            app.file(name).dblclick()
            titlebar = app.page.locator('.app-editor.active .desktop-window-titlebar')
            titlebar.get_by_role('button', name='View', exact=True).click()
            titlebar.get_by_role('button', name='Edit source', exact=True).click()
            content = app.page.locator('.app-editor.active .remote-editor-content')
            content.fill('# Local desktop\n')
            content.press('Control+s')
            eventually(lambda: linux('cat', path) == b'# Local desktop\n')
            app.page.locator('.app-editor.active .window-close').click()
            app.application('Files')
            app.file(name).click(button='right')
            app.action('Terminal here')
            screen = app.page.locator('.app-terminal.active .xterm-accessibility-tree')
            keyboard = app.page.locator('.app-terminal.active .xterm-helper-textarea')
            keyboard.focus()
            app.page.keyboard.type("test -t 0 && grep -l 'Local desktop' ./*.md > terminal-result")
            app.page.keyboard.press('Enter')
            eventually(lambda: linux('sh', '-c', 'test ! -f "$1" || cat "$1"', 'arcnix', directory + '/terminal-result').decode().strip() == './' + name)
            expect(screen).to_be_visible()
            app.application('Settings')
            set_desktop_mode(app.page.locator('.app-settings.active'), 'Dark')
            prefs = eventually(lambda: next(out.glob('local-desktops/*/desktop.json'), None))
            eventually(lambda: json.loads(prefs.read_bytes()).get('appearance', {}).get('overrides', {}).get('mode') == 'Dark')
        def put(filename, data):
            subprocess.run(['wsl.exe', '-d', distro, '--cd', '/', '--exec', 'sh', '-c',
                            'cat > "$1"', 'arcnix', directory + '/' + filename], input=data, check=True)

        put('example.py', b'print(1)\n')
        clip = out / 'sample.mp4'
        subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-f', 'lavfi', '-i',
                        'testsrc2=size=320x180:rate=12', '-t', '8', '-c:v', 'libx264',
                        '-pix_fmt', 'yuv420p', str(clip)], check=True)
        put(clip.name, clip.read_bytes())
        with Desktop(out, None) as app:
            enter(app)
            app.application('Settings')
            panel = app.page.locator('.app-settings.active')
            panel.get_by_role('button', name='Appearance', exact=True).click()
            expect(panel.get_by_role('radio', name='Dark', exact=True)).to_be_checked()
            app.application('Files')
            app.file(directory.rsplit('/', 1)[1]).dblclick()
            app.file('example.py').dblclick()
            code = app.code()
            area = code.locator('.monaco-editor textarea.inputarea').first
            expect(code.locator('.view-lines').first).to_contain_text('print(1)')
            area.focus()
            area.press('Control+End')
            code.keyboard.insert_text('print(2)')
            area.press('Control+s')
            eventually(lambda: linux('cat', directory + '/example.py') == b'print(1)\nprint(2)')
            app.application('Files')
            app.file(clip.name).dblclick()
            video = app.page.locator('.remote-player video')
            eventually(lambda: video.evaluate('v => v.readyState >= 2 && v.duration >= 7'), timeout=25)
            video.evaluate('v => {v.currentTime = 5;}')
            eventually(lambda: video.evaluate('v => !v.seeking && v.currentTime >= 5 && v.readyState >= 2'))
            assert not json.loads(app.config.read_bytes())['config']['servers']
    finally:
        linux('rm', '-f', '--', path, directory + '/New file', directory + '/terminal-result',
              directory + '/example.py', directory + '/sample.mp4')
        linux('rmdir', '--', directory)


if __name__ == '__main__':
    main()
