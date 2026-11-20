"""Verify the actual Windows release build, public channel and themed editing."""
import json, time
from desktop_fixture import ROOT, Peer, Desktop, executable, expect, eventually


def main():
    out=ROOT/'artifacts/release-desktop-ui'/str(time.time_ns()); out.mkdir(parents=True)
    binary=executable().parent.parent/'release/arcnix.exe'
    with Peer(out) as peer:
        (peer.root/'release-edit.txt').write_bytes(b'original\r\n')
        config=peer.config([peer.profile('Release fixture')],out,updates=dict(automatic_checks=False,automatic_install=False))
        config['config']['settings']['theme']='Dark'
        config['config']['settings']['file_associations']={'txt': {'default': 'code'}}
        with Desktop(out,config,binary=binary,extra_env={'ARCNIX_TEST_UPDATE_URL':'not-a-valid-url'}) as app:
            app.settings('Updates'); panel=app.page.locator('.update-panel')
            expect(panel).not_to_contain_text('Development build')
            panel.get_by_role('button',name='Check now',exact=True).click()
            expect(panel.locator('.update-status')).not_to_have_text('Checking for updates…',timeout=40000)
            status=panel.locator('.update-status').inner_text()
            assert status in ['No published release is available yet','You are using the latest stable version','A new version is available'], panel.inner_text()
            app.screenshot('release-update-channel.png')
            app.servers(); app.connect(); app.file('release-edit.txt').dblclick()
            code = app.code()
            expect(code.locator('.view-lines')).to_contain_text('original')
            app.replace_code('saved with shortcut\n').press('Control+s')
            eventually(lambda: (peer.root/'release-edit.txt').read_bytes()==b'saved with shortcut\r\n')
            expect(code.locator('.monaco-workbench.vs-dark')).to_be_visible()
            app.page.set_viewport_size(dict(width=720,height=780))
            eventually(lambda: code.evaluate('innerWidth') <= 720)
            code.screenshot(path=str(out/'dark-code-narrow.png'))
            result=dict(passed=2,checks=['Release builds ignore debug update-source overrides and successfully check the official stable channel', 'Release SSH editing preserves CRLF, Ctrl+S saves actual bytes, and the editor respects dark theme and a narrow viewport'],channel_status=status)
            (out/'results.json').write_text(json.dumps(result,indent=2)); print(json.dumps(result))


if __name__=='__main__':main()
