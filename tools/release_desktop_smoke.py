"""Verify the actual Windows release build, public channel and themed editing."""
import json, time
from desktop_fixture import ROOT, Peer, Desktop, executable, expect


def main():
    out=ROOT/'artifacts/release-desktop-ui'/str(time.time_ns()); out.mkdir(parents=True)
    binary=executable().parent.parent/'release/arcnix.exe'
    with Peer(out) as peer:
        (peer.root/'release-edit.txt').write_bytes(b'original\r\n')
        config=peer.config([peer.profile('Release fixture')],out,updates=dict(automatic_checks=False,automatic_install=False))
        config['config']['settings']['theme']='Dark'
        with Desktop(out,config,binary=binary,extra_env={'ARCNIX_TEST_UPDATE_URL':'not-a-valid-url'}) as app:
            app.settings('About'); panel=app.page.locator('.update-panel')
            expect(panel).not_to_contain_text('Development build')
            panel.get_by_role('button',name='Check now',exact=True).click()
            expect(panel.locator('.update-status')).not_to_have_text('Checking for updates…',timeout=40000)
            status=panel.locator('.update-status').inner_text()
            assert status in ['No published release is available yet','You are using the latest stable version','A new version is available'], panel.inner_text()
            app.screenshot('release-update-channel.png')
            app.servers(); app.connect(); app.file('release-edit.txt').dblclick()
            editor=app.page.get_by_role('dialog',name='Edit remote file',exact=True)
            contents=editor.get_by_role('textbox',name='File contents',exact=True)
            expect(contents).to_have_value('original\n'); contents.fill('saved with shortcut\n'); contents.press('Control+s')
            expect(editor.locator('.editor-feedback')).to_have_text('Remote file saved')
            assert (peer.root/'release-edit.txt').read_bytes()==b'saved with shortcut\r\n'
            assert contents.evaluate('(el)=>getComputedStyle(el).backgroundColor')!='rgb(255, 255, 255)'
            app.page.set_viewport_size(dict(width=720,height=780)); app.screenshot('dark-editor-narrow.png')
            box=editor.bounding_box(); assert box['x']>=0 and box['x']+box['width']<=721
            result=dict(passed=2,checks=['Release builds ignore debug update-source overrides and successfully check the official stable channel', 'Release SSH editing preserves CRLF, Ctrl+S saves actual bytes, and the editor respects dark theme and a narrow viewport'],channel_status=status)
            (out/'results.json').write_text(json.dumps(result,indent=2)); print(json.dumps(result))


if __name__=='__main__':main()
