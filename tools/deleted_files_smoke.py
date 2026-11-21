"""Finite, cancellable disk investigation from recent and independent diagnostics."""
import copy,json,time
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/deleted-files-ui'/str(time.time_ns());checks=[]
def passed(label):checks.append(label);print(label,flush=True)
def queries(peer):
    f=peer.root/'deleted-files-queries.jsonl'
    return [json.loads(s) for s in f.read_text().splitlines()] if f.exists() else []
def options(peer,**values):(peer.root/'deleted-files-fixture.json').write_text(json.dumps(values),encoding='utf-8')
def history(app):return eventually(lambda:json.loads(app.config.with_suffix('.tasks.json').read_text()),timeout=3)
def launches(peer):
    f=peer.root/'task-launches.jsonl';return f.read_bytes() if f.exists() else b''
def opened(p):
    p.locator('.rail-diagnostics').click();p.locator('.diagnostic-deleted-files').first.click()
    d=p.get_by_role('dialog',name='Deleted open files',exact=True);expect(d).to_be_visible();return d
def read(d):
    button=d.get_by_role('button',name='Read open files',exact=True);button.click();expect(button).to_be_disabled();expect(button).to_be_enabled(timeout=15000)

with Peer(out/'alpha') as a,Peer(out/'beta') as b:
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta')],out);host,port=b.data['address'].rsplit(':',1)
    config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    with Desktop(out/'desktop',config) as app:
        p=app.page;app.connect(1);p.locator('.rail-diagnostics').click()
        p.get_by_role('textbox',name='PEM certificate path (optional)',exact=True).fill('/etc/fixture.pem')
        p.get_by_role('button',name='Nginx',exact=True).click()
        p.get_by_role('button',name='Add configuration check',exact=True).click()
        p.locator('.diagnostic-configuration-row').get_by_role('button',name='OpenSSH',exact=True).click()
        p.get_by_role('button',name='Choose servers and review checks…',exact=True).click()
        d=p.get_by_role('dialog',name='Run script on servers',exact=True);d.locator('.target-choice').first.click()
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test')
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready'],timeout=20000)
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Succeeded'],timeout=20000)
        saved=history(app);before=launches(a);before_b=launches(b)
        d=opened(p);assert not queries(a) and not queries(b)
        expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True)).to_have_value('')
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');read(d)
        expect(d.locator('.deleted-files-summary')).to_contain_text('Observed files: 1')
        expect(d.locator('.deleted-file-path')).to_contain_text('[redacted] 中文.log (deleted)')
        expect(d.locator('.deleted-file-sizes')).to_contain_text('References 2')
        expect(d.locator('.deleted-file-holders')).to_contain_text('PID 120 · fd 4 · UID 1000 · worker [redacted]')
        expect(d.locator('.deleted-files-warning')).to_have_text('Some processes or descriptors were unavailable')
        assert queries(a)==[dict(administrator=False)] and not queries(b)
        app.screenshot('english-light.png')
        d.get_by_role('button',name='Close',exact=True).click();app.servers();expect(p.locator('.connection-state')).to_have_text('SSH connected');expect(p.locator('.host-row.selected')).to_contain_text('Beta')
        passed('Disk findings open an independent password/no-SFTP metadata read on Alpha; decoded names are redacted and Beta remains connected')

        d=opened(p);d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');read(d)
        options(a,snapshot=dict(limit='descriptors'));read(d);expect(d.locator('.deleted-files-warning')).to_contain_text('Scan limit reached')
        crowded=[dict(device='8:1',inode=100+i,path='/var/log/'+('long-name-'*45)+str(i)+' (deleted)',allocated_bytes=512,logical_bytes=300,references=1,
            holders=[dict(pid=120+i,start_ticks=500,uid=1000,fd=4,command='worker')]) for i in range(20)]
        options(a,snapshot=dict(processes=30,descriptors=50,files=25,rows=crowded,allocated_bytes=25*512,logical_bytes=25*300));read(d)
        expect(d.locator('.deleted-file-row')).to_have_count(20);expect(d.locator('.deleted-files-summary')).to_contain_text('Observed files: 25')
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert d.get_by_role('button',name='Read open files',exact=True).bounding_box()['y']<p.viewport_size['height']
        options(a,snapshot=dict(files=0,allocated_bytes=0,logical_bytes=0,rows=[]));read(d)
        expect(d.locator('.deleted-files-empty')).to_have_text('No deleted open regular files observed')
        expect(d.locator('.deleted-files-warning')).to_contain_text('unavailable')
        count=len(queries(a));time.sleep(.6);assert len(queries(a))==count
        options(a);read(d)
        passed('Bounded and permission-limited scans keep their coverage warning; a genuine empty observation remains explicit and no automatic refresh starts')

        for payload,error in [(dict(raw='not JSON'),'expected'),(dict(snapshot=dict(version=2)),'Invalid open file snapshot'),
            (dict(snapshot=dict(rows=[])),'Invalid open file snapshot'),(dict(raw='x'*(256*1024+1)),'exceeded the read limit'),
            (dict(exit=127,stderr='python3: not found arcnix-test'),'exit 127'),(dict(stderr='reader warning arcnix-test'),'reader reported')]:
            options(a,**payload);read(d);expect(d.locator('.deleted-files-error')).to_contain_text(error)
            expect(d.locator('.deleted-files-error')).not_to_contain_text('arcnix-test')
            expect(d.locator('.deleted-files-state')).to_contain_text('Showing the previous open file snapshot')
            expect(d.locator('.deleted-file-path')).to_contain_text('[redacted]')
        options(a)
        passed('Invalid, incomplete, oversized, unavailable and warning-bearing replies remain errors with a marked previous snapshot, never an empty successful scan')

        d.get_by_role('button',name='Read process metadata as administrator',exact=True).click();expect(d.locator('.deleted-files-result')).to_have_count(0)
        d.get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret');read(d)
        assert queries(a)[-1]['administrator'];expect(d.locator('.deleted-file-path')).not_to_contain_text('fixture-admin-secret')
        expect(d.locator('.deleted-file-holders')).not_to_contain_text('fixture-admin-secret')
        row=dict(device='8:1',inode=45,path='/var/log/changed.log (deleted)',allocated_bytes=512,logical_bytes=300,references=1,holders=[dict(pid=121,start_ticks=501,uid=0,fd=7,command='changed worker')])
        options(a,snapshot=dict(rows=[row],allocated_bytes=512,logical_bytes=300));read(d)
        expect(d.locator('.deleted-file-path')).to_have_text('/var/log/changed.log (deleted)');expect(d.locator('.deleted-file-holders')).to_contain_text('PID 121 · fd 7')
        passed('Administrator metadata reads require an explicit choice; changing access clears old data and repeated inode rows update their values and holders')

        options(a,delay=3);pending_count=len(queries(a));d.get_by_role('button',name='Read open files',exact=True).click();expect(d.get_by_role('button',name='Read open files',exact=True)).to_be_disabled()
        eventually(lambda:len(queries(a))>pending_count,timeout=3)
        d.get_by_role('button',name='Close',exact=True).click();eventually(lambda:(a.root/'deleted-files-cancelled.jsonl').exists(),timeout=4)
        d=opened(p);expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True)).to_have_value('');expect(d.locator('.deleted-files-result')).to_have_count(0)
        expect(d.get_by_role('textbox',name='Administrator password (optional)',exact=True)).to_have_count(0);time.sleep(.3);expect(d.locator('.deleted-files-state')).to_have_text('No open file snapshot yet')
        d.get_by_role('button',name='Close',exact=True).click();options(a)
        assert launches(a)==before and launches(b)==before_b and not queries(b)
        assert history(app)==saved
        raw=app.config.read_text()+app.config.with_suffix('.tasks.json').read_text();assert 'fixture-admin-secret' not in raw and 'arcnix-test' not in raw and 'changed.log' not in raw
        passed('Closing an active query closes its SSH channel, clears credentials and ignores late results; inspection creates no jobs and does not write observations into task history')

        app.stop();app.config.with_suffix('.tasks.json').unlink();app.launch();p=app.page;p.locator('.rail-tasks').click();p.get_by_role('button',name='Server records',exact=True).click()
        d=p.get_by_role('dialog',name='Connect to server records',exact=True);d.get_by_role('button',name='Select trusted servers',exact=True).click();d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test');d.get_by_role('button',name='Read server records',exact=True).click()
        record=p.locator('.server-record-group').first.locator('.server-task-record').filter(has=p.get_by_text(saved[0]['id'],exact=True));expect(record).to_be_visible(timeout=20000)
        record.get_by_role('button',name='View saved log',exact=True).click();d=p.get_by_role('dialog',name='Saved server task log',exact=True)
        d.get_by_role('button',name='Inspect deleted open files…',exact=True).click();d=p.get_by_role('dialog',name='Deleted open files',exact=True)
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');read(d);expect(d.locator('.deleted-file-row')).to_have_count(1)
        assert not app.config.with_suffix('.tasks.json').exists();d.get_by_role('button',name='Close',exact=True).click()
        passed('Independent server records retain the disk investigation entry after local task history has been removed')

        count=len(queries(a))
        for drift in ['account','endpoint','fingerprint']:
            app.stop();changed=copy.deepcopy(config)
            if drift=='account':changed['config']['servers'][0]['username']='another-account'
            elif drift=='endpoint':changed['config']['servers'][0]['port']=int(b.data['address'].rsplit(':',1)[1])
            else:changed['config']['trusted_hosts'][0]['fingerprint']=b.data['host_key_rsa_fingerprint']
            app.config.write_text(json.dumps(changed),encoding='utf-8');app.config.with_suffix('.tasks.json').write_text(json.dumps(saved),encoding='utf-8');app.launch();p=app.page;p.locator('.rail-diagnostics').click()
            expect(p.locator('.diagnostic-deleted-files')).to_be_disabled();assert len(queries(a))==count and not queries(b)
        app.stop();app.config.write_text(json.dumps(config),encoding='utf-8');bad=copy.deepcopy(saved);bad[0]['targets'][0]['output']='ARCNIX_DIAG_START_V1\t1700000000\t4\n'
        app.config.with_suffix('.tasks.json').write_text(json.dumps(bad),encoding='utf-8');app.launch();p=app.page;p.locator('.rail-diagnostics').click();expect(p.locator('.diagnostic-deleted-files')).to_have_count(0)
        app.stop();bad=copy.deepcopy(saved);bad[0]['diagnostic']['disk']=False
        app.config.with_suffix('.tasks.json').write_text(json.dumps(bad),encoding='utf-8');app.launch();p=app.page;p.locator('.rail-diagnostics').click();expect(p.locator('.diagnostic-deleted-files')).to_have_count(0)
        passed('Changed account, endpoint or fingerprint cannot redirect a read; incomplete saved diagnostics do not expose a disk finding action')

        app.stop();app.config.with_suffix('.tasks.json').write_text(json.dumps(saved),encoding='utf-8');app.launch();p=app.page
        app.settings('General');p.locator('.setting-theme:visible').get_by_role('radio', name='Dark', exact=True).check();p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check();p.locator('.rail-diagnostics').click();p.set_viewport_size(dict(width=900,height=780))
        p.locator('.diagnostic-deleted-files').first.click();d=p.get_by_role('dialog',name='已删除文件占用',exact=True)
        tip=d.get_by_role('button',name='帮助：已删除文件占用',exact=True);tip.hover();expect(p.get_by_role('tooltip')).to_contain_text('独立 SSH')
        tip.click();p.mouse.move(890,770);expect(p.get_by_role('tooltip')).to_be_visible();p.keyboard.press('Escape')
        d.get_by_role('textbox',name='SSH 密码或私钥口令',exact=True).fill('arcnix-test');d.get_by_role('button',name='读取文件占用',exact=True).click()
        expect(d.locator('.deleted-file-row')).to_have_count(1);expect(d.locator('.deleted-files-warning')).to_contain_text('无法读取')
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert not d.locator('*:visible').evaluate_all("els=>els.filter(el=>!el.children.length&&el.textContent.trim()&&parseFloat(getComputedStyle(el).fontSize)<12).map(el=>el.textContent)")
        button=d.get_by_role('button',name='读取文件占用',exact=True);assert button.bounding_box()['y']+button.bounding_box()['height']<=780
        app.screenshot('chinese-dark.png');d.get_by_role('button',name='关闭',exact=True).click()
        passed('Chinese 900 px UI keeps readable type, a visible footer, themed output and hover/click help without permanent explanatory paragraphs')

(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8');print(f'{len(checks)} groups passed; artifacts: {out}',flush=True)
