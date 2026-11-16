"""Local server classification, scoped exact filters and connection-preserving edits."""
import hashlib,json,time
from pathlib import Path
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/server-labels-ui'/str(time.time_ns());checks=[]
def passed(label):checks.append(label);print(label,flush=True)
def saved(app):return eventually(lambda:json.loads(app.config.read_text())['config'])
def history(app):return eventually(lambda:json.loads(app.config.with_suffix('.tasks.json').read_text()))
def edit(p,name,zh=False):
    p.locator('.rail-servers').click();row=p.locator('.host-row').filter(has=p.get_by_text(name,exact=True));row.hover();row.locator('.host-more').click()
    row.locator('.context-popover').get_by_text('编辑服务器' if zh else 'Edit server',exact=True).click();d=p.get_by_role('dialog',name='编辑服务器' if zh else 'Edit server',exact=True)
    d.get_by_role('button',name='分类标签' if zh else 'Labels',exact=True).click();return d
def run(p):
    p.locator('.rail-scripts').click();p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click()
    p.locator('.script-row').filter(has_text='Labels fixture').get_by_role('button',name='Run on servers…',exact=True).click()
    return p.get_by_role('dialog',name='Run script on servers',exact=True)
def row(d,name):return d.locator('.script-target-picker').filter(has=d.page.get_by_role('button',name=__import__('re').compile('^'+name+' · ')))
def filter(d,environment='',tag=''):
    d.get_by_role('textbox',name='Environment filter',exact=True).fill(environment);d.get_by_role('textbox',name='Tag filter',exact=True).fill(tag)
def auth(peer):return Path(peer.data['auth_audit']).read_bytes()
def help_text(p,tip,text):
    tip.scroll_into_view_if_needed();p.evaluate('() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))')
    p.mouse.move(1430,970);tip.hover();expect(p.get_by_role('tooltip')).to_contain_text(text)
    tip.click();p.mouse.move(1430,970);expect(p.get_by_role('tooltip')).to_contain_text(text);p.keyboard.press('Escape')
def folder(p):expect(p.locator('.remote-slot').get_by_role('button',name='/folder',exact=True)).to_be_visible()
def launches(peer):
    f=peer.root/'task-launches.jsonl';return f.read_bytes() if f.exists() else b''

