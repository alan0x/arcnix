"""Owned WebView/SSH checks for metadata browsing and selected-path restores."""
import json,re,time
from desktop_fixture import Peer,Desktop,expect,ROOT,eventually

out=ROOT/'artifacts/snapshot-files-ui'
def load(app,server='Alpha'):
    p=app.page;p.locator('.rail-backups').click();p.get_by_role('button',name='Browse snapshots…',exact=True).click()
    d=p.get_by_role('dialog',name='Backup snapshots',exact=True);d.get_by_role('button',name=server,exact=True).click()
    if server=='Alpha':d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test')
    d.get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret');d.get_by_role('button',name='Load snapshots',exact=True).click();expect(d.locator('.backup-snapshot')).to_have_count(1)
    d.get_by_role('button',name='Browse files…',exact=True).click();d=p.get_by_role('dialog',name='Snapshot files',exact=True)
    expect(d.locator('.snapshot-files-state')).to_have_text('Snapshot file query completed',timeout=30000);return d

def directory(d,path):
    d.get_by_role('textbox',name='Snapshot directory',exact=True).fill(path);d.get_by_role('button',name='Open snapshot directory',exact=True).click()
    expect(d.locator('.snapshot-files-summary')).to_contain_text(path+' ·',timeout=30000);expect(d.locator('.snapshot-files-state')).to_have_text('Snapshot file query completed',timeout=30000)

def select(d,path):
    d.locator('.snapshot-file-row').filter(has=d.page.get_by_text(path,exact=True)).get_by_role('button',name='Select path',exact=True).click()

