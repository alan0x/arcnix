"""Per-server rendered candidates, private history and matching recovery metadata."""
import json,time,re,hashlib
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/configuration-parameters-ui'/str(time.time_ns());checks=[]
body='name={{name}}\nport={{port}}\nliteral={{{{keep}}}}\n'
default="private-template-default ' $(id) {{port}}";beta_value='private-template-beta'
def passed(value):checks.append(value);print(value,flush=True)
def history(app):return eventually(lambda:json.loads(app.config.with_suffix('.tasks.json').read_text(encoding='utf-8')))
def staged(peer,task):return peer.root/'home/arcnix/.local/state/arcnix/config-staging'/task
def target(dialog,name):return dialog.locator('.script-target-picker').filter(has=dialog.page.get_by_role('button',name=re.compile('^'+name+' ·')))
def editor(p,content=body,parameterized=True):
    p.locator('.rail-configurations').click()
    if not p.locator('.configuration-content:visible').count():p.get_by_role('button',name='Prepare configuration change…',exact=True).click()
    p.get_by_role('button',name='Custom application configuration',exact=True).click()
    for label,value in [('Configuration destination','/srv/template.conf'),('Candidate validation command','test -s "$1"'),('Reload command','true'),('Health check command','true')]:p.get_by_role('textbox',name=label,exact=True).fill(value)
    p.get_by_role('textbox',name='Candidate configuration (UTF-8)',exact=True).fill(content)
    button=p.get_by_role('button',name='Use configuration parameters',exact=True)
    if ('active' in button.get_attribute('class').split())!=parameterized:button.click()
def review(p):
    p.get_by_role('button',name='Review configuration on servers…',exact=True).click()
    d=p.get_by_role('dialog',name='Run script on servers',exact=True);expect(d).to_be_visible();return d
def select_all(d):d.get_by_role('button',name='Select visible trusted servers',exact=True).click()
def defaults(d,name=default,port='8080'):
    d.get_by_role('textbox',name='Default value: name',exact=True).fill(name);d.get_by_role('textbox',name='Default value: port',exact=True).fill(port)
def override(row,key,value):
    button=row.get_by_role('button',name=f'Override: {key}',exact=True)
    if 'active' not in button.get_attribute('class').split():button.click()
    row.get_by_role('textbox',name=f'Server value: {key}',exact=True).fill(value)
def finish(p):
    p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Succeeded']*p.locator('.task-status').count(),timeout=25000)

