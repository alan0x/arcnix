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
        page.get_by_role('button', name='Try the local desktop', exact=True).click()
        choice = page.locator('.wsl-distribution').filter(has=page.get_by_text(distro, exact=True))
        expect(choice.or_(page.locator('.desktop-dock:visible'))).to_be_visible(timeout=30000)
        if choice.is_visible():
            choice.click()
        expect(page.locator('.connection-state')).to_have_text('Connected locally', timeout=30000)
        expect(page.locator('.desktop-dock')).to_be_visible()
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
        with Desktop(out, None) as app:
            enter(app)
            app.application('Settings')
            panel = app.page.locator('.app-settings.active')
            panel.get_by_role('button', name='Appearance', exact=True).click()
            expect(panel.get_by_role('radio', name='Dark', exact=True)).to_be_checked()
            assert not json.loads(app.config.read_bytes())['config']['servers']
    finally:
        linux('rm', '-f', '--', path, directory + '/New file', directory + '/terminal-result')
        linux('rmdir', '--', directory)


if __name__ == '__main__':
    main()
