"""Verify help interactions and readable layouts in the owned desktop."""
import json, time
from desktop_fixture import Peer, Desktop, expect, ROOT

out = ROOT/'artifacts/help-readability-ui'/str(time.time_ns())
checks = []

def passed(message):
    checks.append(message)
    print(message, flush=True)

def within_viewport(page):
    box = page.get_by_role('tooltip').bounding_box()
    assert box and box['x'] >= 10 and box['y'] >= 10
    assert box['x'] + box['width'] <= page.viewport_size['width'] - 10
    assert box['y'] + box['height'] <= page.viewport_size['height'] - 10

def readable(page):
    misplaced = page.locator('.help-trigger:visible').evaluate_all('''elements => elements.filter(el => {
      const trigger=getComputedStyle(el), circle=getComputedStyle(el,'::before');
      const size=axis => parseFloat(circle[axis]) + (circle.boxSizing==='border-box' ? 0 :
        parseFloat(circle[axis==='width' ? 'borderLeftWidth' : 'borderTopWidth']) * 2);
      const centerX=parseFloat(circle.left)+parseFloat(circle.marginLeft)+size('width')/2;
      const centerY=parseFloat(circle.top)+parseFloat(circle.marginTop)+size('height')/2;
      return trigger.position!=='relative' || Math.abs(centerX-el.clientWidth/2)>1 || Math.abs(centerY-el.clientHeight/2)>1;
    }).map(el => el.getAttribute('aria-label'))''')
    assert not misplaced, misplaced
    small = page.evaluate("""() => [...document.querySelectorAll('body *')]
      .filter(el => !el.children.length && el.textContent.trim() && el.getClientRects().length &&
        getComputedStyle(el).visibility !== 'hidden' && parseFloat(getComputedStyle(el).fontSize) > 0 &&
        parseFloat(getComputedStyle(el).fontSize) < 12)
      .map(el => ({text: el.textContent.slice(0,80), size: getComputedStyle(el).fontSize}))""")
    assert not small, small
    assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
    assert page.locator('.main-panel').evaluate('el => el.scrollWidth <= el.clientWidth')