with Peer(out/'alpha') as a,Peer(out/'beta') as b:
    config=a.config([a.profile('Alpha'),b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    for peer in [a,b]:(peer.root/'configuration-strict-restore').touch()
    with Desktop(out/'desktop',config) as app:
        p=app.page;editor(p,'literal={{untouched}}\n',False);d=review(p)
        expect(d.locator('.configuration-parameter-defaults')).to_have_count(0);target(d,'Alpha').locator('.target-choice').click();d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready'],timeout=20000)
        legacy=history(app)[0];assert (staged(a,legacy['id'])/'candidate').read_text(encoding='utf-8')=='literal={{untouched}}\n' and 'configuration_targets' not in legacy
        finish(p);passed('Parameter mode is explicit; ordinary configuration braces pass unchanged through actual SSH/SFTP staging and legacy history remains compatible')

        editor(p);p.get_by_role('textbox',name='Default value: name',exact=True).fill(default);p.get_by_role('textbox',name='Default value: port',exact=True).fill('8080')
        d=review(p);select_all(d);beta=target(d,'Beta');beta.get_by_role('button',name='Configuration values…',exact=True).click();override(beta,'port','9090')
        d.get_by_role('textbox',name='Default value: port',exact=True).fill('8082');expect(beta.get_by_role('textbox',name='Server value: port',exact=True)).to_have_value('9090')
        beta.get_by_role('button',name='Use default values',exact=True).click();expect(beta.get_by_role('textbox',name='Server value: port',exact=True)).to_have_value('8082');expect(beta.get_by_role('textbox',name='Server value: port',exact=True)).to_be_disabled();override(beta,'port','9090');d.get_by_role('textbox',name='Default value: port',exact=True).fill('8080')
        d.get_by_role('textbox',name='Search target servers',exact=True).fill('Alpha');expect(d.get_by_role('button',name='Check targets',exact=True)).to_be_disabled();d.get_by_role('button',name='Review selected servers',exact=True).click()
        beta=target(d,'Beta');expect(beta.get_by_role('textbox',name='Server value: port',exact=True)).to_have_value('9090')
        d.get_by_role('button',name='Clear target filters',exact=True).click();beta.locator('.target-choice').click();beta.locator('.target-choice').click();beta.get_by_role('button',name='Configuration values…',exact=True).click()
        expect(beta.get_by_role('textbox',name='Server value: port',exact=True)).to_have_value('8080');expect(beta.get_by_role('textbox',name='Server value: port',exact=True)).to_be_disabled()
        override(beta,'name',beta_value);override(beta,'port','8081')
        d.get_by_role('button',name='Cancel',exact=True).click();d=review(p);select_all(d);beta=target(d,'Beta');beta.get_by_role('button',name='Configuration values…',exact=True).click()
        expect(beta.get_by_role('textbox',name='Server value: port',exact=True)).to_have_value('8080');expect(beta.get_by_role('textbox',name='Server value: port',exact=True)).to_be_disabled()
        passed('Filtering preserves selected overrides; hidden targets require review, and deselection or cancelling discards per-server values without changing editor defaults')

        override(beta,'name',beta_value);override(beta,'port','8081')
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=20000)
        task=history(app)[0];expected=[f'name={default}\nport=8080\nliteral={{{{keep}}}}\n',f'name={beta_value}\nport=8081\nliteral={{{{keep}}}}\n']
        for peer,id,text in [(a,'Alpha',expected[0]),(b,'Beta',expected[1])]:
            content=(staged(peer,task['id'])/'candidate').read_bytes();assert content==text.encode('utf-8')
            request=json.loads((staged(peer,task['id'])/'request.json').read_text(encoding='utf-8'));assert request['spec']['sha256']==hashlib.sha256(content).hexdigest()==task['configuration_targets'][id]['spec']['sha256']
        expect(p.locator('.configuration-diff').first).to_contain_text(default);expect(p.locator('.configuration-diff').last).to_contain_text(beta_value)
        p.locator('.rail-configurations').click();p.get_by_role('button',name='Prepare configuration change…',exact=True).click();p.get_by_role('textbox',name='Candidate configuration (UTF-8)',exact=True).fill('changed-after-preflight={{name}}\n')
        p.locator('.rail-tasks').click();finish(p)
        assert (staged(b,task['id'])/'candidate').read_text(encoding='utf-8')==expected[1]
        saved=app.config.read_text(encoding='utf-8')+app.config.with_suffix('.tasks.json').read_text(encoding='utf-8');assert default not in saved and beta_value not in saved
        passed('Each server receives its own literal candidate, digest and diff; checking freezes values, later editor changes cannot alter the job, and private values stay out of persisted configuration/history')

        app.stop();app.launch();p=app.page;p.locator('.rail-configurations').click();row=p.locator('.configuration-run').first.locator('.configuration-target').filter(has_text='Beta')
        row.get_by_role('button',name='Restore previous recorded version…',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        expect(d.locator('.target-choice.active')).to_contain_text('Beta');expect(d.locator('.configuration-parameter-defaults')).to_have_count(0)
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready'],timeout=20000)
        restore=history(app)[0];request=json.loads((staged(b,restore['id'])/'request.json').read_text(encoding='utf-8'))
        assert request['restore_from']==task['id'] and request['spec']['sha256']==task['configuration_targets']['Beta']['spec']['sha256'];finish(p)
        passed('Restarted history restores Beta with Beta’s original digest and fresh review, without reusing Alpha’s candidate or reopening template parameters')

        editor(p,'{{bad-name}}');p.get_by_role('button',name='Review configuration on servers…',exact=True).click();expect(p.locator('.form-error')).to_contain_text('parameter names')
        editor(p,'{{x}}'*40);p.get_by_role('textbox',name='Default value: x',exact=True).fill('z'*4096);d=review(p);select_all(d)
        before=app.config.with_suffix('.tasks.json').read_bytes();stage_counts=[len(list((peer.root/'home/arcnix/.local/state/arcnix/config-staging').iterdir())) for peer in [a,b]]
        d.get_by_role('button',name='Check targets',exact=True).click();expect(d.locator('.form-error')).to_contain_text('exceeds 128 KiB')
        assert app.config.with_suffix('.tasks.json').read_bytes()==before
        assert [len(list((peer.root/'home/arcnix/.local/state/arcnix/config-staging').iterdir())) for peer in [a,b]]==stage_counts
        d.get_by_role('button',name='Cancel',exact=True).click();passed('Invalid placeholders and oversized rendered files fail locally before staging any target or creating task history')

        editor(p);p.get_by_role('textbox',name='Default value: name',exact=True).fill(default);p.get_by_role('textbox',name='Default value: port',exact=True).fill('8080')
        app.settings('General');p.get_by_role('button',name='Dark',exact=True).click();p.locator('.rail-configurations').click();assert p.locator('.configuration-content').evaluate("e=>getComputedStyle(e).backgroundColor!=='rgb(255, 255, 255)'&&getComputedStyle(e).fontSize==='13px'");app.screenshot('configuration-parameters-dark.png')
        app.settings('General');p.get_by_role('button',name='Light',exact=True).click();p.get_by_role('button',name='简体中文',exact=True).click();p.locator('.rail-configurations').click();p.set_viewport_size(dict(width=900,height=780))
        tip=p.get_by_role('button',name='帮助：配置参数',exact=True);tip.scroll_into_view_if_needed();p.evaluate('() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');p.mouse.move(890,50);tip.hover();expect(p.get_by_role('tooltip')).to_contain_text('只替换一次');tip.click();p.mouse.move(890,770);expect(p.get_by_role('tooltip')).to_be_visible();p.keyboard.press('Escape')
        p.get_by_role('button',name='在服务器上预览配置…',exact=True).click();d=p.get_by_role('dialog',name='选择服务器运行脚本',exact=True)
        d.get_by_role('button',name='选择当前可见的已信任服务器',exact=True).click();beta=d.locator('.script-target-picker').filter(has=p.get_by_role('button',name=re.compile('^Beta ·')));beta.get_by_role('button',name='配置参数值…',exact=True).click()
        expect(beta.get_by_role('textbox',name='此服务器的值：name',exact=True)).to_be_disabled()
        beta.get_by_role('textbox',name='此服务器的值：port',exact=True).scroll_into_view_if_needed()
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert not d.locator('*:visible').evaluate_all("els=>els.filter(el=>!el.children.length&&el.textContent.trim()&&parseFloat(getComputedStyle(el).fontSize)<12).map(el=>el.textContent)")
        button=d.get_by_role('button',name='检查目标',exact=True);assert button.bounding_box()['y']+button.bounding_box()['height']<=780;app.screenshot('configuration-parameters-chinese.png');d.get_by_role('button',name='取消',exact=True).click();passed('Chinese narrow forms and both themes keep readable labels, themed code fields, collapsed per-server controls and hover/click help instead of permanent instructions')

(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8');print(f'{len(checks)} groups passed; artifacts: {out}',flush=True)
