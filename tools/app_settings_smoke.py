"""Check per-app settings, persistence and folder-to-terminal window routing."""
import subprocess
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def main():
    out = ROOT / 'artifacts/app-settings' / str(time.time_ns())
    with Peer(out / 'peer') as peer:
        (peer.root / 'Projects').mkdir()
        (peer.root / 'note.txt').write_text('First line\nSecond line')
        subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-f', 'lavfi', '-i',
                        'color=size=160x90:rate=6', '-t', '6', '-c:v', 'libx264',
                        '-pix_fmt', 'yuv420p', str(peer.root / 'sample.mp4')], check=True)
        with Desktop(out / 'app', peer.config([peer.profile('App settings')], out)) as app:
            app.connect()
            page = app.page
            dialog = page.get_by_role('dialog', name='Settings', exact=True)

            def settings(app_class):
                page.locator(f'.desktop-window.active.app-{app_class} .desktop-window-actions').get_by_role(
                    'button', name='Settings', exact=True).click()
                expect(dialog).to_be_visible()

            def close_settings():
                dialog.get_by_role('button', name='Close dialog', exact=True).click()

            def terminal(destination):
                app.application('Files')
                app.file('Projects').click(button='right')
                menu = page.locator('.file-context-menu:visible')
                expect(menu.get_by_role('button', name='Open folder', exact=True)).to_have_count(0)
                menu.get_by_role('button', name='Open in', exact=True).hover()
                expect(menu.locator('.file-open-submenu button')).to_have_text(
                    ['New tab', 'New window', 'New Terminal tab', 'New Terminal window'])
                app.open_in(destination)
                active = page.locator('.desktop-window.app-terminal.active')
                expect(active.locator('.terminal-status')).to_have_text('Interactive SSH')
                active.locator('.xterm-helper-textarea').focus()
                page.keyboard.type('pwd')
                page.keyboard.press('Enter')
                expect(active.locator('.xterm-accessibility-tree')).to_contain_text('/Projects')

            terminal('New Terminal tab')
            first = page.locator('.tab-window-app-terminal-0')
            settings('terminal')
            dialog.get_by_role('combobox', name='Font size').select_option(label='18 px')
            dialog.get_by_role('combobox', name='Font family').select_option(label='Consolas')
            dialog.get_by_role('radio', name='Underline', exact=True).check()
            dialog.get_by_role('radio', name='Off', exact=True).check()
            close_settings()
            expect(first.locator('.xterm-rows')).to_have_css('font-size', '18px')
            terminal('New Terminal window')
            second = page.locator('.tab-window-app-terminal-1')
            expect(second.locator('.terminal-tab')).to_have_count(1)
            terminal('New Terminal tab')
            expect(second.locator('.terminal-tab')).to_have_count(2)
            # Minimize the overlapping second window and explicitly activate the first.
            second.locator('.window-minimize').click()
            page.locator('.desktop-window.app-files.active .window-minimize').click()
            first.locator('.terminal-tab').first.click()
            terminal('New Terminal tab')
            expect(first.locator('.terminal-tab')).to_have_count(2)
            expect(second.locator('.terminal-tab')).to_have_count(2)

            app.application('Files')
            app.file('note.txt').dblclick()
            editor = page.locator('.desktop-window.app-editor.active')
            expect(editor.locator('textarea.remote-editor-content')).to_be_visible()
            settings('editor')
            dialog.get_by_role('combobox', name='Font size').select_option(label='20 px')
            dialog.get_by_role('radio', name='Off', exact=True).check()
            close_settings()
            expect(editor.locator('textarea.remote-editor-content')).to_have_css('font-size', '20px')
            expect(editor.locator('.editor-gutter')).to_be_hidden()

            app.application('Files')
            app.file('sample.mp4').dblclick()
            video = page.locator('.remote-player video')
            eventually(lambda: video.evaluate('v => v.readyState >= 2'))
            settings('player')
            dialog.get_by_role('combobox', name='Default volume').select_option(label='40%')
            dialog.get_by_role('combobox', name='Playback speed').select_option(label='1.5×')
            close_settings()
            eventually(lambda: video.evaluate('v => v.volume === .4 && v.playbackRate === 1.5'))

            app.stop()
            app.launch()
            app.connect()
            page = app.page
            dialog = page.get_by_role('dialog', name='Settings', exact=True)
            app.application('Terminal')
            settings('terminal')
            expect(dialog.get_by_role('combobox', name='Font size').locator('option:checked')).to_have_text('18 px')
            expect(dialog.get_by_role('radio', name='Off', exact=True)).to_be_checked()
            close_settings()
            app.application('Files')
            app.file('note.txt').dblclick()
            settings('editor')
            expect(dialog.get_by_role('combobox', name='Font size').locator('option:checked')).to_have_text('20 px')
            expect(dialog.get_by_role('radio', name='Off', exact=True)).to_be_checked()
            close_settings()
            app.application('Files')
            app.file('sample.mp4').dblclick()
            video = page.locator('.remote-player video')
            eventually(lambda: video.evaluate('v => v.readyState >= 2 && v.volume === .4 && v.playbackRate === 1.5'))
            print('Verified folder routing, live app settings and persistence after relaunch.')


if __name__ == '__main__':
    main()
