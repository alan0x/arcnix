"""Finite listener investigation from recent and independent diagnostics."""
import copy,json,time
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/tcp-listeners-ui'/str(time.time_ns());checks=[]
def passed(label):checks.append(label);print(label,flush=True)
def queries(peer):
    f=peer.root/'tcp-listeners-queries.jsonl'
    return [json.loads(s) for s in f.read_text().splitlines()] if f.exists() else []
def options(peer,**values):(peer.root/'tcp-listeners-fixture.json').write_text(json.dumps(values),encoding='utf-8')
def history(app):return eventually(lambda:json.loads(app.config.with_suffix('.tasks.json').read_text()),timeout=3)
def launches(peer):
    f=peer.root/'task-launches.jsonl';return f.read_bytes() if f.exists() else b''
def opened(p):
    p.locator('.rail-diagnostics').click();p.locator('.diagnostic-tcp-listeners').first.click()
    d=p.get_by_role('dialog',name='TCP listeners',exact=True);expect(d).to_be_visible();return d
def read(d):
    button=d.get_by_role('button',name='Read TCP listeners',exact=True);button.click();expect(button).to_be_disabled();expect(button).to_be_enabled(timeout=15000)
def password(d):d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test')

with Peer(out/'alpha') as a,Peer(out/'beta') as b:
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta')],out,updates=dict(automatic_checks=False,automatic_install=False))
    host,port=b.data['address'].rsplit(':',1)
    config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    for peer in (a,b):(peer.root/'trace-task-queries').touch()
    with Desktop(out/'desktop',config) as app:
        p=app.page;app.connect(1)
        p.get_by_role('button',name='Terminal',exact=True).click()
        terminal=p.locator('.terminal-surface .xterm-accessibility-tree')
        expect(terminal).to_contain_text('Arcnix terminal fixture')
        p.locator('.xterm-helper-textarea').focus();p.keyboard.type('echo TCP_SESSION_KEPT');p.keyboard.press('Enter')
        expect(terminal).to_contain_text('TCP_SESSION_KEPT')
        p.locator('.rail-diagnostics').click()
        p.get_by_role('textbox',name='Expected TCP ports (optional, comma-separated)',exact=True).fill('443')
        p.get_by_role('button',name='Choose servers and review checks…',exact=True).click()
        d=p.get_by_role('dialog',name='Run script on servers',exact=True);d.locator('.target-choice').first.click();password(d)
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready'],timeout=20000)
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Succeeded'],timeout=20000)
        saved=history(app);before=launches(a);before_b=launches(b);disk_before=app.config.read_bytes();history_before=app.config.with_suffix('.tasks.json').read_bytes()
        d=opened(p);assert not queries(a) and not queries(b)
        expect(d.get_by_role('textbox',name='TCP port filter (optional, comma-separated)',exact=True)).to_have_value('443')
        expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True)).to_have_value('')
        password(d);read(d)
        expect(d.locator('.tcp-listeners-summary')).to_have_text('Observed TCP listeners: 2 · Showing 2')
        expect(d.locator('.tcp-listener-endpoint')).to_have_text(['127.0.0.1:443','[::]:443'])
        expect(d.locator('.tcp-listener-holders').first).to_contain_text('PID 120 · worker [redacted]')
        expect(d.locator('.tcp-listener-holders').last).to_have_text('Process information unavailable')
        assert queries(a)==[dict(ports=[443],administrator=False)] and not queries(b)
        app.screenshot('english-light.png');d.get_by_role('button',name='Close',exact=True).click()
        app.servers();expect(p.locator('.connection-state')).to_have_text('SSH connected');expect(p.locator('.host-row.selected')).to_contain_text('Beta')
        p.get_by_role('button',name='Terminal',exact=True).click();expect(terminal).to_contain_text('TCP_SESSION_KEPT')
        passed('A saved port finding prefills Alpha and its filter, reads without SFTP, redacts process metadata and preserves Beta SSH/terminal')

        d=opened(p);password(d)
        field=d.get_by_role('textbox',name='TCP port filter (optional, comma-separated)',exact=True)
        count=len(queries(a));field.fill('443;id');d.get_by_role('button',name='Read TCP listeners',exact=True).click()
        expect(d.locator('.tcp-listeners-error')).to_contain_text('comma-separated');assert len(queries(a))==count
        field.fill('443,80,443');read(d);assert queries(a)[-1]['ports']==[80,443]
        expect(d.locator('.tcp-listener-endpoint')).to_have_text(['127.0.0.1:80','[::]:443'])
        field.fill('443');options(a,snapshot=dict(limits=['descriptors']));read(d)
        expect(d.locator('.tcp-listeners-warning')).to_contain_text('Scan limit reached')
        crowded=[dict(family=4,address='127.0.0.1',port=443,uid=1000,inode=100+i,holders=[],holders_limited=False) for i in range(20)]
        options(a,snapshot=dict(matched=25,rows=crowded));read(d);expect(d.locator('.tcp-listener-row')).to_have_count(20)
        expect(d.locator('.tcp-listeners-summary')).to_contain_text('25 · Showing 20')
        assert d.get_by_role('button',name='Read TCP listeners',exact=True).bounding_box()['y']<p.viewport_size['height']
        options(a,snapshot=dict(matched=0,rows=[]));read(d);expect(d.locator('.tcp-listeners-empty')).to_have_text('No matching TCP listeners observed')
        count=len(queries(a));time.sleep(.6);assert len(queries(a))==count
        options(a);read(d)
        passed('Port input validates locally and canonicalizes scope; capped and empty observations keep explicit coverage and never start automatic refresh')

        for payload,error in [(dict(raw='not JSON'),'expected'),(dict(snapshot=dict(version=2)),'Invalid TCP listener snapshot'),
            (dict(snapshot=dict(ports=[80])),'Invalid TCP listener snapshot'),(dict(snapshot=dict(rows=[])),'Invalid TCP listener snapshot'),
            (dict(raw='x'*(256*1024+1)),'exceeded the read limit'),(dict(exit=127,stderr='python3: not found arcnix-test'),'exit 127'),
            (dict(stderr='reader warning arcnix-test'),'reader reported')]:
            options(a,**payload);read(d);expect(d.locator('.tcp-listeners-error')).to_contain_text(error)
            expect(d.locator('.tcp-listeners-error')).not_to_contain_text('arcnix-test')
            expect(d.locator('.tcp-listeners-state')).to_contain_text('Showing the previous TCP listener snapshot')
            expect(d.locator('.tcp-listener-row')).to_have_count(2)
        options(a)
        passed('Malformed, mismatched, oversized, unavailable and warning-bearing replies preserve a marked previous snapshot and hide known credentials')

        d.get_by_role('button',name='Read process metadata as administrator',exact=True).click()
        expect(d.locator('.tcp-listeners-result')).to_have_count(0)
        d.get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret');read(d)
        assert queries(a)[-1]['administrator']
        private_text=d.locator('.tcp-listeners-result').inner_text()
        assert 'fixture-admin-secret' not in private_text and 'arcnix-test' not in private_text
        changed=dict(family=4,address='127.0.0.1',port=443,uid=1000,inode=42,holders_limited=False,
                     holders=[dict(pid=121,start_ticks=501,uid=0,fd=7,command='changed worker',cgroup=None,unit=None)])
        options(a,snapshot=dict(matched=1,rows=[changed]));read(d)
        expect(d.locator('.tcp-listener-holders')).to_contain_text('PID 121 · changed worker')
        passed('Administrator reads require a separate explicit choice; changed holder data updates existing listener rows without retaining old process values')

        options(a,delay=3);count=len(queries(a));d.get_by_role('button',name='Read TCP listeners',exact=True).click()
        eventually(lambda:len(queries(a))>count,timeout=3)
        d.get_by_role('button',name='Close',exact=True).click();eventually(lambda:(a.root/'tcp-listeners-cancelled.jsonl').exists(),timeout=4)
        d=opened(p);expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True)).to_have_value('')
        expect(d.get_by_role('textbox',name='Administrator password (optional)',exact=True)).to_have_count(0)
        expect(d.locator('.tcp-listeners-result')).to_have_count(0)
        time.sleep(.3);expect(d.locator('.tcp-listeners-state')).to_have_text('No TCP listener snapshot yet')
        d.get_by_role('button',name='Close',exact=True).click();options(a)
        assert before==launches(a) and before_b==launches(b) and not queries(b)
        assert app.config.read_bytes()==disk_before and app.config.with_suffix('.tasks.json').read_bytes()==history_before
        passed('Closing cancels the SSH channel, clears credentials and ignores late results; metadata reads add no server jobs or saved configuration/history')

        app.stop();app.config.with_suffix('.tasks.json').unlink();app.launch();p=app.page
        p.locator('.rail-tasks').click();p.get_by_role('button',name='Server records',exact=True).click()
        d=p.get_by_role('dialog',name='Connect to server records',exact=True)
        d.get_by_role('button',name='Select trusted servers',exact=True).click()
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test')
        d.get_by_role('button',name='Read server records',exact=True).click()
        record=p.locator('.server-record-group').first.locator('.server-task-record').filter(has=p.get_by_text(saved[0]['id'],exact=True));expect(record).to_be_visible(timeout=20000)
        record.get_by_role('button',name='View saved log',exact=True).click()
        p.get_by_role('dialog',name='Saved server task log',exact=True).get_by_role('button',name='Inspect TCP listeners…',exact=True).click()
        d=p.get_by_role('dialog',name='TCP listeners',exact=True);password(d);read(d)
        expect(d.locator('.tcp-listener-row')).to_have_count(2)
        assert not app.config.with_suffix('.tasks.json').exists();d.get_by_role('button',name='Close',exact=True).click()
        passed('Independent server records retain the port investigation entry and original filter after local history is removed')

        count=len(queries(a))
        for drift in ('account','endpoint','fingerprint'):
            app.stop();changed_config=copy.deepcopy(config)
            if drift=='account':changed_config['config']['servers'][0]['username']='another-account'
            elif drift=='endpoint':changed_config['config']['servers'][0]['port']=int(b.data['address'].rsplit(':',1)[1])
            else:changed_config['config']['trusted_hosts'][0]['fingerprint']=b.data['host_key_rsa_fingerprint']
            app.config.write_text(json.dumps(changed_config));app.config.with_suffix('.tasks.json').write_text(json.dumps(saved));app.launch();p=app.page
            p.locator('.rail-diagnostics').click();expect(p.locator('.diagnostic-tcp-listeners').first).to_be_disabled()
        assert count==len(queries(a))
        app.stop();app.config.write_text(json.dumps(config));bad=copy.deepcopy(saved);bad[0]['diagnostic']['ports']=False
        app.config.with_suffix('.tasks.json').write_text(json.dumps(bad));app.launch();p=app.page;p.locator('.rail-diagnostics').click()
        expect(p.locator('.diagnostic-tcp-listeners')).to_have_count(0)
        passed('Changed account, endpoint, fingerprint or saved port selection cannot redirect or reauthorize an inspection')

        app.stop();app.config.with_suffix('.tasks.json').write_text(json.dumps(saved));app.launch();p=app.page
        app.settings('General');p.get_by_role('button',name='Dark',exact=True).click();p.get_by_role('button',name='简体中文',exact=True).click()
        p.locator('.rail-diagnostics').click();p.set_viewport_size(dict(width=900,height=780))
        p.locator('.diagnostic-tcp-listeners').first.click();d=p.get_by_role('dialog',name='TCP 监听',exact=True)
        tip=d.get_by_role('button',name='帮助：TCP 监听',exact=True);tip.hover();expect(p.get_by_role('tooltip')).to_contain_text('独立 SSH')
        tip.click();p.mouse.move(890,770);expect(p.get_by_role('tooltip')).to_be_visible();p.keyboard.press('Escape')
        d.get_by_role('textbox',name='SSH 密码或私钥口令',exact=True).fill('arcnix-test')
        d.get_by_role('button',name='读取 TCP 监听',exact=True).click();expect(d.locator('.tcp-listener-row')).to_have_count(2)
        expect(d.locator('.tcp-listeners-summary')).to_contain_text('已观察 TCP 监听：2')
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert not d.locator('*:visible').evaluate_all("els=>els.filter(e=>!e.children.length&&e.textContent.trim()&&parseFloat(getComputedStyle(e).fontSize)<12).map(e=>e.textContent)")
        button=d.get_by_role('button',name='读取 TCP 监听',exact=True);box=button.bounding_box();assert box['y']+box['height']<=780
        app.screenshot('chinese-dark-900.png');d.get_by_role('button',name='关闭',exact=True).click()
        passed('Chinese dark narrow dialogs keep readable type, IPv6, visible actions and hover/click help without permanent explanatory paragraphs')

(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8')
print(f'{len(checks)} groups passed; artifacts: {out}',flush=True)
