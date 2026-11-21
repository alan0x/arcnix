"""Reviewed script-only exchange, sharing copies and normal multi-host execution."""
import json,time
from pathlib import Path
from desktop_fixture import Peer,Desktop,expect,ROOT

out=ROOT/'artifacts/script-bundles-ui';out.mkdir(parents=True,exist_ok=True);checks=[]
def passed(name):checks.append(name)
def script(name,command,admin=False,description='Portable description'):
    return dict(name=name,description=description,command=command,administrator=admin)
def bundle(path,scripts,**fields):
    path.write_text(json.dumps(dict(kind='arcnix-scripts',version=1,scripts=scripts,**fields),ensure_ascii=False),encoding='utf-8')
def library(app):
    p=app.page;p.locator('.rail-scripts').click()
    p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click();return p
def importer(app,path):
    p=app.page;p.get_by_role('button',name='Import scripts…',exact=True).click()
    d=p.get_by_role('dialog',name='Import script bundle',exact=True)
    d.get_by_role('textbox',name='Script bundle path',exact=True).fill(str(path))
    d.get_by_role('button',name='Read bundle',exact=True).click();return d
def review(d):
    d.get_by_role('button',name='Review import…',exact=True).click()
    expect(d.locator('.script-bundle-import-review')).to_be_visible();return d
def apply(d):
    d.get_by_role('button',name='Apply import',exact=True).click()
    expect(d).not_to_be_visible()
