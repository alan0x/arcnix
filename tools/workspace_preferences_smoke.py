"""Real server-scoped locations, preferences, logs, conflicts and remote editing."""
from pathlib import Path
import json, os, time
from desktop_fixture import ROOT, Peer, Desktop, expect, eventually, close_dialog


def main():
    out = ROOT/'artifacts/workspace-preferences-ui'
    out.mkdir(parents=True, exist_ok=True)
    checks = []
    with Peer(out) as peer:
        local_a = out/f'local-a-{time.time_ns()}'; local_b = out/f'local-b-{time.time_ns()}'
        (local_a/'nested').mkdir(parents=True); local_b.mkdir()
        (local_a/'nested'/'size.bin').write_bytes(b'x'*1234567)
        (local_a/'nested'/'config.txt').write_bytes(b'uploaded replacement\n')
        (local_b/'only-b.txt').write_bytes(b'B')
        remote = peer.root/'folder'/'subdir'; remote.mkdir()
        original = b'\xef\xbb\xbftoken=fixture-editor-secret\r\nvalue=old\r\n'
        (remote/'config.txt').write_bytes(original)
        (remote/'binary.bin').write_bytes(b'\0binary')
        a = peer.profile('Server A', locations=dict(local_directory=str(local_a), remember_paths=True)); a['initial_remote_path']='/folder'
        b = peer.profile('Server B', locations=dict(local_directory=str(local_b), remember_paths=True))
        config = peer.config([a,b], local_a)
        with Desktop(out, config) as app:
            page = app.page; app.connect()
            app.file('nested', False).dblclick(); app.file('subdir').dblclick()
            expect(app.file('config.txt')).to_be_visible(); expect(app.file('size.bin', False)).to_be_visible()
            app.connect(1)
            expect(app.file('only-b.txt', False)).to_be_visible()
            app.connect(0)
            expect(app.file('size.bin', False)).to_be_visible(); expect(app.file('config.txt')).to_be_visible()
            saved = json.loads(app.config.read_text())['config']['servers']
            assert Path(saved[0]['locations']['last_local']).name=='nested'
            assert saved[0]['locations']['last_remote']=='/folder/subdir'
            assert saved[1]['locations']['last_remote']=='/'
            checks.append('Each server restores independent default and remembered local/remote paths across reconnections')

            app.settings('Files')
            page.get_by_role('button', name='Exact bytes', exact=True).click()
            page.get_by_role('checkbox', name='Use thousands separators', exact=True).check()
            page.get_by_role('button', name='ISO 8601', exact=True).click(); page.get_by_role('button', name='UTC', exact=True).click()
            app.servers()
            row = page.locator('.local-slot .file-row').filter(has=page.get_by_role('button', name='size.bin', exact=True))
            expect(row.locator('.file-size')).to_have_text('1,234,567 B')
            expect(row.locator('.file-modified')).to_contain_text('Z')
            app.settings('Connection'); page.get_by_role('button', name='60 seconds', exact=True).first.click()
            eventually(lambda: json.loads(app.config.read_text())['config']['settings']['connection']['timeout_seconds']==60)
            app.settings('Logging')
            page.get_by_role('checkbox', name='Write operation logs to files', exact=True).check()
            logs = out/'logs'; page.get_by_role('textbox', name='Log directory', exact=True).fill(str(logs))
            page.get_by_role('button', name='Apply directory', exact=True).click()
            page.get_by_role('checkbox', name='Write operation logs to files', exact=True).check()
            app.servers()
            app.file('size.bin', False).click(button='right'); app.action('Copy selected paths')
            content = eventually(lambda: (lambda text: text if 'Paths copied' in text else None)((logs/'arcnix.log').read_text(encoding='utf-8')))
            assert 'Paths copied' in content and content.startswith('[')
            checks.append('Display sizes and timestamps update live; connection defaults and timestamped file logging persist and write to the selected directory')

            app.file('config.txt').click(button='right'); app.action('Edit file')
            editor = page.get_by_role('dialog', name='Edit remote file', exact=True)
            contents = editor.get_by_role('textbox', name='File contents', exact=True)
            expect(contents).to_be_enabled(); expect(contents).to_have_value('token=fixture-editor-secret\nvalue=old\n')
            contents.fill('token=fixture-editor-secret\nvalue=new\n')
            editor.get_by_role('button', name='Save to server', exact=True).click()
            expect(editor.locator('.editor-feedback')).to_have_text('Remote file saved')
            assert (remote/'config.txt').read_bytes()==original.replace(b'value=old', b'value=new')
            app.screenshot('remote-editor.png')
            metadata = (remote/'config.txt').stat(); changed=(remote/'config.txt').read_bytes().replace(b'value=new', b'value=bad')
            (remote/'config.txt').write_bytes(changed); os.utime(remote/'config.txt', (metadata.st_atime, metadata.st_mtime))
            contents.fill('token=fixture-editor-secret\nvalue=client\n')
            editor.get_by_role('button', name='Save to server', exact=True).click()
            expect(editor.locator('.editor-feedback')).to_contain_text('changed')
            assert (remote/'config.txt').read_bytes()==changed
            editor.get_by_role('button', name='Close dialog', exact=True).click()
            expect(editor.get_by_role('button', name='Keep editing', exact=True)).to_be_visible()
            editor.get_by_role('button', name='Keep editing', exact=True).click()
            expect(contents).to_have_value('token=fixture-editor-secret\nvalue=client\n')
            editor.get_by_role('button', name='Close dialog', exact=True).click()
            editor.get_by_role('button', name='Discard changes', exact=True).click(); expect(editor).not_to_be_visible()
            app.file('binary.bin').click(button='right'); app.action('Edit file')
            expect(editor.locator('.editor-feedback')).to_contain_text('UTF-8')
            close_dialog(editor)
            assert 'fixture-editor-secret' not in (logs/'arcnix.log').read_text(encoding='utf-8')
            checks.append('Remote editing preserves BOM/CRLF, refuses same-size/same-time concurrent changes, confirms discarding drafts, rejects binary files and never logs file contents')

            app.settings('Transfers')
            section=page.locator('.settings-section').filter(has=page.get_by_text('Existing files', exact=True))
            section.get_by_role('button', name='Skip', exact=True).first.click()
            app.servers(); app.file('config.txt', False).click()
            page.locator('.local-slot').get_by_role('button', name='Upload', exact=True).click()
            expect(page.locator('.queue-row')).to_have_count(1); expect(page.locator('.queue-row').last).to_contain_text('Skipped 1 files')
            assert (remote/'config.txt').read_bytes()==changed
            page.locator('.remote-slot').get_by_role('button', name='Remote file actions', exact=True).click()
            page.locator('.file-actions-menu').get_by_role('button', name='Transfer defaults', exact=True).click()
            defaults=page.get_by_role('dialog', name='Transfer defaults', exact=True)
            defaults.get_by_role('button', name='Overwrite', exact=True).first.click()
            defaults.get_by_role('button', name='Done', exact=True).click()
            app.file('config.txt', False).click(); page.locator('.local-slot').get_by_role('button', name='Upload', exact=True).click()
            expect(page.locator('.queue-row')).to_have_count(2); expect(page.locator('.queue-row').last).to_contain_text('Completed')
            assert (remote/'config.txt').read_bytes()==b'uploaded replacement\n'
            checks.append('Global conflict defaults are applied by real transfers and explicit per-server rules override them')

            page.locator('.host-select').first.hover(); page.locator('.host-more').first.click()
            page.get_by_role('button', name='Edit server', exact=True).click()
            server=page.get_by_role('dialog', name='Edit server', exact=True)
            server.get_by_role('button', name='Files', exact=True).click()
            server.get_by_role('checkbox', name='Remember browsing locations for this server', exact=True).uncheck()
            server.get_by_role('textbox', name='Default local directory', exact=True).fill(str(local_a))
            server.get_by_role('button', name='Connection', exact=True).click()
            server.get_by_role('checkbox', name='Use application connection defaults', exact=True).uncheck()
            server.get_by_role('button', name='5 seconds', exact=True).click()
            app.screenshot('server-connection.png')
            server.get_by_role('button', name='Save server', exact=True).click(); expect(server).not_to_be_visible()
            config_now=json.loads(app.config.read_text())['config']
            assert config_now['servers'][0]['connection']['timeout_seconds']==5
            assert not config_now['servers'][0]['locations']['remember_paths']
            app.settings('Files'); app.screenshot('display-settings.png')
            app.stop(); app.launch(); page=app.page; app.connect()
            expect(app.file('nested', False)).to_be_visible(); expect(app.file('subdir')).to_be_visible()
            app.settings('Logging'); expect(page.get_by_role('checkbox', name='Write operation logs to files', exact=True)).to_be_checked()
            checks.append('Server editor separates General/Files/Connection, connection overrides persist, disabling location memory uses defaults, and a new process reloads preferences')
            result=dict(passed=len(checks),checks=checks); (out/'results.json').write_text(json.dumps(result,indent=2)); print(json.dumps(result))


if __name__=='__main__':main()
