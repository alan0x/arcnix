"""Multiple certificate/configuration items, exact report binding and legacy UI."""
import base64,json,time
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/multi-diagnostics-ui'/str(time.time_ns());checks=[]
def passed(label):checks.append(label);print(label,flush=True)
def history(app):return eventually(lambda:json.loads(app.config.with_suffix('.tasks.json').read_text()),timeout=3)
def prepare(app,two=True):
    p=app.page;p.get_by_role('button',name='Choose servers and review checks…',exact=True).click()
    d=p.get_by_role('dialog',name='Run script on servers',exact=True)
    if two:d.get_by_role('button',name='Select visible trusted servers',exact=True).click()
    else:d.locator('.target-choice').first.click()
    d.get_by_role('button',name='Check targets',exact=True).click()
    expect(p.locator('.task-status')).to_have_text(['Ready']*(2 if two else 1))
    return p
def settings(p):
    p.locator('.rail-diagnostics').click()
    if not p.locator('.diagnostic-settings').count():p.get_by_role('button',name='Configure checks…',exact=True).click()
    expect(p.locator('.diagnostic-settings')).to_be_visible()

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    config=a.config([a.profile('Alpha'),b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1)
    config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    legacy_plan=dict(disk=False,inodes=False,services=False,ports=False,certificate='/srv/legacy.pem')
    detail=base64.b64encode(b'notAfter=Nov 15 2027 GMT').decode()
    legacy_output='ARCNIX_DIAG_START_V1\t1700000000\t1\nARCNIX_DIAG_V1\tcertificate\tpass\t-\t'+detail+'\nARCNIX_DIAG_END_V1\t1\n'
    profile=config['config']['servers'][0]
    legacy=dict(id='legacy-1',name='Legacy diagnostics',script_id='builtin:diagnostics:v1',revision='legacy',created=1700000000,diagnostic=legacy_plan,targets=[dict(id='Alpha',name='Alpha',endpoint=f"arcnix@{profile['host']}:{profile['port']}",fingerprint=a.data['host_key_rsa_fingerprint'],status='Succeeded',preview='',output=legacy_output,exit_code=0,remote_directory='/home/arcnix/.local/state/arcnix/tasks/legacy-1')])
    app=Desktop(out/'desktop',config)
    app.config.with_suffix('.tasks.json').write_text(json.dumps([legacy]))
    with app:
        p=app.page;p.locator('.rail-diagnostics').click()
        p.get_by_role('button',name='Findings and suggestions',exact=True).click()
        expect(p.locator('.diagnostic-finding')).to_have_count(1)
        expect(p.locator('.diagnostic-finding')).to_contain_text('/srv/legacy.pem')
        passed('Old single-item plans and v1 saved reports still show their original certificate context')
        settings(p)
        p.get_by_role('textbox',name='PEM certificate path (optional)',exact=True).fill('/etc/soon.pem')
        p.get_by_role('button',name='Nginx',exact=True).click()
        p.get_by_role('textbox',name='Configuration file (optional; default if empty)',exact=True).fill('/etc/nginx/nginx.conf')
        p.get_by_role('button',name='Add certificate',exact=True).click()
        expect(p.locator('.diagnostic-certificate-row')).to_have_count(1)
        p.get_by_role('textbox',name='Additional PEM certificate path',exact=True).first.fill('/etc/remove-me.pem')
        p.get_by_role('button',name='Add certificate',exact=True).click()
        expect(p.locator('.diagnostic-certificate-row')).to_have_count(2)
        p.get_by_role('textbox',name='Additional PEM certificate path',exact=True).nth(1).fill('/etc/expired.pem')
        p.locator('.diagnostic-certificate-row').first.get_by_role('button',name='Remove certificate',exact=True).click()
        expect(p.get_by_role('textbox',name='Additional PEM certificate path',exact=True)).to_have_value('/etc/expired.pem')
        for count,path in enumerate(['/etc/missing.pem',"/etc/healthy ' $${literal} {{literal}} @DISK@.pem"],2):
            p.get_by_role('button',name='Add certificate',exact=True).click()
            expect(p.locator('.diagnostic-certificate-row')).to_have_count(count)
            p.get_by_role('textbox',name='Additional PEM certificate path',exact=True).last.fill(path)
        p.get_by_role('button',name='Add configuration check',exact=True).click()
        expect(p.locator('.diagnostic-configuration-row')).to_have_count(1)
        row=p.locator('.diagnostic-configuration-row').first
        row.get_by_role('button',name='OpenSSH',exact=True).click()
        row.get_by_role('textbox',name='Additional configuration file (optional; default if empty)',exact=True).fill('/etc/ssh/sshd_config')
        p.get_by_role('button',name='Add configuration check',exact=True).click()
        expect(p.locator('.diagnostic-configuration-row')).to_have_count(2)
        p.locator('.diagnostic-configuration-row').last.get_by_role('textbox',name='Additional configuration file (optional; default if empty)',exact=True).fill('/etc/nginx/nginx.conf')
        for index,path in enumerate(['/etc/expired.pem','/etc/missing.pem',"/etc/healthy ' $${literal} {{literal}} @DISK@.pem"]):
            expect(p.get_by_role('textbox',name='Additional PEM certificate path',exact=True).nth(index)).to_have_value(path)
        p.get_by_role('button',name='Choose servers and review checks…',exact=True).click()
        expect(p.locator('.form-error')).to_contain_text('Duplicate diagnostic configuration')
        p.locator('.diagnostic-configuration-row').last.get_by_role('textbox',name='Additional configuration file (optional; default if empty)',exact=True).fill('/etc/nginx/bad.conf')
        p.get_by_role('textbox',name='Additional PEM certificate path',exact=True).last.fill('/etc/soon.pem')
        p.get_by_role('button',name='Choose servers and review checks…',exact=True).click()
        expect(p.locator('.form-error')).to_contain_text('Duplicate diagnostic certificate')
        p.get_by_role('textbox',name='Additional PEM certificate path',exact=True).last.fill("/etc/healthy ' $${literal} {{literal}} @DISK@.pem")
        p.get_by_role('button',name='Help: Multiple diagnostic items',exact=True).scroll_into_view_if_needed()
        p.evaluate('() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))')
        p.get_by_role('button',name='Help: Multiple diagnostic items',exact=True).focus()
        try:expect(p.get_by_role('tooltip')).to_contain_text('20 checks')
        except Exception:
            app.screenshot('help-failure.png')
            (out/'help-failure.json').write_text(json.dumps(p.get_by_role('button',name='Help: Multiple diagnostic items',exact=True).evaluate('el => ({html:el.parentElement.outerHTML,active:document.activeElement===el})')))
            raise
        p.keyboard.press('Escape')
        app.screenshot('multi-item-form.png')
        passed('Stable editable rows retain values after removal; duplicate certificate/configuration items are rejected before submission')

        p=prepare(app);identifier=history(app)[0]['id']
        for text in ['ARCNIX_DIAG_START_V2','certificate:3','configuration:2','{{literal}} @DISK@.pem']:
            expect(p.locator('.task-target').first).to_contain_text(text)
        p.get_by_role('button',name='Execute ready targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Running','Running'])
        app.stop();time.sleep(4.5);app.launch();p=app.page;p.locator('.rail-diagnostics').click()
        expect(p.locator('.diagnostic-run').first.locator('.diagnostic-summary')).to_have_text(['Result unknown','Result unknown'])
        p.locator('.diagnostic-run').first.get_by_role('button',name='View task and raw output',exact=True).click()
        p.locator('.task-batch-refresh:visible').click();d=p.get_by_role('dialog',name='Query batch results',exact=True)
        d.get_by_role('button',name='Query selected results',exact=True).click();expect(d).not_to_be_visible()
        expect(p.locator('.task-status')).to_have_text(['Succeeded','Succeeded'])
        p.locator('.rail-diagnostics').click();card=p.locator('.diagnostic-run').first
        expect(card.locator('.diagnostic-summary')).to_contain_text(['6 findings · 2 inconclusive','6 findings · 2 inconclusive'])
        for button in card.get_by_role('button',name='Findings and suggestions',exact=True).all():button.click()
        expect(card.locator('.diagnostic-finding')).to_have_count(22)
        expect(card.locator('.diagnostic-critical')).to_have_count(4)
        expect(card.locator('.diagnostic-unknown')).to_have_count(4)
        target=card.locator('.diagnostic-target').first
        for path in ['/etc/soon.pem','/etc/expired.pem','/etc/missing.pem',"/etc/healthy ' $${literal} {{literal}} @DISK@.pem",'OpenSSH · /etc/ssh/sshd_config','Nginx · /etc/nginx/bad.conf']:
            expect(target.locator('.diagnostic-item-context').filter(has_text=path)).to_have_count(1)
        expired=target.locator('.diagnostic-finding').filter(has_text='/etc/expired.pem')
        expect(expired).to_have_count(1)
        tip=expired.get_by_role('button',name='Help: Local certificate expiry',exact=True)
        tip.scroll_into_view_if_needed()
        p.evaluate('() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))')
        tip.focus()
        expect(p.get_by_role('tooltip')).to_contain_text('first certificate in each selected local PEM')
        p.keyboard.press('Escape');app.screenshot('per-item-findings.png')
        passed('Two server-owned multi-item jobs survive client exit and recover 22 distinct findings with exact saved paths and mixed severities')

        for flag in ['diagnostic-wrong-selection','diagnostic-duplicate-item','diagnostic-reordered-items']:
            settings(p)
            p.get_by_role('textbox',name='PEM certificate path (optional)',exact=True).fill('/etc/soon.pem')
            if not p.locator('.diagnostic-certificate-row').count():p.get_by_role('button',name='Add certificate',exact=True).click()
            p.get_by_role('textbox',name='Additional PEM certificate path',exact=True).first.fill('/etc/healthy.pem')
            marker=a.root/flag;marker.touch()
            p=prepare(app,False);p.get_by_role('button',name='Execute ready targets',exact=True).click()
            expect(p.locator('.task-status')).to_have_text(['Succeeded'],timeout=20000)
            p.locator('.rail-diagnostics').click()
            expect(p.locator('.diagnostic-run').first).to_contain_text('Diagnostic results are not complete')
            expect(p.locator('.diagnostic-run').first.get_by_role('button',name='Findings and suggestions',exact=True)).to_be_disabled()
            marker.unlink();passed(flag+' cannot be presented as a complete set of diagnostic findings')

        app.settings('General');p.get_by_role('button',name='简体中文',exact=True).click()
        expect(p.locator('.page-header-title')).to_have_text('常规')
        p.locator('.rail-diagnostics').click();p.set_viewport_size(dict(width=900,height=700))
        good=next(i for i,task in enumerate(history(app)) if task['id']==identifier)
        card=p.locator('.diagnostic-run').nth(good)
        expect(card.locator('.diagnostic-finding')).to_have_count(22)
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        card.scroll_into_view_if_needed();app.screenshot('multi-findings-chinese-narrow.png')
        p.get_by_role('button',name='配置检查项…',exact=True).click()
        expect(p.get_by_role('textbox',name='额外 PEM 证书路径',exact=True)).to_be_visible()
        app.screenshot('multi-form-chinese-narrow.png')
        saved=history(app);request=next(task for task in saved if task['id']==identifier)['diagnostic']
        assert len(request['certificates'])==3 and len(request['configurations'])==2
        assert not any('id' in item for item in request['configurations'])
        assert saved[-1]['id']=='legacy-1'
        passed('Chinese narrow forms and reports preserve multiple saved items and legacy history without storing UI row identities')

(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps(dict(passed=len(checks),artifacts=str(out)),ensure_ascii=False),flush=True)
