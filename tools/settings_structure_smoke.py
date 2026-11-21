"""Validate settings ownership, empty feedback, real I/O errors and saved choices."""
import json, time, re
from desktop_fixture import ROOT, Desktop, Peer, expect, expect_radio_options, eventually


def main():
    out=ROOT/'artifacts/settings-structure-ui'/str(time.time_ns()); out.mkdir(parents=True)
    checks=[]
    with Peer(out) as peer:
        (out/'settings-check.txt').write_text('settings fixture')
        config=peer.config([peer.profile('Settings fixture')],out,updates=dict(automatic_checks=False,automatic_install=False))
        with Desktop(out,config) as app:
            page=app.page
            expected={
                'General':['Language','Theme','Configuration backup'],
                'Connection':['Timeout and keepalive'],
                'Files':['Hidden files','File sizes','Dates and times','Default apps'],
                'Transfers':['Concurrent transfers','Existing files'], 'Monitoring':['Resource refresh interval'],
                'Logging':['Operation log','Log files'], 'Updates':['Updates'], 'About':['Application'],
            }
            for category,titles in expected.items():
                app.settings(category)
                expect(page.locator('.page-header-title')).to_have_text(category)
                expect(page.locator('.settings-section:visible .settings-section-title')).to_have_text(titles)
                expect(page.locator('.settings-page .software-error:visible')).to_have_count(0)
                if category!='Updates': expect(page.locator('.update-panel')).not_to_be_visible()
            expect(page.locator('.context-links').get_by_role('button',name='Appearance',exact=True)).to_have_count(0)
            expect(page.locator('.context-links').get_by_role('button').last).to_have_text('About')
            checks.append('Each settings category owns its intended sections, with update controls under Updates')
            app.settings('About'); expect(page.locator('.about-identity .subtle')).to_have_text(re.compile(r'Version \d+\.\d+\.\d+'))
            page.locator('.about-identity .subtle').dblclick(position={'x': 8, 'y': 8})
            assert page.evaluate('window.getSelection().toString().trim()') == 'Version'
            expect(page.get_by_role('button',name='Download update',exact=True)).not_to_be_visible()
            expect(page.get_by_role('button',name='Restart and install',exact=True)).not_to_be_visible()
            app.screenshot('about-light.png')
            expect(page.get_by_role('button',name='Project on GitHub',exact=True)).to_be_visible()
            expect(page.get_by_role('button',name='Official website · arcnix.org',exact=True)).to_be_visible()
            expect(page.get_by_role('button',name='Check now',exact=True)).not_to_be_visible()
            checks.append('About displays the installed version and repository link, and is the last settings category')
            app.settings('Logging')
            export=page.get_by_role('button',name='Export current log…',exact=True)
            assert export.bounding_box()['width']<300
            expect(page.get_by_role('textbox',name='Log directory',exact=True)).not_to_be_visible()
            page.get_by_role('checkbox',name='Write operation logs to files',exact=True).check()
            directory=page.get_by_role('textbox',name='Log directory',exact=True)
            expect(directory).to_have_value(str(out/'logs'))
            directory.press('Control+a')
            assert directory.evaluate('el => el.value.slice(el.selectionStart, el.selectionEnd)') == str(out/'logs')
            expect(page.get_by_role('button',name='Apply directory',exact=True)).to_be_disabled()
            app.screenshot('logging-light.png')
            bad=out/'not-a-directory'; bad.write_text('preserve this file')
            directory.fill(str(bad)); page.get_by_role('button',name='Apply directory',exact=True).click()
            app.servers(); app.connect(); app.settings('Logging')
            expect(page.locator('.log-error')).to_contain_text('Log file error:',timeout=10000)
            assert page.locator('.log-error').inner_text().strip() and bad.read_text()=='preserve this file'
            good=out/'valid-logs'
            directory.fill(str(good)); page.get_by_role('button',name='Apply directory',exact=True).click()
            expect(page.locator('.log-error')).not_to_be_visible()
            app.servers(); app.file('settings-check.txt',False).click(button='right'); app.action('Copy selected paths')
            eventually(lambda: (good/'arcnix.log').is_file())
            app.settings('Logging'); expect(page.locator('.log-error')).not_to_be_visible()
            checks.append('Logging separates display/export from optional file output, collapses inactive options, uses a compact export action, shows real I/O errors only when present and clears recovered errors')
            app.settings('Files')
            sizes=page.locator('.settings-section').filter(has=page.get_by_text('File sizes',exact=True))
            expect_radio_options(sizes.get_by_role('radiogroup',name='File size units').get_by_role('radio'), ['Decimal · KB / MB', 'IEC · KiB / MiB'])
            expect_radio_options(sizes.get_by_role('radiogroup',name='Decimal places').get_by_role('radio'), ['0', '1', '2'])
            preview = sizes.locator('.preference-preview')
            expect(preview).to_have_text('Preview: 123.5 MB')
            sizes.locator('.setting-file-size-units:visible').get_by_role('radio', name='IEC · KiB / MiB', exact=True).check()
            sizes.locator('.setting-decimal-places:visible').get_by_role('radio', name='2', exact=True).check()
            expect(preview).to_have_text('Preview: 117.74 MiB')
            dates=page.locator('.settings-section').filter(has=page.get_by_text('Dates and times',exact=True))
            dates.locator('.setting-date-format:visible').get_by_role('radio', name='ISO 8601', exact=True).check()
            sizes.locator('.settings-card-heading').get_by_role('button', name='Reset', exact=True).click()
            expect(preview).to_have_text('Preview: 123.5 MB')
            eventually(lambda: json.loads(app.config.read_text(encoding='utf-8'))['config']['settings']['display']['units']=='Decimal')
            assert json.loads(app.config.read_text(encoding='utf-8'))['config']['settings']['display']['dates']=='Iso'
            app.screenshot('files-light.png')
            app.settings('General'); page.locator('.setting-theme:visible').get_by_role('radio', name='Dark', exact=True).check()
            app.settings('Logging'); app.screenshot('logging-dark.png')
            page.set_viewport_size(dict(width=850,height=780)); app.screenshot('logging-narrow.png')
            assert page.locator('.settings-page').evaluate('(el)=>el.scrollWidth<=el.clientWidth')
            app.settings('General'); page.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check()
            page.locator('.context-links').get_by_role('button',name='关于',exact=True).click()
            expect(page.locator('.page-header-title')).to_have_text('关于')
            expect(page.locator('.about-identity .subtle')).to_have_text(re.compile(r'版本 \d+\.\d+\.\d+'))
            app.screenshot('about-chinese-dark.png')
            checks.append('File size previews switch units and precision and reset to decimal with one place; settings support dark/light themes, narrow windows and live Chinese translation')
            app.stop(); app.launch(); page=app.page
            page.locator('.settings-link').click(); page.locator('.context-links').get_by_role('button',name='日志设置',exact=True).click()
            expect(page.get_by_role('checkbox',name='将操作日志写入文件',exact=True)).to_be_checked()
            expect(page.get_by_role('textbox',name='日志目录',exact=True)).to_have_value(str(good))
            checks.append('A fresh process restores the same language, theme and file log preferences after the navigation restructure')
    result=dict(passed=len(checks),checks=checks,artifacts=str(out)); (out/'results.json').write_text(json.dumps(result,indent=2)); print(json.dumps(result))


if __name__=='__main__':main()