def prepare(app):
    p=app.page;d=p.get_by_role('dialog',name='Run script on servers',exact=True)
    expect(d.locator('.target-choice.active')).to_have_count(1)
    d.locator('.target-choice').filter(has_text=re.compile(r'^Beta ·')).click()
    d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test')
    for box in d.get_by_role('textbox',name='Administrator password (optional)',exact=True).all():box.fill('fixture-admin-secret')
    d.get_by_role('button',name='Check targets',exact=True).click()

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    config['config']['backup_policies']=[dict(id='website',name='Website',repository='/backups/site',password_file='/secrets/restic',source='/srv/site',servers=['Alpha','Beta'],administrator=True,excludes=[],keep_last=7)]
    with Desktop(out/f'desktop-{time.time_ns()}',config) as app:
        p=app.page;d=load(app)
        assert not app.config.with_suffix('.tasks.json').exists(), 'Metadata browsing must not create task history'
        expect(d.locator('.snapshot-file-row')).to_have_count(4)
        expect(d.locator('.snapshot-file-row').filter(has=d.page.get_by_text('/srv/site/link',exact=True)).get_by_role('button',name='Select path',exact=True)).to_be_disabled()
        select(d,'/srv/site/config[prod]*?.txt');expect(d.get_by_role('button',name='Restore selected…',exact=True)).to_be_enabled()
        old=d.locator('.snapshot-files-summary').inner_text()
        for flag,error in [('snapshot-files-wrong-id','Snapshot is outside this policy'),('snapshot-files-cross-path','Invalid snapshot child metadata'),('snapshot-files-truncated','EOF while parsing'),('snapshot-files-fail','fixture snapshot read failed')]:
            (a.root/flag).touch();d.get_by_role('button',name='Open snapshot directory',exact=True).click()
            expect(d).to_contain_text(error,timeout=30000);expect(d.locator('.snapshot-files-state')).to_have_text('Snapshot file query failed')
            expect(d.locator('.snapshot-files-summary')).to_have_text(old);expect(d.locator('.snapshot-file-row')).to_have_count(4)
            expect(d.get_by_role('button',name='Restore selected…',exact=True)).to_be_disabled();assert 'fixture-admin-secret' not in d.inner_text() and 'arcnix-test' not in d.inner_text()
            (a.root/flag).unlink();directory(d,'/srv/site')
        select(d,'/srv/site/config[prod]*?.txt');select(d,'/srv/site/assets')
        d.locator('.snapshot-file-row').filter(has=d.page.get_by_text('/srv/site/many',exact=True)).get_by_role('button',name='Open directory',exact=True).click()
        expect(d.locator('.snapshot-file-row')).to_have_count(100,timeout=30000);select(d,'/srv/site/many/entry-000');d.get_by_role('button',name='More files',exact=True).click()
        expect(d.locator('.snapshot-file-row')).to_have_count(5,timeout=30000);select(d,'/srv/site/many/entry-104');d.get_by_role('button',name='Previous files',exact=True).click()
        expect(d.locator('.snapshot-file-row')).to_have_count(100,timeout=30000);expect(d.locator('.snapshot-file-row').filter(has=d.page.get_by_text('/srv/site/many/entry-000',exact=True)).locator('.snapshot-file-select')).to_have_class(re.compile(r'\bactive\b'))
        directory(d,'/srv/site/assets');select(d,'/srv/site/assets/中文.txt')
        expect(d.locator('.snapshot-selected-paths')).to_contain_text('Selected snapshot paths · 5');app.screenshot('snapshot-selection.png')
        d.get_by_role('button',name='Restore selected…',exact=True).click();d=p.get_by_role('dialog',name='Restore snapshot',exact=True)
        expect(d.locator('.snapshot-restore-selection')).to_contain_text('/srv/site/config[prod]*?.txt');d.get_by_role('textbox',name='New restore directory',exact=True).fill('/srv/selected-restore');d.get_by_role('button',name='Review restore…',exact=True).click()
        (b.root/'snapshot-selection-fail').touch();prepare(app);expect(p.locator('.task-status')).to_have_text(['Ready','Failed'],timeout=30000)
        expect(p.locator('.task-target').first).to_contain_text('Literal snapshot paths');expect(p.locator('.task-target').first).not_to_contain_text('base64')
        p.get_by_role('button',name='Stop pending targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Cancelled','Failed']);(b.root/'snapshot-selection-fail').unlink()
        d=load(app);select(d,'/srv/site/config[prod]*?.txt');d.get_by_role('button',name='Restore selected…',exact=True).click();d=p.get_by_role('dialog',name='Restore snapshot',exact=True)
        d.get_by_role('textbox',name='New restore directory',exact=True).fill('/srv/selected-final');d.get_by_role('button',name='Review restore…',exact=True).click();prepare(app)
        expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=30000);app.screenshot('selected-restore-review.png')
        (b.root/'task-launch-delay').touch();p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status').nth(1)).to_have_text('Submitting',timeout=20000);expect(p.locator('.task-status')).to_have_text(['Running','Running'],timeout=20000);(b.root/'task-launch-delay').unlink()
        app.stop();time.sleep(4.5);app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        expect(p.locator('.task-status')).to_have_text(['Result unknown','Result unknown'])
        for index in range(2):
            p.locator('.task-target').nth(index).get_by_role('button',name='Query server result',exact=True).click();q=p.get_by_role('dialog',name='Query server result',exact=True);q.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');q.get_by_role('button',name='Query',exact=True).click();expect(p.locator('.task-status').nth(index)).to_have_text('Succeeded',timeout=20000)
        stored=json.loads(app.config.read_text());confirmations=[e for e in stored['config']['backup_evidence'] if e['kind']=='FileRestore'];assert len(confirmations)==2
        assert all(e['last_success']['paths']==['/srv/site/config[prod]*?.txt'] and e['last_success']['origin_server']=='Alpha' for e in confirmations)
        p.locator('.rail-backups').click();expect(p.locator('.backup-confirmations')).to_contain_text('Selected paths: 1')
        d=load(app);d.get_by_role('button',name='Back to snapshots',exact=True).click();d=p.get_by_role('dialog',name='Backup snapshots',exact=True);d.get_by_role('button',name='Beta',exact=True).click()
        expect(d.locator('.backup-snapshot')).to_have_count(0);expect(d.get_by_role('textbox',name='Administrator password (optional)',exact=True)).to_have_value('')
        d.get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret');d.get_by_role('button',name='Load snapshots',exact=True).click();expect(d.locator('.backup-snapshot')).to_have_count(1);d.get_by_role('button',name='Browse files…',exact=True).click();d=p.get_by_role('dialog',name='Snapshot files',exact=True)
        expect(d.locator('.snapshot-files-state')).to_have_text('Snapshot file query completed',timeout=30000);expect(d.locator('.snapshot-selected-paths')).to_have_text('Selected snapshot paths · 0\n')
        d.get_by_role('button',name='Forget backup access',exact=True).click();expect(d.locator('.snapshot-file-row')).to_have_count(0);expect(d.get_by_role('button',name='Open snapshot directory',exact=True)).to_be_disabled();d.get_by_role('button',name='Close',exact=True).click()
        d=load(app);(a.root/'snapshot-files-delay').touch();d.get_by_role('button',name='Open snapshot directory',exact=True).click();expect(d.locator('.snapshot-files-state')).to_have_text('Reading snapshot files…');d.get_by_role('button',name='Close',exact=True).click();time.sleep(1.5);(a.root/'snapshot-files-delay').unlink();expect(p.locator('.snapshot-file-row')).to_have_count(0)
        d=load(app);p.set_viewport_size(dict(width=900,height=700));assert p.evaluate('document.documentElement.scrollWidth <= innerWidth');app.screenshot('snapshot-files-narrow.png');d.get_by_role('button',name='Close',exact=True).click()
        app.settings('General');p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check();expect(p.locator('.page-header-title')).to_have_text('常规');p.locator('.rail-backups').click();p.get_by_role('button',name='浏览快照…',exact=True).click();d=p.get_by_role('dialog',name='备份快照',exact=True)
        d.get_by_role('textbox',name='SSH 密码或私钥口令',exact=True).fill('arcnix-test');d.get_by_role('textbox',name='管理员密码（可选）',exact=True).fill('fixture-admin-secret');d.get_by_role('button',name='加载快照',exact=True).click();expect(d.locator('.backup-snapshot')).to_have_count(1);d.get_by_role('button',name='浏览文件…',exact=True).click();d=p.get_by_role('dialog',name='快照文件',exact=True);expect(d.locator('.snapshot-files-state')).to_have_text('快照文件查询完成',timeout=30000);app.screenshot('snapshot-files-chinese.png');assert p.evaluate('document.documentElement.scrollWidth <= innerWidth')
        raw=app.config.read_text()+app.config.with_suffix('.tasks.json').read_text();assert 'fixture-admin-secret' not in raw and 'arcnix-test' not in raw and '/srv/site/many/entry-001' not in raw

checks=['global backup entry','password sudo without SFTP','bound snapshot file metadata','unsupported node cannot be selected','wrong snapshot rejected','cross-directory response rejected','truncated response rejected','query error redaction','failed query preserves old page and disables restore','100-plus-5 pagination','cross-page selection survives','directory and Unicode path selection','selected paths reviewed literally','multi-server independent preflight failure','cancel leaves ready targets unsubmitted','fresh credentials for restore','server-owned selected restore','desktop restart queries without replay','success evidence retains exact paths and origin','server change revokes prior selection','forget access removes private detail','late result after close ignored','narrow layout','Chinese file browser','credentials and unselected inventory not persisted']
(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),indent=2));print(f'{len(checks)} snapshot file UI checks passed')
