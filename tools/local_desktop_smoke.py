"""Exercise the homepage local desktop on a native Windows host, in both access modes."""
import json
from pathlib import Path
import subprocess
import tempfile

from desktop_fixture import Desktop, ROOT, eventually, expect, set_desktop_mode


def main():
    def enter(app, mode):
        page = app.page
        entry = page.locator('.server-explorer').get_by_role('button', name='Try the local desktop', exact=True)
        expect(page.locator('.local-desktop-entry + .server-search')).to_be_visible()
        entry.click()
        dialog = page.get_by_role('dialog', name='Try the local desktop', exact=True)
        expect(dialog.get_by_text('Read-only is recommended', exact=True)).to_be_visible()
        dialog.get_by_role('button', name=mode, exact=True).click()
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
        if mode == 'Read-only mode':
            expect(catalog.get_by_role('button', name='Terminal', exact=True)).to_have_count(0)
        catalog.get_by_role('button', name='Close dialog', exact=True).click()
        expect(page.locator('.workbench-tabs').get_by_role('button', name='Transfer files')).to_have_count(0)
        assert 'Could not read directory' not in (app.out / 'logs/arcnix.log').read_text(encoding='utf-8')

    name = "local 桌面's note.md"
    out = Path(tempfile.mkdtemp(prefix='local-desktop-', dir=ROOT / 'artifacts'))
    cfg = dict(version=1, config=dict(servers=[], groups=[], trusted_hosts=[],
        local_directory=str(out / 'unmounted-transfer-directory'), settings=dict(
        language='English', theme='Light', show_hidden_files=False, resource_interval_seconds=3,
        logging=dict(write_file=True))))
    with tempfile.TemporaryDirectory(prefix='arcnix-demo-', dir=Path.home()) as temp:
        directory = Path(temp)
        with Desktop(out, cfg) as app:
            enter(app, 'Read-only mode')
            rust = directory / 'readonly.rs'
            source = 'fn main() {}'
            rust.write_text(source, encoding='utf-8')
            (directory / 'readonly.txt').write_text('Host file', encoding='utf-8')
            app.application('Files')
            app.file(directory.name).dblclick()
            app.file('readonly.txt').click(button='right')
            expect(app.page.locator('.file-context-menu:visible').get_by_role('button', name='Rename', exact=True)).to_be_disabled()
            app.page.keyboard.press('Escape')
            app.file('readonly.txt').dblclick()
            area = app.page.locator('.app-editor.active .remote-editor-content')
            expect(area).to_have_value('Host file')
            expect(area).not_to_be_editable()
            app.page.locator('.app-editor.active .window-close').click()
            (directory / 'readonly.txt').unlink()
            app.file(rust.name).dblclick()
            code = app.code()
            lines = code.locator('.monaco-editor .view-lines').first
            expect(lines).to_contain_text(source)
            expect(code.get_by_role('menubar').get_by_role('menuitem', name='Terminal', exact=True)).to_have_count(0)
            code.keyboard.press('Control+Backquote')
            expect(code.locator('.xterm:visible')).to_have_count(0)
            # The command palette and shortcuts must not be able to invoke a missing backend.
            assert code.evaluate('''async () => {
                try { await window.arcnixCommand('workbench.action.terminal.new'); return false; }
                catch (error) { return /command .* not found/i.test(error.message); }
            }''')
            document = code.evaluate_handle('document')
            app.page.locator('.app-code.active .window-minimize').click()
            app.application('Code Editor')
            assert code.evaluate('old => document === old', document)
            expect(lines).to_contain_text(source)
            rendered = lines.inner_text()
            code.locator('.monaco-editor textarea.inputarea').first.press('x')
            assert lines.inner_text() == rendered
            app.page.locator('.app-code.active .window-close').click()
            expect(app.page.locator('.app-code.active')).to_have_count(0)
            app.application('Files')
            app.file(rust.name).click(button='right')
            app.page.locator('.file-context-menu:visible').get_by_role('button', name='Open with', exact=True).hover()
            app.action('Text Editor')
            expect(area).to_have_value(source)
            expect(area).not_to_be_editable()
            app.page.locator('.app-editor.active .window-close').click()
            assert rust.read_text(encoding='utf-8') == source
            rust.unlink()
            app.application('Monitor')
            expect(app.page.locator('.monitor-metric-grid')).to_contain_text('logical processors', timeout=30000)
            app.page.locator('.app-monitor.active').get_by_role('button', name='Disks', exact=True).click()
            expect(app.page.locator('.disk-card').first).to_be_visible()
            expect(app.page.locator('.disk-donut').first).to_be_visible()
            expect(app.page.locator('.disk-mount-link').first).to_be_visible()
            expect(app.page.locator('.disks-error')).to_have_text('')
            enter(app, 'Read-write mode')
            app.application('Files')
            app.file(directory.name).dblclick()
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
            eventually(lambda: (directory / name).read_bytes() == b'# Local desktop\n')
            app.page.locator('.app-editor.active .window-close').click()
            app.application('Files')
            app.file(name).click(button='right')
            app.action('Terminal here')
            screen = app.page.locator('.app-terminal.active .xterm-accessibility-tree')
            keyboard = app.page.locator('.app-terminal.active .xterm-helper-textarea')
            keyboard.focus()
            app.page.keyboard.type("$PSVersionTable.PSVersion.Major | Out-File -Encoding ascii terminal-result")
            app.page.keyboard.press('Enter')
            eventually(lambda: int((directory / 'terminal-result').read_text().strip()) >= 5)
            expect(screen).to_be_visible()
            app.application('Settings')
            set_desktop_mode(app.page.locator('.app-settings.active'), 'Dark')
            prefs = eventually(lambda: next(out.glob('self-demo-server/local-windows/desktop.json'), None))
            eventually(lambda: json.loads(prefs.read_bytes()).get('appearance', {}).get('overrides', {}).get('mode') == 'Dark')
        (directory / 'example.py').write_bytes(b'print(1)\n')
        clip = out / 'sample.mp4'
        subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-f', 'lavfi', '-i',
                        'testsrc2=size=320x180:rate=12', '-t', '8', '-c:v', 'libx264',
                        '-pix_fmt', 'yuv420p', str(clip)], check=True)
        (directory / clip.name).write_bytes(clip.read_bytes())
        with Desktop(out, None) as app:
            enter(app, 'Read-write mode')
            app.application('Settings')
            panel = app.page.locator('.app-settings.active')
            panel.get_by_role('button', name='Appearance', exact=True).click()
            expect(panel.get_by_role('radio', name='Dark', exact=True)).to_be_checked()
            app.application('Files')
            app.file(directory.name).dblclick()
            app.file('example.py').dblclick()
            code = app.code()
            area = code.locator('.monaco-editor textarea.inputarea').first
            expect(code.locator('.view-lines').first).to_contain_text('print(1)')
            code.keyboard.press('Control+Backquote')
            rows = code.locator('.xterm-rows:visible').last
            expect(rows).to_contain_text('PS ', timeout=15000)
            keyboard = code.locator('.xterm-helper-textarea:visible').last
            keyboard.focus()
            code.keyboard.type('Write-Output (6 * 7)')
            keyboard.press('Enter')
            expect(rows).to_contain_text('42')
            code.evaluate("window.arcnixCommand('workbench.action.terminal.kill')")
            expect(rows).to_have_count(0)
            area.focus()
            area.press('Control+End')
            code.keyboard.insert_text('print(2)')
            area.press('Control+s')
            eventually(lambda: (directory / 'example.py').read_bytes() == b'print(1)\nprint(2)')
            app.application('Files')
            app.file(clip.name).dblclick()
            video = app.page.locator('.remote-player video')
            eventually(lambda: video.evaluate('v => v.readyState >= 2 && v.duration >= 7'), timeout=25)
            video.evaluate('v => {v.currentTime = 5;}')
            eventually(lambda: video.evaluate('v => !v.seeking && v.currentTime >= 5 && v.readyState >= 2'))
            assert not json.loads(app.config.read_bytes())['config']['servers']


if __name__ == '__main__':
    main()