def rows(app):return json.loads(app.config.read_text())['config']['snippets']

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    config=a.config([a.profile('Alpha'),b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1)
    config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    original=[dict(id='existing',**script('Existing','echo old')),
        dict(id='private',**script('Private original','echo hardcoded-demo-token',True,'description-demo-token'))]
    config['config']['snippets']=original
    source=out/'incoming.json';bundle(source,[script('Existing','echo replacement'),script('Install Nginx',"printf '%s\\n' {{message}}")])
    with Desktop(out/f'desktop-{time.time_ns()}',config) as app:
        p=app.page;p.locator('.rail-scripts').click()
        assert 'rail-servers' in p.locator('.navigation-rail button').first.get_attribute('class')
        expect(p.get_by_role('button',name='Import scripts…',exact=True)).to_be_visible()
        expect(p.get_by_role('button',name='Export scripts…',exact=True)).to_have_count(0)
        library(app);initial=app.config.read_text();history=app.config.with_suffix('.tasks.json')
        passed('global exchange requires no active server and export is offered for user scripts')
        for number,contents,error in [
            (0,dict(kind='arcnix-scripts',version=1,scripts=[dict(script('Forged','true'),installation={})]),'unknown field'),
            (1,dict(kind='arcnix-scripts',version=1,scripts=[dict(script('Forged','true'),id='builtin:nginx')]),'unknown field'),
            (2,dict(kind='arcnix-scripts',version=99,scripts=[script('Wrong version','true')]),'Unsupported script bundle'),
            (3,dict(kind='arcnix-scripts',version=1,scripts=[script('A','true'),script(' a ','false')]),'distinct ignoring case'),
            (4,dict(kind='arcnix-scripts',version=1,scripts=[script('Bad parameter',"echo '{{message}}'")]),'unquoted shell words'),
        ]:
            path=out/f'invalid-{number}.json';path.write_text(json.dumps(contents),encoding='utf-8');d=importer(app,path)
            expect(d).to_contain_text(error,timeout=10000);expect(d.locator('.script-bundle-entry')).to_have_count(0)
            expect(d.get_by_role('button',name='Review import…',exact=True)).to_be_disabled()
            d.get_by_role('button',name='Cancel',exact=True).click()
            assert app.config.read_text()==initial and not history.exists()
        passed('invalid schema authority fields versions duplicate names and parameter syntax leave configuration untouched')
        d=importer(app,source);expect(d.locator('.script-bundle-entry')).to_have_count(2)
        expect(d.locator('.script-bundle-selection')).to_contain_text('2')
        d.get_by_role('button',name='Clear script selection',exact=True).click();expect(d.get_by_role('button',name='Review import…',exact=True)).to_be_disabled()
        d.get_by_role('button',name='Select new scripts',exact=True).click();review(d)
        expect(d.locator('.script-bundle-import-review')).to_contain_text('Existing (imported)')
        expect(d.locator('.script-bundle-import-review')).to_contain_text('New copy')
        assert app.config.read_text()==initial and not history.exists()
        app.screenshot('copy-import-review.png');d.get_by_role('button',name='Cancel',exact=True).click()
        assert rows(app)==original;passed('copy preview and cancellation do not save or run scripts')
        d=importer(app,source);review(d);apply(d)
        expect(p.locator('.script-row')).to_have_count(4)
        saved=rows(app);assert next(v for v in saved if v['id']=='existing')['command']=='echo old'
        copied=next(v for v in saved if v['name']=='Existing (imported)');assert copied['id']!='existing'
        imported=next(v for v in saved if v['name']=='Install Nginx');assert not imported['id'].startswith('builtin:')
        assert not history.exists();passed('applied copies get fresh IDs while preserving existing definitions')
        d=importer(app,source)
        entry=d.locator('.script-bundle-entry').filter(has=p.get_by_role('button',name='Install Nginx',exact=True))
        expect(entry.get_by_role('button',name='Install Nginx',exact=True)).to_be_disabled();expect(entry).to_contain_text('Identical script already exists')
        d.get_by_role('button',name='Replace scripts with matching names',exact=True).click();review(d)
        expect(d.locator('.script-bundle-import-review')).to_contain_text('Existing definition:')
        expect(d.locator('.script-bundle-import-review')).to_contain_text('echo old')
        expect(d.locator('.script-bundle-import-review')).to_contain_text('echo replacement')
        apply(d)
        saved=rows(app);assert len(saved)==4 and next(v for v in saved if v['id']=='existing')['command']=='echo replacement'
        passed('identical entries are skipped and explicit replacement preserves the original ID')
        frozen=out/'frozen.json';bundle(frozen,[script('Frozen import','echo reviewed')])
        d=importer(app,frozen);review(d);bundle(frozen,[script('Frozen import','echo changed-after-review')])
        apply(d)
        assert next(v for v in rows(app) if v['name']=='Frozen import')['command']=='echo reviewed'
        passed('source file changes cannot alter an already reviewed import')
        before_export=rows(app)
        p.get_by_role('button',name='Export scripts…',exact=True).click();d=p.get_by_role('dialog',name='Export script bundle',exact=True)
        expect(d.locator('.script-share-editor')).to_have_count(0)
        d.locator('.script-share-item').filter(has=p.get_by_role('button',name='Private original',exact=True)).get_by_role('button',name='Private original',exact=True).click()
        d.get_by_role('textbox',name='Shared script name',exact=True).fill('Shared safe copy')
        d.get_by_role('textbox',name='Shared script description',exact=True).fill('Reviewed sharing description')
        d.get_by_role('textbox',name='Shared script command',exact=True).fill("printf '%s\\n' {{message}}")
        d.get_by_role('button',name='Run as administrator',exact=True).click()
        d.get_by_role('textbox',name='Export folder',exact=True).fill(str(out))
        d.get_by_role('textbox',name='Bundle file name',exact=True).fill('../outside.json')
        d.get_by_role('button',name='Review export…',exact=True).click();expect(d).to_contain_text('plain bundle file name')
        target=out/'shared-safe.json'
        if target.exists():target.unlink()
        d.get_by_role('textbox',name='Bundle file name',exact=True).fill(target.name)
        d.get_by_role('button',name='Review export…',exact=True).click()
        expect(d.locator('.script-bundle-export-review')).to_contain_text('Shared safe copy')
        expect(d.locator('.script-bundle-export-review')).not_to_contain_text('hardcoded-demo-token')
        assert not target.exists() and rows(app)==before_export
        app.screenshot('sharing-copy-review.png');d.get_by_role('button',name='Write new bundle',exact=True).click();expect(d).not_to_be_visible()
        payload=json.loads(target.read_text(encoding='utf-8'));assert payload['scripts']==[script('Shared safe copy',"printf '%s\\n' {{message}}",False,'Reviewed sharing description')]
        assert 'id' not in payload['scripts'][0] and 'servers' not in payload
        assert 'hardcoded-demo-token' not in target.read_text() and 'description-demo-token' not in target.read_text()
        assert rows(app)==before_export and next(v for v in rows(app) if v['id']=='private')['command']=='echo hardcoded-demo-token'
        passed('sharing copy edits remove chosen secrets from export without changing the original library')
        p.get_by_role('button',name='Export scripts…',exact=True).click();d=p.get_by_role('dialog',name='Export script bundle',exact=True)
        d.get_by_role('button',name='Select all scripts',exact=True).click()
        d.get_by_role('textbox',name='Export folder',exact=True).fill(str(out));d.get_by_role('textbox',name='Bundle file name',exact=True).fill(target.name)
        d.get_by_role('button',name='Review export…',exact=True).click();expect(d).to_contain_text('Export destination already exists')
        d.get_by_role('textbox',name='Bundle file name',exact=True).fill('race.json')
        race=out/'race.json'
        if race.exists():race.unlink()
        d.get_by_role('button',name='Review export…',exact=True).click();race.write_text('another writer')
        d.get_by_role('button',name='Write new bundle',exact=True).click();expect(d.locator('.form-error')).to_be_visible()
        assert race.read_text()=='another writer'
        d.get_by_role('button',name='Cancel',exact=True).click();passed('existing destinations and a writer arriving after review cannot be overwritten')
        d=importer(app,target);review(d);apply(d)
        p.locator('.script-row').filter(has=p.get_by_text('Shared safe copy',exact=True)).get_by_role('button',name='Run on servers…',exact=True).click()
        d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        expect(d.locator('.installation-scope')).to_have_text('')
        d.get_by_role('textbox',name='message',exact=True).fill("portable '; $(id) {{message}}")
        for index in range(2):d.locator('.target-choice').nth(index).click()
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=30000)
        expect(p.locator('.task-target').first).to_contain_text("portable '\\''; $(id) {{message}}")
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Running','Running'],timeout=20000)
        app.stop();time.sleep(4.5);app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        for index in range(2):
            p.locator('.task-target').nth(index).get_by_role('button',name='Query server result',exact=True).click()
            p.get_by_role('dialog',name='Query server result',exact=True).get_by_role('button',name='Query',exact=True).click()
            expect(p.locator('.task-status').nth(index)).to_have_text('Succeeded',timeout=20000)
        task=json.loads(history.read_text())[0];assert task['installation'] is None and task['drill'] is None
        passed('imported script uses literal parameters and normal server-owned batch execution after restart')
        library(app);p.get_by_role('button',name='Export scripts…',exact=True).click();d=p.get_by_role('dialog',name='Export script bundle',exact=True)
        expect(d.get_by_role('textbox',name='Export folder',exact=True)).to_have_value('')
        expect(d.locator('.script-share-editor')).to_have_count(0);d.get_by_role('button',name='Cancel',exact=True).click()
        p.get_by_role('button',name='Import scripts…',exact=True).click();d=p.get_by_role('dialog',name='Import script bundle',exact=True)
        expect(d.get_by_role('textbox',name='Script bundle path',exact=True)).to_have_value('');d.get_by_role('button',name='Cancel',exact=True).click()
        passed('closing exchange dialogs discards source paths selected scripts and sharing drafts')
        app.settings('General');p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check();expect(p.locator('.page-header-title')).to_have_text('常规')
        p.locator('.rail-scripts').click();p.set_viewport_size(dict(width=900,height=700))
        p.get_by_role('button',name='导出脚本…',exact=True).click();d=p.get_by_role('dialog',name='导出脚本包',exact=True)
        expect(d).to_contain_text('分享副本');assert p.evaluate('document.documentElement.scrollWidth <= innerWidth')
        box=d.get_by_role('button',name='取消',exact=True).bounding_box();assert box and box['y']+box['height']<=700
        app.screenshot('script-bundle-chinese-narrow.png');passed('Chinese bundle controls and narrow dialog footer remain usable')
        stored=json.loads(app.config.read_text())['config']
        # Saving normalizes omitted optional profile fields to model defaults.
        assert len(stored['servers'])==len(config['config']['servers'])
        for original_profile in config['config']['servers']:
            actual=next(v for v in stored['servers'] if v['id']==original_profile['id'])
            assert all(actual[key]==value for key,value in original_profile.items())
        assert stored['trusted_hosts']==config['config']['trusted_hosts']
        assert 'portable ' not in app.config.read_text() and 'fixture-admin-secret' not in app.config.read_text()
        passed('exchange preserves server profiles trust and keeps run-time parameter values out of configuration')
    full=json.loads(json.dumps(config));full['config']['snippets']=[dict(id=f'full-{i}',**script(f'Script {i}','true')) for i in range(200)]
    extra=out/'extra.json';bundle(extra,[script('One extra','true')])
    with Desktop(out/f'capacity-{time.time_ns()}',full) as app:
        library(app);before=app.config.read_text();d=importer(app,extra)
        d.get_by_role('button',name='Review import…',exact=True).click();expect(d).to_contain_text('at most 200 scripts')
        assert app.config.read_text()==before and not app.config.with_suffix('.tasks.json').exists()
        passed('capacity failure is rejected before review commit without partial import')
(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),ensure_ascii=False,indent=2),encoding='utf-8')
print(f'{len(checks)} script bundle desktop groups passed')
