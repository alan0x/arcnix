"""Typed literal parameters, opted-in local presets and portable v2 definitions."""
import json,time,re
from desktop_fixture import Peer,Desktop,expect,ROOT,eventually

out=ROOT/'artifacts/script-parameters-ui';out.mkdir(parents=True,exist_ok=True);checks=[]
def passed(label):checks.append(label)
def parameter(name,kind='text',default='',allow=False,required=True,**extra):
    return dict(name=name,kind=kind,default_value=default,allow_preset=allow,required=required,**extra)
def saved(app):return json.loads(app.config.read_text(encoding='utf-8'))['config']
def presets(app):return saved(app).get('script_presets',[])
def mine(app):
    p=app.page;p.locator('.rail-scripts').click();p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click();return p
def open_run(app,name='Typed task'):
    p=mine(app);p.locator('.script-row').filter(has=p.get_by_text(name,exact=True)).get_by_role('button',name='Run on servers…',exact=True).click()
    return p.get_by_role('dialog',name='Run script on servers',exact=True)
def cancel(d):d.get_by_role('button',name='Cancel',exact=True).click();expect(d).not_to_be_visible()
def select_preset(d,name):d.locator('.parameter-presets').get_by_role('button',name=name,exact=True).click()

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    config=a.config([a.profile('Alpha'),b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    definitions=[parameter('directory','path','/srv/default',True),parameter('enabled','boolean','true',True),parameter('mode','choice','staging',True,choices=['staging','production']),parameter('note',required=False),parameter('port','port','8080',True),parameter('retries','integer','3',True,minimum=1,maximum=5)]
    original=dict(id='typed',name='Typed task',description='Typed parameters demo',command="printf '%s\\n' {{directory}} {{enabled}} {{mode}} {{note}} {{port}} {{retries}}",administrator=False,parameters=definitions)
    legacy=dict(id='legacy',name='Legacy task',description='',command='echo {{message}}',administrator=False)
    config['config']['snippets']=[original,legacy]
    with Desktop(out/f'desktop-{time.time_ns()}',config) as app:
        p=app.page;d=open_run(app)
        expect(d.get_by_role('textbox',name='port',exact=True)).to_have_value('8080');expect(d.get_by_role('textbox',name='retries',exact=True)).to_have_value('3')
        expect(d.get_by_role('button',name='mode: staging',exact=True)).to_have_class(re.compile(r'\bchoice active\b'))
        expect(d.get_by_role('button',name='enabled: true',exact=True)).to_have_class(re.compile(r'\bchoice active\b'))
        d.locator('.target-choice').first.click();d.get_by_role('textbox',name='port',exact=True).fill('65536');d.get_by_role('button',name='Check targets',exact=True).click();expect(d.locator('.form-error')).to_contain_text('65535')
        d.get_by_role('textbox',name='port',exact=True).fill('8080');d.get_by_role('textbox',name='retries',exact=True).fill('6');d.get_by_role('button',name='Check targets',exact=True).click();expect(d.locator('.form-error')).to_contain_text('bounds')
        d.get_by_role('textbox',name='retries',exact=True).fill('3');d.get_by_role('textbox',name='directory',exact=True).fill('../srv');d.get_by_role('button',name='Check targets',exact=True).click();expect(d.locator('.form-error')).to_contain_text('absolute')
        passed('typed defaults, choice and boolean controls; invalid ports, bounds and relative paths block preflight')
        d.get_by_role('textbox',name='directory',exact=True).fill("/srv/a' ; $(id) {{note}}")
        d.get_by_role('button',name='mode: production',exact=True).click();d.get_by_role('button',name='enabled: false',exact=True).click()
        d.get_by_role('textbox',name='note',exact=True).fill('SESSION-ONLY-NOTE')
        d.get_by_role('textbox',name='Preset name',exact=True).fill('Production');d.get_by_role('button',name='Save new preset',exact=True).click()
        eventually(lambda:len(presets(app))==1);stored=presets(app)[0];assert stored['source']=='user' and stored['script']=='typed' and 'note' not in stored['values']
        assert 'SESSION-ONLY-NOTE' not in app.config.read_text()
        select_preset(d,'Production');d.get_by_role('textbox',name='port',exact=True).fill('9000');d.get_by_role('button',name='Load selected preset',exact=True).click()
        expect(d.get_by_role('textbox',name='port',exact=True)).to_have_value('8080');expect(d.get_by_role('textbox',name='note',exact=True)).to_have_value('')
        d.get_by_role('textbox',name='port',exact=True).fill('8090');d.get_by_role('button',name='Update selected preset',exact=True).click();eventually(lambda:presets(app)[0]['values']['port']=='8090')
        before=app.config.read_text();d.get_by_role('button',name='Save new preset',exact=True).click();expect(d.locator('.form-error')).to_contain_text('Duplicate');assert app.config.read_text()==before
        d.get_by_role('button',name='Reset parameter defaults',exact=True).click();expect(d.get_by_role('textbox',name='port',exact=True)).to_have_value('8080')
        d.get_by_role('button',name='Load selected preset',exact=True).click();expect(d.get_by_role('textbox',name='port',exact=True)).to_have_value('8090');app.screenshot('typed-runtime-presets.png');cancel(d)
        passed('only opted-in fields persist; load resets other values, explicit update works and duplicate names preserve the config')
        app.stop();app.launch();p=app.page;d=open_run(app);expect(d.get_by_role('textbox',name='port',exact=True)).to_have_value('8080');select_preset(d,'Production');d.get_by_role('button',name='Load selected preset',exact=True).click();expect(d.get_by_role('textbox',name='port',exact=True)).to_have_value('8090');cancel(d)
        passed('local presets survive a client restart without being applied automatically')
        p=mine(app);p.locator('.script-row').filter(has=p.get_by_text('Typed task',exact=True)).get_by_role('button',name='Edit',exact=True).click()
        e=p.get_by_role('dialog',name='Save command',exact=True)
        retries=e.locator('.parameter-definition').filter(has=p.get_by_text('retries',exact=True))
        retries.get_by_role('textbox',name='Minimum integer: retries',exact=True).fill('bad');e.get_by_role('button',name='Save command',exact=True).click();expect(e.locator('.form-error')).to_contain_text('decimal integer')
        retries.get_by_role('textbox',name='Minimum integer: retries',exact=True).fill('2')
        note=e.locator('.parameter-definition').filter(has=p.get_by_text('note',exact=True));note.get_by_role('button',name='Allow in presets',exact=True).click()
        directory=e.locator('.parameter-definition').filter(has=p.get_by_text('directory',exact=True));directory.get_by_role('textbox',name='Parameter label: directory',exact=True).fill('Destination directory')
        e.get_by_role('button',name='Save command',exact=True).click();expect(e).not_to_be_visible()
        d=open_run(app);expect(d.locator('.parameter-presets')).to_contain_text('Stale preset');select_preset(d,'Production · Stale preset');expect(d.get_by_role('button',name='Load selected preset',exact=True)).to_be_disabled()
        expect(d.locator('.parameter-value').filter(has=p.get_by_role('textbox',name='directory',exact=True))).to_contain_text('Destination directory')
        d.get_by_role('textbox',name='port',exact=True).fill('8181');d.get_by_role('button',name='Update selected preset',exact=True).click();eventually(lambda:presets(app)[0]['values']['port']=='8181');expect(d.get_by_role('button',name='Load selected preset',exact=True)).to_be_enabled();cancel(d)
        passed('parameter editing rejects invalid bounds; definition changes disable stale loads until an explicit update')
        p=mine(app);p.locator('.script-row').filter(has=p.get_by_text('Typed task',exact=True)).get_by_role('button',name='Edit',exact=True).click();e=p.get_by_role('dialog',name='Save command',exact=True)
        directory=e.locator('.parameter-definition').filter(has=p.get_by_text('directory',exact=True));directory.get_by_role('button',name='Allow in presets',exact=True).click();e.get_by_role('button',name='Save command',exact=True).click();expect(e).not_to_be_visible();assert 'directory' not in presets(app)[0]['values']
        passed('removing preset permission removes the previously saved value without rebasing the stale revision')
        p.get_by_role('button',name='New script',exact=True).click();e=p.get_by_role('dialog',name='Save command',exact=True);e.get_by_role('textbox',name='Command name',exact=True).fill('New typed')
        e.get_by_role('textbox',name='Shell command',exact=True).fill('echo {{port}} {{mode}} {{enabled}}');e.get_by_role('button',name='Configure parameters',exact=True).click();expect(e.locator('.parameter-definition')).to_have_count(3)
        port_rule=e.locator('.parameter-definition').filter(has=p.get_by_text('port',exact=True));port_rule.get_by_role('button',name='Port',exact=True).click();port_rule.get_by_role('textbox',name='Default value: port',exact=True).fill('443')
        mode_rule=e.locator('.parameter-definition').filter(has=p.get_by_text('mode',exact=True));mode_rule.get_by_role('button',name='Choice',exact=True).click();mode_rule.get_by_role('textbox',name='Options, one per line: mode',exact=True).fill('safe\nfast');mode_rule.get_by_role('textbox',name='Default value: mode',exact=True).fill('safe')
        enabled=e.locator('.parameter-definition').filter(has=p.get_by_text('enabled',exact=True));enabled.get_by_role('button',name='Boolean',exact=True).click();enabled.get_by_role('textbox',name='Default value: enabled',exact=True).fill('false')
        e.get_by_role('button',name='Save command',exact=True).click();expect(e).not_to_be_visible();d=open_run(app,'New typed');expect(d.get_by_role('textbox',name='port',exact=True)).to_have_value('443');expect(d.get_by_role('button',name='mode: safe',exact=True)).to_have_class(re.compile(r'\bchoice active\b'));expect(d.locator('.parameter-presets')).to_have_count(0);cancel(d)
        passed('authors can configure new port, choice and boolean arguments; preset storage starts disabled')
        target=out/f'typed-{time.time_ns()}.json';p.get_by_role('button',name='Export scripts…',exact=True).click();e=p.get_by_role('dialog',name='Export script bundle',exact=True)
        e.get_by_role('textbox',name='Export folder',exact=True).fill(str(out));e.get_by_role('textbox',name='Bundle file name',exact=True).fill(target.name);e.get_by_role('button',name='Typed task',exact=True).click()
        shared=e.locator('.script-share-editor');shared.get_by_role('textbox',name='Default value: port',exact=True).fill('8443');shared.get_by_role('textbox',name='Shared script name',exact=True).fill('Shared typed')
        e.get_by_role('button',name='Review export…',exact=True).click();expect(e.locator('.script-bundle-export-review')).to_contain_text('8443');expect(e.locator('.script-bundle-export-review')).not_to_contain_text('8181')
        e.get_by_role('button',name='Write new bundle',exact=True).click();expect(e).not_to_be_visible();payload=json.loads(target.read_text());assert payload['version']==2 and len(payload['scripts'])==1 and 'script_presets' not in payload
        assert next(v for v in saved(app)['snippets'] if v['id']=='typed')['parameters'][4]['default_value']=='8080'
        p.get_by_role('button',name='Import scripts…',exact=True).click();e=p.get_by_role('dialog',name='Import script bundle',exact=True);e.get_by_role('textbox',name='Script bundle path',exact=True).fill(str(target));e.get_by_role('button',name='Read bundle',exact=True).click();e.get_by_role('button',name='Review import…',exact=True).click();expect(e.locator('.script-bundle-import-review')).to_contain_text('Parameter definitions:');e.get_by_role('button',name='Apply import',exact=True).click();expect(e).not_to_be_visible()
        d=open_run(app,'Shared typed');expect(d.get_by_role('textbox',name='port',exact=True)).to_have_value('8443');expect(d.locator('.parameter-presets')).not_to_contain_text('Production');cancel(d)
        passed('v2 sharing copies can edit parameter defaults; typed metadata round trips without preset values or original changes')
        d=open_run(app);d.get_by_role('textbox',name='directory',exact=True).fill('/');d.get_by_role('textbox',name='note',exact=True).fill("literal '; $(id) {{port}}")
        d.locator('.target-choice').nth(0).click();d.locator('.target-choice').nth(1).click();d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready','Ready']);expect(p.locator('.task-target').first).to_contain_text("literal '\\''; $(id) {{port}}")
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Running','Running']);app.stop();time.sleep(4.5);app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click();expect(p.locator('.task-status')).to_have_text(['Result unknown','Result unknown'])
        for i in range(2):p.locator('.task-target').nth(i).get_by_role('button',name='Query server result',exact=True).click();p.get_by_role('dialog',name='Query server result',exact=True).get_by_role('button',name='Query',exact=True).click();expect(p.locator('.task-status').nth(i)).to_have_text('Succeeded')
        passed('typed values remain single-pass shell literals and multi-server work can be queried after closing the client')
        d=open_run(app,'Legacy task');expect(d.get_by_role('textbox',name='message',exact=True)).to_have_value('');expect(d.locator('.parameter-presets')).to_have_count(0);cancel(d)
        p.locator('.context-links').get_by_role('button',name='Official scripts',exact=True).click();p.locator('.script-row').filter(has=p.get_by_text('Directory usage',exact=True)).get_by_role('button',name='Run on servers…',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True);d.get_by_role('textbox',name='directory',exact=True).fill('/var/log');d.get_by_role('textbox',name='Preset name',exact=True).fill('Logs');d.get_by_role('button',name='Save new preset',exact=True).click();eventually(lambda:any(v['source']=='official' for v in presets(app)));assert len([v for v in presets(app) if v['source']=='user'])==1;cancel(d)
        passed('legacy scripts retain text behavior; official and user presets have distinct ownership')
        d=open_run(app);select_preset(d,'Production · Stale preset');d.get_by_role('button',name='Delete selected preset',exact=True).click();eventually(lambda:len(presets(app))==1);expect(d.locator('.parameter-presets')).not_to_contain_text('Production');cancel(d)
        passed('stale presets can be deleted independently of server tasks and official presets')
        app.stop();document=json.loads(app.config.read_text());document['config']['settings']['language']='SimplifiedChinese';app.config.write_text(json.dumps(document),encoding='utf-8');app.launch();p=app.page;p.set_viewport_size(dict(width=900,height=700));p.locator('.rail-scripts').click();p.locator('.context-links').get_by_role('button',name='我的脚本',exact=True).click();p.locator('.script-row').filter(has=p.get_by_text('Typed task',exact=True)).get_by_role('button',name='在服务器上运行…',exact=True).click();d=p.get_by_role('dialog',name='选择服务器运行脚本',exact=True)
        expect(d.get_by_role('button',name='保存新预设',exact=True)).to_be_visible();assert p.evaluate('document.documentElement.scrollWidth<=innerWidth');assert d.evaluate('(e)=>e.scrollWidth<=e.clientWidth');app.screenshot('typed-parameters-chinese-narrow.png')
        passed('Chinese narrow layout keeps typed controls and preset actions within the dialog')

(out/'results.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps(dict(passed=len(checks),checks=checks),ensure_ascii=False))
