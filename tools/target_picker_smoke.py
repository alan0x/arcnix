"""Owned desktop checks for scoped, searchable and reviewable task target sets."""
import json,re,time
from desktop_fixture import Peer,Desktop,expect,ROOT

out=ROOT/'artifacts/target-picker-ui'
def row(d,name):
    return d.locator('.script-target-picker').filter(has=d.page.get_by_role('button',name=re.compile('^'+re.escape(name)+' ·')))
def search(d,value):d.get_by_role('textbox',name='Search target servers',exact=True).fill(value)
def group(d,name):d.locator('.target-group-filters').get_by_role('button',name=name,exact=True).click()
def run_script(app):
    p=app.page;p.locator('.rail-scripts').click();p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click();p.locator('.script-row').filter(has_text='Target selection fixture').get_by_role('button',name='Run on servers…',exact=True).click()
    d=p.get_by_role('dialog',name='Run script on servers',exact=True);d.get_by_role('textbox',name='message',exact=True).fill("hello '; $(id)");return d

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b,Peer(out/'peer-c') as c:
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None,group_id='prod')
    beta=b.profile('Beta');beta['group_id']='stage'
    delta=c.profile('Delta')
    untrusted=a.profile('Untrusted');untrusted.update(host='127.0.0.3',port=22,group_id='prod')
    alias=a.profile('Alias');alias.update(username='root',group_id='aliases')
    profiles=[alpha,beta,delta,untrusted,alias]
    for n in range(33):
        p=a.profile('Capacity %02d'%n);p.update(host='127.0.0.2',port=23000+n,group_id='capacity');profiles.append(p)
    config=a.config(profiles,out)
    config['config']['groups']=[dict(id='prod',name='Production 生产'),dict(id='stage',name='Staging'),dict(id='aliases',name='Other accounts'),dict(id='capacity',name='Capacity')]
    for peer in [b,c]:
        host,port=peer.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=peer.data['host_key_rsa_fingerprint']))
    for n in range(33):config['config']['trusted_hosts'].append(dict(host='127.0.0.2',port=23000+n,fingerprint=a.data['host_key_rsa_fingerprint']))
    config['config']['snippets']=[dict(id='picker',name='Target selection fixture',description='Explicitly reviewed owned fixture',command="printf '%s\\n' {{message}}",administrator=True)]
    config['config']['backup_policies']=[dict(id='website',name='Website',repository='/backups/site',password_file='/secrets/restic',source='/srv/site',servers=['Alpha','Beta'],administrator=False,excludes=[],keep_last=7)]
    with Desktop(out/f'desktop-{time.time_ns()}',config) as app:
        p=app.page;d=run_script(app);expect(d.locator('.target-choice')).to_have_count(38)
        search(d,'bEtA');expect(d.locator('.target-choice')).to_have_count(1);expect(d.locator('.target-choice')).to_contain_text('Beta')
        search(d,'arcnix-nosftp');expect(d.locator('.target-choice')).to_have_count(1);expect(d.locator('.target-choice')).to_contain_text('Alpha')
        search(d,b.data['address'].rsplit(':',1)[1]);expect(d.locator('.target-choice')).to_have_count(1);expect(d.locator('.target-choice')).to_contain_text('Beta')
        search(d,'生产');expect(d.locator('.target-choice')).to_have_count(2);search(d,'Alpha arcnix-nosftp');expect(d.locator('.target-choice')).to_have_count(1)
        search(d,'no matches');expect(d.locator('.target-choice')).to_have_count(0);expect(d.get_by_role('button',name='Select visible trusted servers',exact=True)).to_be_disabled();expect(d).to_contain_text('No servers match')
        d.get_by_role('button',name='Clear target filters',exact=True).click();group(d,'Production 生产');expect(d.locator('.target-choice')).to_have_count(2)
        expect(row(d,'Untrusted').locator('.target-choice')).to_be_disabled();d.get_by_role('button',name='Select visible trusted servers',exact=True).click();expect(d.locator('.target-choice.active')).to_have_count(1)
        row(d,'Alpha').get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');row(d,'Alpha').get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret')
        group(d,'Staging');expect(d.locator('.target-choice')).to_have_count(1);expect(d.locator('.target-selection-summary')).to_contain_text('Hidden selections: 1');expect(d.get_by_role('button',name='Check targets',exact=True)).to_be_disabled()
        d.get_by_role('button',name='Select visible trusted servers',exact=True).click();expect(d.locator('.target-selection-summary')).to_contain_text('Selected targets: 2 / 32');row(d,'Beta').get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret')
        d.get_by_role('button',name='Review selected servers',exact=True).click();expect(d.locator('.target-choice')).to_have_count(2);expect(d.get_by_role('button',name='Check targets',exact=True)).to_be_enabled()
        expect(row(d,'Alpha').get_by_role('textbox',name='SSH password or key passphrase',exact=True)).to_have_value('arcnix-test');expect(row(d,'Alpha').get_by_role('textbox',name='Administrator password (optional)',exact=True)).to_have_value('fixture-admin-secret');expect(row(d,'Beta').get_by_role('textbox',name='Administrator password (optional)',exact=True)).to_have_value('fixture-admin-secret')
        expect(d.get_by_role('textbox',name='message',exact=True)).to_have_value("hello '; $(id)");assert 'fixture-admin-secret' not in d.inner_text()
        d.get_by_role('button',name='Clear target filters',exact=True).click();group(d,'Ungrouped');expect(d.locator('.target-choice')).to_have_count(1);d.get_by_role('button',name='Select visible trusted servers',exact=True).click();expect(d.locator('.target-selection-summary')).to_contain_text('Selected targets: 3 / 32')
        d.get_by_role('button',name='Clear visible selection',exact=True).click();expect(d.locator('.target-selection-summary')).to_contain_text('Selected targets: 2 / 32');expect(d.locator('.target-choice.active')).to_have_count(0)
        d.get_by_role('button',name='Clear target filters',exact=True).click();group(d,'Other accounts');row(d,'Alias').locator('.target-choice').click();expect(d).to_contain_text('Select one account per server endpoint');expect(d.locator('.target-selection-summary')).to_contain_text('Selected targets: 2 / 32');expect(d.locator('.target-choice.active')).to_have_count(0)
        group(d,'Capacity');d.get_by_role('button',name='Select visible trusted servers',exact=True).click();expect(d).to_contain_text('Select at most 32 servers');expect(d.locator('.target-selection-summary')).to_contain_text('Selected targets: 2 / 32');expect(d.locator('.target-choice.active')).to_have_count(0)
        d.get_by_role('button',name='Review selected servers',exact=True).click();expect(row(d,'Alpha').get_by_role('textbox',name='SSH password or key passphrase',exact=True)).to_have_value('arcnix-test')
        row(d,'Alpha').locator('.target-choice').click();row(d,'Beta').locator('.target-choice').click();expect(d.locator('.target-choice')).to_have_count(0)
        d.get_by_role('button',name='Clear target filters',exact=True).click();search(d,'Alpha');row(d,'Alpha').locator('.target-choice').click();expect(row(d,'Alpha').get_by_role('textbox',name='SSH password or key passphrase',exact=True)).to_have_value('');expect(row(d,'Alpha').get_by_role('textbox',name='Administrator password (optional)',exact=True)).to_have_value('')
        d.get_by_role('button',name='Clear all selections',exact=True).click();expect(d.locator('.target-selection-summary')).to_contain_text('Selected targets: 0 / 32');assert not app.config.with_suffix('.tasks.json').exists()
        d.get_by_role('button',name='Cancel',exact=True).click();d=run_script(app);expect(d.get_by_role('textbox',name='Search target servers',exact=True)).to_have_value('');expect(d.locator('.target-choice.active')).to_have_count(0)
        group(d,'Production 生产');d.get_by_role('button',name='Select visible trusted servers',exact=True).click();row(d,'Alpha').get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');row(d,'Alpha').get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret')
        group(d,'Staging');d.get_by_role('button',name='Select visible trusted servers',exact=True).click();row(d,'Beta').get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret');d.get_by_role('button',name='Review selected servers',exact=True).click();app.screenshot('selected-target-review.png')
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=30000);expect(p.locator('.task-target')).to_have_count(2)
        history=json.loads(app.config.with_suffix('.tasks.json').read_text());assert [t['id'] for t in history[0]['targets']]==['Alpha','Beta']
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Running','Running'],timeout=20000)
        app.stop();time.sleep(4.5);app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click();expect(p.locator('.task-status')).to_have_text(['Result unknown','Result unknown'])
        for index in range(2):
            p.locator('.task-target').nth(index).get_by_role('button',name='Query server result',exact=True).click();q=p.get_by_role('dialog',name='Query server result',exact=True);q.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');q.get_by_role('button',name='Query',exact=True).click();expect(p.locator('.task-status').nth(index)).to_have_text('Succeeded',timeout=20000)
        before=app.config.with_suffix('.tasks.json').read_text();assert 'arcnix-test' not in before and 'fixture-admin-secret' not in before and 'target_filter' not in before
        p.locator('.rail-backups').click();p.get_by_role('button',name='Back up now',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        expect(d.locator('.target-choice')).to_have_count(2);expect(d.locator('.target-choice.active')).to_have_count(2);expect(d.locator('.target-group-filters').get_by_role('button',name='Capacity',exact=True)).to_have_count(0)
        group(d,'Production 生产');expect(d.locator('.target-choice')).to_have_count(1);expect(d.get_by_role('button',name='Check targets',exact=True)).to_be_disabled();d.get_by_role('button',name='Clear all selections',exact=True).click();d.get_by_role('button',name='Select visible trusted servers',exact=True).click();d.get_by_role('button',name='Review selected servers',exact=True).click();expect(d.locator('.target-choice')).to_have_count(1);expect(d.locator('.target-choice')).to_contain_text('Alpha');d.get_by_role('button',name='Cancel',exact=True).click();assert app.config.with_suffix('.tasks.json').read_text()==before
        d=run_script(app);search(d,'Alpha');row(d,'Alpha').locator('.target-choice').click();row(d,'Alpha').get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');row(d,'Alpha').get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret');d.get_by_role('button',name='Cancel',exact=True).click()
        app.settings('General');p.get_by_role('button',name='简体中文',exact=True).click();expect(p.locator('.page-header-title')).to_have_text('常规');p.locator('.rail-scripts').click();p.locator('.context-links').get_by_role('button',name='我的脚本',exact=True).click();p.locator('.script-row').filter(has_text='Target selection fixture').get_by_role('button',name='在服务器上运行…',exact=True).click();d=p.get_by_role('dialog',name='选择服务器运行脚本',exact=True)
        d.get_by_role('textbox',name='搜索目标服务器',exact=True).fill('生产');expect(d.locator('.target-choice')).to_have_count(2);d.get_by_role('button',name='选择当前可见的已信任服务器',exact=True).click();expect(d.get_by_role('textbox',name='SSH 密码或私钥口令',exact=True)).to_have_value('');expect(d.locator('.target-selection-summary')).to_contain_text('已选目标: 1 / 32')
        p.set_viewport_size(dict(width=900,height=700));assert p.evaluate('document.documentElement.scrollWidth <= innerWidth');app.screenshot('target-picker-chinese.png')
        assert 'fixture-admin-secret' not in app.config.read_text() and 'target_filter' not in app.config.read_text()

checks=['global target picker','case-insensitive name search','account search','port search','group name and Unicode search','multi-term search','empty-result state','untrusted targets remain disabled','visible additive selection','hidden selected count','hidden selection blocks preflight','review all selected targets','credentials survive filter remount','parameters survive filtering','ungrouped filter','clear visible keeps hidden selections','duplicate endpoint rejected atomically','over-32 bulk selection rejected atomically','deselection discards both credentials','clear all and cancel create no task','new operation resets filters','only reviewed targets enter server preflight','server execution survives desktop restart','backup scope cannot be expanded by filtering','cancel and later dialog clear credentials','Chinese narrow layout','credentials and filters are not persisted']
(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),indent=2));print(f'{len(checks)} target picker UI checks passed')