with Peer(out/'alpha') as a,Peer(out/'beta',io_delay=.025) as b,Peer(out/'gamma') as c:
    local=out/'local';local.mkdir(parents=True)
    content=bytes(range(256))*65536;(local/'active.bin').write_bytes(content)
    alpha=a.profile('Alpha',group_id='east',environment='Production',tags=['Web','中文','服务器']);alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    beta=b.profile('Beta',group_id='west',locations=dict(remember_paths=True))
    gamma=c.profile('Gamma',group_id='east',environment='Production',tags=['WebApp'])
    untrusted=a.profile('Untrusted',group_id='east',environment='Production',tags=['Web']);untrusted.update(host='127.0.0.3',port=22)
    tag_only=a.profile('Tag only',tags=['服务器']);tag_only.update(host='127.0.0.4',port=22)
    config=a.config([alpha,beta,gamma,untrusted,tag_only],local);config['config']['groups']=[dict(id='east',name='East 亚洲'),dict(id='west',name='West')]
    for peer in [b,c]:
        host,port=peer.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=peer.data['host_key_rsa_fingerprint']))
    config['config']['snippets']=[dict(id='labels',name='Labels fixture',description='',command="printf '%s\\n' 'label fixture'",administrator=True)]
    config['config']['backup_policies']=[dict(id='website',name='Website',repository='/backups/site',password_file='/secrets/restic',source='/srv/site',servers=['Alpha','Beta'],administrator=False,excludes=[],keep_last=7)]
    with Desktop(out/'desktop',config) as app:
        p=app.page;p.locator('.host-row').filter(has=p.get_by_text('Beta',exact=True)).locator('.host-select').click();p.locator('.overview-connect').click();p.get_by_role('dialog',name='Connect to server',exact=True).get_by_role('button',name='Connect',exact=True).click();expect(p.locator('.connection-state')).to_have_text('SSH connected')
        app.file('folder').dblclick();folder(p)
        eventually(lambda:saved(app)['servers'][1].get('locations',{}).get('last_remote')=='/folder')
        baseline=saved(app);before_auth=auth(b);d=edit(p,'Beta')
        expect(d.get_by_role('textbox',name='Environment (optional)',exact=True)).to_have_value('');expect(d.get_by_role('textbox',name='Tags (comma-separated)',exact=True)).to_have_value('')
        d.get_by_role('textbox',name='Environment (optional)',exact=True).fill('x'*33);d.get_by_role('button',name='Save server',exact=True).click();expect(d.locator('.form-error')).to_contain_text('at most 32')
        assert saved(app)==baseline
        d.get_by_role('textbox',name='Environment (optional)',exact=True).fill(' Staging ');d.get_by_role('textbox',name='Tags (comma-separated)',exact=True).fill('web, WEB, db，中文')
        d.get_by_role('button',name='Save server',exact=True).click();expect(d).not_to_be_visible()
        profile=saved(app)['servers'][1];assert profile['environment']=='Staging' and profile['tags']==['web','db','中文']
        expect(p.locator('.connection-state')).to_have_text('SSH connected');folder(p);assert auth(b)==before_auth
        d=edit(p,'Beta');d.get_by_role('textbox',name='Tags (comma-separated)',exact=True).fill('discarded');d.get_by_role('button',name='Cancel',exact=True).click();assert saved(app)['servers'][1]['tags']==profile['tags']
        passed('Legacy profiles start without labels; validation is local, CSV tags trim and deduplicate, cancellation preserves stored labels, and metadata-only saves keep the SSH session and remembered directory')

        app.file('active.bin',False).click();p.locator('.local-slot').get_by_role('button',name='Upload',exact=True).click()
        eventually(lambda:0<float(p.locator('.queue-row').last.get_by_role('progressbar').get_attribute('aria-valuenow'))<100)
        before_auth=auth(b)
        d=edit(p,'Beta');d.get_by_role('textbox',name='Tags (comma-separated)',exact=True).fill('web,db,中文,active')
        assert 0<float(p.locator('.queue-row').last.get_by_role('progressbar',include_hidden=True).get_attribute('aria-valuenow'))<100
        d.get_by_role('button',name='Save server',exact=True).click();expect(d).not_to_be_visible()
        expect(p.locator('.queue-row')).to_have_count(1);expect(p.locator('.queue-row').last.locator('.job-status')).to_have_text('Completed',timeout=30000)
        assert hashlib.sha256((b.root/'folder/active.bin').read_bytes()).digest()==hashlib.sha256(content).digest()
        assert auth(b)==before_auth;expect(p.locator('.connection-state')).to_have_text('SSH connected');folder(p)
        passed('Changing labels during an actual throttled 16 MiB SFTP upload preserves the running job, connection, destination and exact transferred bytes without reauthentication')

        search=p.get_by_role('textbox',name='Search',exact=True)
        search.fill('production web');expect(p.locator('.host-name')).to_have_text(['Alpha','Gamma','Untrusted'])
        search.fill('亚洲 中文');expect(p.locator('.host-name')).to_have_text(['Alpha']);expect(p.locator('.tree-no-results')).to_have_count(0)
        search.fill('staging ACTIVE');expect(p.locator('.host-name')).to_have_text(['Beta'])
        search.fill('staging absent');expect(p.locator('.host-name')).to_have_count(0);expect(p.locator('.tree-no-results')).to_be_visible();search.fill('')
        passed('Server-tree search combines words across names, groups, environments and tags, retaining Unicode and case-insensitive matching with accurate empty results')

        d=run(p)
        help_text(p,row(d,'Tag only').get_by_role('button',name='Help: Server labels',exact=True),'服务器')
        filter(d,' production ','WEB');expect(d.locator('.target-choice')).to_have_count(2)
        expect(row(d,'Untrusted').locator('.target-choice')).to_be_disabled()
        d.get_by_role('button',name='Select visible trusted servers',exact=True).click();expect(d.locator('.target-choice.active')).to_have_count(1)
        help_text(p,row(d,'Alpha').get_by_role('button',name='Help: Server labels',exact=True),'Production · Web · 中文 · 服务器')
        row(d,'Alpha').get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');row(d,'Alpha').get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret')
        filter(d,'Staging','web');expect(d.locator('.target-choice')).to_have_count(1);expect(d.get_by_role('button',name='Check targets',exact=True)).to_be_disabled()
        expect(d.locator('.target-selection-summary')).to_contain_text('Hidden selections: 1')
        d.get_by_role('button',name='Select visible trusted servers',exact=True).click();row(d,'Beta').get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret')
        d.get_by_role('button',name='Review selected servers',exact=True).click();expect(d.locator('.target-choice')).to_have_count(2)
        expect(d.get_by_role('textbox',name='Environment filter',exact=True)).to_have_value('');expect(d.get_by_role('textbox',name='Tag filter',exact=True)).to_have_value('')
        expect(row(d,'Alpha').get_by_role('textbox',name='SSH password or key passphrase',exact=True)).to_have_value('arcnix-test')
        d.get_by_role('textbox',name='Environment filter',exact=True).focus();p.keyboard.press('Tab');expect(d.get_by_role('textbox',name='Tag filter',exact=True)).to_be_focused()
        p.set_viewport_size(dict(width=900,height=780));assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        button=d.get_by_role('button',name='Check targets',exact=True);assert button.bounding_box()['y']+button.bounding_box()['height']<=780
        app.screenshot('label-filters-narrow.png');p.set_viewport_size(dict(width=1440,height=980))
        assert not launches(a) and not launches(b) and not launches(c)
        passed('Exact environment and tag filters combine without substring matches or implicit trust; hidden selections block preflight and review restores all selected targets and their independent credentials')

        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=30000)
        task=history(app)[0];assert [t['id'] for t in task['targets']]==['Alpha','Beta']
        d=edit(p,'Gamma');d.get_by_role('textbox',name='Tags (comma-separated)',exact=True).fill('Web');d.get_by_role('button',name='Save server',exact=True).click();expect(d).not_to_be_visible()
        p.locator('.rail-tasks').click();p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Succeeded','Succeeded'],timeout=30000)
        assert [t['id'] for t in history(app)[0]['targets']]==['Alpha','Beta'] and not launches(c)
        passed('Preflight freezes the reviewed server IDs; later label edits making Gamma match the original filter cannot add Gamma to the accepted batch')

        before=app.config.with_suffix('.tasks.json').read_bytes();p.locator('.rail-backups').click();p.get_by_role('button',name='Back up now',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        filter(d,'Production','web');expect(d.locator('.target-choice')).to_have_count(1);expect(d.locator('.target-choice')).to_contain_text('Alpha')
        expect(d.get_by_role('button',name='Check targets',exact=True)).to_be_disabled();d.get_by_role('button',name='Review selected servers',exact=True).click();expect(d.locator('.target-choice')).to_have_count(2)
        expect(d.get_by_role('textbox',name='Environment filter',exact=True)).to_have_value('');d.get_by_role('button',name='Cancel',exact=True).click();assert app.config.with_suffix('.tasks.json').read_bytes()==before
        passed('Backup actions reuse label filters while preserving their policy allowlist; matching labels on Gamma cannot expand the backup scope')

        p.locator('.rail-configurations').click();p.get_by_role('button',name='Compare current files',exact=True).click();p.get_by_role('button',name='Read configuration snapshots…',exact=True).click();d=p.get_by_role('dialog',name='Read configuration snapshots',exact=True)
        filter(d,'Staging','web');d.get_by_role('button',name='Select visible trusted servers',exact=True).click()
        filter(d,'Production','中文');d.get_by_role('button',name='Select visible trusted servers',exact=True).click();d.get_by_role('button',name='Read snapshots',exact=True).click();expect(d.locator('.form-error')).to_contain_text('Review hidden selected servers')
        assert not (a.root/'configuration-compare-queries.jsonl').exists() and not (b.root/'configuration-compare-queries.jsonl').exists()
        d.get_by_role('button',name='View selected servers',exact=True).click();expect(d.locator('.comparison-server-choice')).to_have_count(2)
        expect(d.get_by_role('textbox',name='Environment filter',exact=True)).to_have_value('');expect(d.get_by_role('textbox',name='Tag filter',exact=True)).to_have_value('')
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test');d.get_by_role('button',name='Read snapshots',exact=True).click()
        expect(p.get_by_role('button',name='Read configuration snapshots…',exact=True)).to_be_enabled(timeout=15000);expect(p.locator('.configuration-comparison-server')).to_have_count(2)
        assert app.config.with_suffix('.tasks.json').read_bytes()==before
        passed('Current configuration comparison uses the same exact label filters; reviewing selected servers resets every facet and reads only the two reviewed targets')

        app.stop();app.launch();p=app.page;d=edit(p,'Beta')
        expect(d.get_by_role('textbox',name='Tags (comma-separated)',exact=True)).to_have_value('web, db, 中文, active')
        d.get_by_role('textbox',name='Environment (optional)',exact=True).fill('');d.get_by_role('textbox',name='Tags (comma-separated)',exact=True).fill('');d.get_by_role('button',name='Save server',exact=True).click();expect(d).not_to_be_visible()
        assert 'environment' not in saved(app)['servers'][1] and 'tags' not in saved(app)['servers'][1]
        app.settings('General');p.get_by_role('button',name='Dark',exact=True).click();p.get_by_role('button',name='简体中文',exact=True).click();p.set_viewport_size(dict(width=900,height=700));d=edit(p,'Beta',True)
        d.get_by_role('textbox',name='环境（可选）',exact=True).fill('测试环境');d.get_by_role('textbox',name='标签（逗号分隔）',exact=True).fill('数据库,华东')
        tip=d.get_by_role('button',name='帮助：服务器分类',exact=True);tip.hover();expect(p.get_by_role('tooltip')).to_contain_text('本地备注');tip.click();p.mouse.move(890,690);expect(p.get_by_role('tooltip')).to_be_visible();p.keyboard.press('Escape')
        assert not d.locator('*:visible').evaluate_all("els=>els.filter(el=>!el.children.length&&el.textContent.trim()&&parseFloat(getComputedStyle(el).fontSize)<12).map(el=>el.textContent)")
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth');button=d.get_by_role('button',name='保存服务器',exact=True);assert button.bounding_box()['y']+button.bounding_box()['height']<=700
        app.screenshot('labels-chinese-dark.png');d.get_by_role('button',name='取消',exact=True).click()
        assert 'arcnix-test' not in app.config.read_text() and 'fixture-admin-secret' not in app.config.read_text()
        passed('Labels survive restart and clear back to legacy-compatible fields; Chinese 900 px editor keeps hover/click help, keyboard controls, consistent type and a visible footer')

(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8');print(f'{len(checks)} groups passed; artifacts: {out}',flush=True)
