"""Finite service-journal reads keep the original account and workspace intact."""
import json,re,time
from desktop_fixture import Peer,Desktop,expect,ROOT

out=ROOT/'artifacts/diagnostic-log-ui'/str(time.time_ns());checks=[]
def passed(label):checks.append(label);print(label,flush=True)
def queries(peer):
    path=peer.root/'diagnostic-log-queries.jsonl'
    return [json.loads(s) for s in path.read_text().splitlines()] if path.exists() else []
def launches(peer):
    path=peer.root/'task-launches.jsonl'
    return path.read_bytes() if path.exists() else b''
def open_logs(p):
    p.locator('.rail-diagnostics').click();p.locator('.diagnostic-service-logs').first.click()
    d=p.get_by_role('dialog',name='Diagnostic service logs',exact=True);expect(d).to_be_visible();return d
def read(d):
    d.get_by_role('button',name='Read service logs',exact=True).click()
    expect(d.get_by_role('button',name='Read service logs',exact=True)).to_be_disabled()
    expect(d.get_by_role('button',name='Read service logs',exact=True)).to_be_enabled(timeout=15000)

with Peer(out/'alpha') as a,Peer(out/'beta') as b:
    (a.root/'diagnostic-log-units').touch()
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    with Desktop(out/'desktop',config) as app:
        p=app.page;app.connect(1)
        p.locator('.rail-diagnostics').click();p.get_by_role('button',name='Nginx',exact=True).click();p.get_by_role('button',name='Add configuration check',exact=True).click();p.locator('.diagnostic-configuration-row').get_by_role('button',name='OpenSSH',exact=True).click();p.get_by_role('button',name='Choose servers and review checks…',exact=True).click()
        d=p.get_by_role('dialog',name='Run script on servers',exact=True);d.locator('.target-choice').first.click()
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test')
        d.get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret')
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready'],timeout=20000)
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Succeeded'],timeout=20000)
        saved=json.loads(app.config.with_suffix('.tasks.json').read_text());before=launches(a);before_b=launches(b)
        d=open_logs(p);expect(d.locator('.diagnostic-log-unit')).to_have_text(['nginx.service',r'worker@blue\x20team.service',r'a\x2ablue.service'])
        expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True)).to_have_value('')
        assert queries(a)==queries(b)==[]
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');read(d)
        expect(d.locator('.diagnostic-log-output')).to_contain_text('Saved service failure nginx.service [redacted] 中文')
        expect(d.locator('.diagnostic-log-warnings')).to_contain_text('Journal fixture warning [redacted]')
        q=queries(a)[-1];assert q==dict(unit='nginx.service',period=['--since=@1794639100','--until=@1794640900'],limit=200,administrator=False),q
        assert not queries(b)
        d.get_by_role('button',name='Close',exact=True).click();app.servers();expect(p.locator('.connection-state')).to_have_text('SSH connected')
        expect(p.locator('.host-row.selected')).to_contain_text('Beta')
        passed('Saved failed rows offer only literal service names; the independent password/no-SFTP read uses the original Alpha account and diagnostic window while Beta stays connected')

        d=open_logs(p);d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');read(d)
        n=len(queries(a));d.get_by_role('button',name=r'worker@blue\x20team.service',exact=True).click()
        expect(d.locator('.diagnostic-log-output')).to_have_text('');assert len(queries(a))==n
        d.get_by_role('button',name='Last hour',exact=True).click();d.get_by_role('button',name='500',exact=True).click();read(d)
        q=queries(a)[-1];assert q['unit']==r'worker@blue\x20team.service' and q['limit']==500 and q['period']==['--since=1 hour ago']
        (a.root/'diagnostic-log-denied').touch();read(d)
        expect(d.locator('.diagnostic-log-state')).to_contain_text('Showing the previous log snapshot');expect(d.locator('.diagnostic-log-output')).to_contain_text(r'worker@blue\x20team.service')
        expect(d.locator('.diagnostic-log-error')).to_contain_text('exit 1');expect(d.locator('.diagnostic-log-error')).not_to_contain_text('arcnix-test')
        (a.root/'diagnostic-log-denied').unlink()
        d.get_by_role('button',name='Read journal as administrator',exact=True).click();expect(d.locator('.diagnostic-log-output')).to_have_text('')
        d.get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret');read(d)
        assert queries(a)[-1]['administrator'];expect(d.locator('.diagnostic-log-output')).not_to_contain_text('fixture-admin-secret');expect(d.locator('.diagnostic-log-warnings')).not_to_contain_text('fixture-admin-secret')
        passed('Changing unit, period or limit clears the snapshot without reading; explicit administrator reads redact decoded content and failed repeats retain a marked previous snapshot')

        for mode,error in [('missing','exit 127'),('invalid','JSON'),('oversize','exceeded the read limit')]:
            flag=a.root/('diagnostic-log-'+mode);flag.touch();read(d);expect(d.locator('.diagnostic-log-error')).to_contain_text(error);expect(d.locator('.diagnostic-log-state')).to_contain_text('previous log snapshot');flag.unlink()
        (a.root/'diagnostic-log-empty').touch();read(d);expect(d.locator('.diagnostic-log-output')).to_have_text('No journal entries returned for this selection');expect(d.locator('.diagnostic-log-warnings')).to_contain_text('Journal fixture warning');(a.root/'diagnostic-log-empty').unlink()
        passed('Unavailable journal, invalid JSON and oversized replies remain errors; genuine empty results keep permission warnings and never fall back to a different source')

        (a.root/'diagnostic-log-slow').touch();d.get_by_role('button',name='Read service logs',exact=True).click();expect(d.get_by_role('button',name='Read service logs',exact=True)).to_be_disabled();d.get_by_role('button',name='Close',exact=True).click()
        d=open_logs(p);expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True)).to_have_value('');expect(d.get_by_role('textbox',name='Administrator password (optional)',exact=True)).to_have_count(0)
        time.sleep(2.5);expect(d.locator('.diagnostic-log-output')).to_have_text('');expect(d.locator('.diagnostic-log-state')).to_have_text('No log snapshot yet');(a.root/'diagnostic-log-slow').unlink();d.get_by_role('button',name='Close',exact=True).click()
        assert launches(a)==before and launches(b)==before_b and not queries(b)
        raw=app.config.read_text()+app.config.with_suffix('.tasks.json').read_text();assert 'fixture-admin-secret' not in raw and 'arcnix-test' not in raw
        passed('Closing a pending read discards its late result and clears credentials; log reads create no hosted jobs or persisted credentials and never contact Beta')

        app.stop();app.config.with_suffix('.tasks.json').unlink();app.launch();p=app.page;p.locator('.rail-tasks').click();p.get_by_role('button',name='Server records',exact=True).click()
        d=p.get_by_role('dialog',name='Connect to server records',exact=True);d.get_by_role('button',name='Select trusted servers',exact=True).click();d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test');d.get_by_role('button',name='Read server records',exact=True).click()
        row=p.locator('.server-record-group').first.locator('.server-task-record').filter(has=p.get_by_text(saved[0]['id'],exact=True));expect(row).to_be_visible(timeout=20000);row.get_by_role('button',name='View saved log',exact=True).click()
        d=p.get_by_role('dialog',name='Saved server task log',exact=True);d.get_by_role('button',name='Failed service logs…',exact=True).click();d=p.get_by_role('dialog',name='Diagnostic service logs',exact=True)
        expect(d.locator('.diagnostic-log-unit')).to_have_count(3);d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');read(d);assert queries(a)[-1]['period']==['--since=@1794639100','--until=@1794640900']
        assert not app.config.with_suffix('.tasks.json').exists();d.get_by_role('button',name='Close',exact=True).click()
        passed('Independent server metadata restores service names and the original sampling window after local history deletion without rebuilding task history')

        count=len(queries(a))
        for drift in ['account','endpoint','fingerprint']:
            app.stop();changed=json.loads(json.dumps(config))
            if drift=='account':changed['config']['servers'][0]['username']='another-account'
            elif drift=='endpoint':changed['config']['servers'][0]['port']=int(b.data['address'].rsplit(':',1)[1])
            else:changed['config']['trusted_hosts'][0]['fingerprint']=b.data['host_key_rsa_fingerprint']
            app.config.write_text(json.dumps(changed),encoding='utf-8');app.config.with_suffix('.tasks.json').write_text(json.dumps(saved),encoding='utf-8');app.launch();p=app.page;p.locator('.rail-diagnostics').click()
            expect(p.locator('.diagnostic-service-logs')).to_be_disabled();assert len(queries(a))==count and not queries(b)
        passed('Changed account, endpoint or trusted fingerprint disables the original diagnostic entry and cannot redirect a log read to another server')

    legacy=json.loads(json.dumps(saved));legacy[0]['diagnostic']=None;legacy[0]['targets'][0]['status']='Unknown';legacy[0]['targets'][0]['exit_code']=None
    zh=json.loads(json.dumps(config));zh['config']['settings']['language']='SimplifiedChinese'
    with Desktop(out/'chinese',zh) as app:
        app.stop();app.config.with_suffix('.tasks.json').write_text(json.dumps(legacy),encoding='utf-8');app.launch();p=app.page;p.set_viewport_size(dict(width=900,height=780));p.locator('.rail-diagnostics').click();p.get_by_role('button',name='故障服务日志…',exact=True).click()
        d=p.get_by_role('dialog',name='诊断服务日志',exact=True);expect(d).to_contain_text('采集结果尚未确认')
        tip=d.get_by_role('button',name='帮助：诊断服务日志',exact=True);tip.hover();expect(p.get_by_role('tooltip')).to_contain_text('独立 SSH');tip.click();p.mouse.move(890,770);expect(p.get_by_role('tooltip')).to_be_visible();p.keyboard.press('Escape')
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert not d.locator('*:visible').evaluate_all("els=>els.filter(el=>!el.children.length&&el.textContent.trim()&&parseFloat(getComputedStyle(el).fontSize)<12).map(el=>el.textContent)")
        button=d.get_by_role('button',name='读取服务日志',exact=True);assert button.bounding_box()['y']+button.bounding_box()['height']<=780
        app.screenshot('chinese.png');d.get_by_role('button',name='关闭',exact=True).click()
        # Incomplete saved protocol must never expose a service action.
        app.stop();legacy[0]['targets'][0]['output']=re.sub(r'ARCNIX_DIAG_END_V2[^\n]*\n','',legacy[0]['targets'][0]['output']);app.config.with_suffix('.tasks.json').write_text(json.dumps(legacy),encoding='utf-8');app.launch();p=app.page;p.locator('.rail-diagnostics').click();expect(p.locator('.diagnostic-service-logs')).to_have_count(0)
        passed('Unconfirmed legacy findings remain labelled read-only observations; incomplete reports offer no entry, and Chinese 900 px UI keeps readable type, hover/click help and a visible footer')

(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8');print(f'{len(checks)} groups passed; artifacts: {out}',flush=True)