with Peer(out/'peer') as peer:
    config = peer.config([peer.profile('Production server 中文')], out)
    config['config']['snippets'] = [dict(id='help-example',name='Help example',description='',
        command='echo {{message}}',administrator=False,parameters=[dict(name='message',kind='text',
        label='Message',description='Tasks',required=True,default_value='hello',allow_preset=False)])]
    with Desktop(out/'desktop', config) as app:
        p = app.page
        p.locator('.rail-scripts').click()
        expect(p.locator('.context-sidebar .context-hint')).to_have_count(0)
        expect(p.get_by_text('Jobs run as server-owned systemd services. Closing Arcnix does not stop submitted jobs.', exact=True)).not_to_be_visible()
        tip = p.locator('.context-title').get_by_role('button', name='Help: Server tasks', exact=True)
        tip.hover()
        expect(p.get_by_role('tooltip')).to_contain_text('Closing Arcnix does not stop submitted jobs')
        within_viewport(p)
        panel = p.get_by_role('tooltip').bounding_box()
        p.mouse.move(panel['x']+20,panel['y']+20)
        expect(p.get_by_role('tooltip')).to_be_visible()
        p.mouse.move(1400,950)
        expect(p.get_by_role('tooltip')).not_to_be_visible()
        passed('Help is hidden initially, appears on hover, stays readable under the pointer and closes after leaving')
        tip.click(); p.mouse.move(1400,950)
        expect(p.get_by_role('tooltip')).to_be_visible()
        tip.click(); expect(p.get_by_role('tooltip')).not_to_be_visible()
        tip.click(); p.locator('.page-header-title').click()
        expect(p.get_by_role('tooltip')).not_to_be_visible()
        tip.focus(); expect(p.get_by_role('tooltip')).to_be_visible()
        p.keyboard.press('Escape'); expect(p.get_by_role('tooltip')).not_to_be_visible()
        assert tip.get_attribute('aria-describedby') and tip.get_attribute('aria-expanded') == 'false'
        passed('Click pins and toggles help; outside click and Escape dismiss it; keyboard focus and accessible descriptions work')
        readable(p); app.screenshot('scripts-light.png')
        p.locator('.script-row').filter(has=p.get_by_text('Install Nginx',exact=True)).get_by_role('button',name='Run on servers…',exact=True).click()
        d = p.get_by_role('dialog', name='Run script on servers', exact=True)
        help_button = d.get_by_role('button', name='Help: Installation policy', exact=True)
        help_button.click(); expect(p.get_by_role('tooltip')).to_contain_text('preserves installed packages')
        within_viewport(p); app.screenshot('installation-help.png')
        p.keyboard.press('Escape'); expect(d).to_be_visible(); expect(p.get_by_role('tooltip')).not_to_be_visible()
        expect(d).to_contain_text('Failed installation can leave partial changes')
        readable(p)
        p.set_viewport_size(dict(width=900,height=700))
        help_button.click(); within_viewport(p); readable(p)
        app.screenshot('installation-narrow.png')
        p.keyboard.press('Escape'); d.get_by_role('button',name='Cancel',exact=True).click()
        passed('Installation help fits the modal and narrow viewport; Escape preserves the form and the failure warning stays visible')
        for rail in ['tasks','backups','diagnostics']:
            p.locator('.rail-'+rail).click(); readable(p)
            assert p.locator('.context-sidebar .context-hint').count() == 0
            app.screenshot(rail+'-narrow.png')
        passed('Scripts, Tasks, Backups and Diagnostics have no tiny text or horizontal overflow at 900 × 700')
        p.set_viewport_size(dict(width=1440,height=980))
        app.settings('General')
        language = p.get_by_role('button',name='Help: Language',exact=True)
        language.click(); expect(p.get_by_role('tooltip')).to_contain_text('Choose the language')
        p.keyboard.press('Escape')
        p.get_by_role('button',name='Dark',exact=True).click()
        expect(p.locator('.app')).to_have_class('app theme-dark')
        p.get_by_role('button',name='简体中文',exact=True).click()
        expect(p.locator('.page-header-title')).to_have_text('常规')
        p.locator('.rail-scripts').click()
        expect(p.locator('.page-header-title')).to_have_text('运维脚本')
        p.get_by_role('button',name='帮助：服务器任务',exact=True).hover()
        expect(p.get_by_role('tooltip')).not_to_contain_text('Closing Arcnix')
        within_viewport(p); readable(p); app.screenshot('scripts-chinese-dark-help.png')
        p.mouse.move(1400,950)
        expect(p.get_by_role('tooltip')).not_to_be_visible()
        p.locator('.context-links').get_by_role('button',name='我的脚本',exact=True).click()
        expect(p.locator('.script-row')).to_have_count(1)
        expect(p.locator('.script-row')).to_contain_text('Help example')
        p.locator('.script-row').get_by_role('button',name='在服务器上运行…',exact=True).click()
        d = p.get_by_role('dialog')
        d.get_by_role('button',name='帮助：参数说明',exact=True).click()
        expect(p.get_by_role('tooltip')).to_contain_text('Tasks')
        p.keyboard.press('Escape'); d.get_by_role('button',name='取消',exact=True).click()
        app.servers(); p.locator('.host-select').first.click(); p.locator('.overview-connect').click()
        p.get_by_role('dialog',name='连接服务器',exact=True).get_by_role('button',name='连接',exact=True).click()
        expect(p.locator('.connection-state')).to_have_text('SSH 已连接')
        readable(p); app.screenshot('files-chinese-dark.png')
        passed('Help and typography work with live Chinese translation, dark/light themes and the connected file workspace')
        assert not p.locator('.help-visible').count()

(out/'results.json').write_text(json.dumps(dict(passed=len(checks),checks=checks),ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps(dict(passed=len(checks),artifacts=str(out)),ensure_ascii=False))
