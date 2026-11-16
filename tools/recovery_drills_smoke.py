"""On-demand file/PG recovery acceptance, restart queries and independent reports."""
import json,os,time
from desktop_fixture import Peer,Desktop,expect,ROOT

out=ROOT/'artifacts/recovery-drills-ui';checks=[]
def passed(name):checks.append(name)
def card(p,name):return p.locator('.backup-policy').filter(has=p.get_by_text(name,exact=True))
def open_drill(app,name='Website'):
    p=app.page;p.locator('.rail-backups').click()
    card(p,name).get_by_role('button',name='Browse snapshots…',exact=True).click()
    d=p.get_by_role('dialog',name='Backup snapshots',exact=True)
    d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test')
    d.get_by_role('button',name='Load snapshots',exact=True).click()
    expect(d.locator('.backup-snapshot')).to_have_count(1,timeout=20000)
    d.get_by_role('button',name='Run recovery drill…',exact=True).click()
    return p.get_by_role('dialog',name='PostgreSQL recovery drill' if name=='Postgres' else 'Recovery drill',exact=True)
def file_input(d,destination='/restore/rehearsal',path='/srv/site/index.html'):
    d.get_by_role('textbox',name='New restore directory',exact=True).fill(destination)
    d.get_by_role('textbox',name='Snapshot file path',exact=True).first.fill(path)
def prepare(app,d,two=False):
    d.get_by_role('button',name='Review recovery drill…',exact=True).click()
    p=app.page;d=p.get_by_role('dialog',name='Run script on servers',exact=True)
    expect(d.locator('.target-choice')).to_have_count(2)
    expect(d.locator('.target-choice.active')).to_have_count(1)
    if two:d.locator('.target-choice').nth(1).click()
    d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test')
    d.get_by_role('textbox',name='Administrator password (optional)',exact=True).first.fill('fixture-admin-secret')
    expect(d).not_to_contain_text('zlib.decompress')
    d.get_by_role('button',name='Check targets',exact=True).click()
    expect(p.locator('.task-status').first).to_have_text('Ready',timeout=30000)
    if two:expect(p.locator('.task-status')).to_have_text(['Ready','Ready'])
    return p
def query(app,index=0):
    p=app.page;p.locator('.task-target').nth(index).get_by_role('button',name='Query server result',exact=True).click()
    d=p.get_by_role('dialog',name='Query server result',exact=True)
    if index==0:d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test')
    d.get_by_role('button',name='Query',exact=True).click()
    expect(p.locator('.task-status').nth(index)).to_have_text('Succeeded',timeout=20000)

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta'),b.profile('Outside policy')],out)
    host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    site=dict(id='a1-b1',name='Website',repository='/backups/site',password_file='/secrets/restic',source='/srv/site',servers=['Alpha','Beta'],administrator=False,excludes=[],keep_last=7)
    database=dict(id='a2-b2',name='Postgres',repository='/backups/pg',password_file='/secrets/restic',source='/staging',servers=['Alpha','Beta'],administrator=False,excludes=[],keep_last=7,
        database=dict(engine='PostgreSql',connection=dict(host='127.0.0.1',port=5432,name='app',username='backup',auth_file='/secrets/pgpass')))
    mysql=json.loads(json.dumps(database));mysql.update(id='a3-b3',name='MySQL',repository='/backups/mysql')
    mysql['database'].update(engine='MySql');mysql['database']['connection'].update(port=3306,auth_file='/secrets/mysql')
    config['config']['backup_policies']=[site,database,mysql]
    with Desktop(out/f'desktop-{time.time_ns()}',config) as app:
        p=app.page
        assert 'rail-servers' in p.locator('.navigation-rail button').first.get_attribute('class')
        d=open_drill(app);expect(d).to_contain_text('complete snapshot')
        expect(d).to_contain_text('Cleanup is a separate manual decision')
        d.get_by_role('textbox',name='New restore directory',exact=True).fill('/restore/checks')
        d.get_by_role('button',name='Review recovery drill…',exact=True).click()
        expect(d).to_contain_text('canonical absolute snapshot path')
        file_input(d,path='/outside/file')
        d.get_by_role('button',name='Review recovery drill…',exact=True).click();expect(d).to_contain_text('outside the snapshot scope')
        file_input(d);d.get_by_role('textbox',name='Minimum bytes',exact=True).fill('-1')
        d.get_by_role('button',name='Review recovery drill…',exact=True).click();expect(d).to_contain_text('nonnegative minimum')
        d.get_by_role('textbox',name='Minimum bytes',exact=True).fill('1')
        d.get_by_role('textbox',name='Expected SHA-256 (optional)',exact=True).fill('bad')
        d.get_by_role('button',name='Review recovery drill…',exact=True).click();expect(d).to_contain_text('optional SHA-256')
        d.get_by_role('textbox',name='Expected SHA-256 (optional)',exact=True).fill('a'*64)
        passed('typed required files reject empty paths scope escapes negative counts and bad hashes')
        d.get_by_role('button',name='Add acceptance check',exact=True).click()
        d.get_by_role('textbox',name='Snapshot file path',exact=True).nth(1).fill('/srv/site/empty')
        d.get_by_role('textbox',name='Minimum bytes',exact=True).nth(1).fill('0')
        for _ in range(10):d.get_by_role('button',name='Add acceptance check',exact=True).click()
        expect(d.locator('.drill-check-editor')).to_have_count(12);expect(d.get_by_role('button',name='Add acceptance check',exact=True)).to_be_disabled()
        for _ in range(10):d.get_by_role('button',name='Remove check',exact=True).last.click()
        expect(d.get_by_role('textbox',name='Snapshot file path',exact=True).first).to_have_value('/srv/site/index.html')
        expect(d.get_by_role('textbox',name='Snapshot file path',exact=True).nth(1)).to_have_value('/srv/site/empty')
        passed('acceptance editor bounds count and preserves keyed values on removal')
        app.screenshot('file-acceptance-form.png');p=prepare(app,d,True)
        expect(p.locator('.task-target').first).to_contain_text('Acceptance checks')
        expect(p.locator('.task-target').first).to_contain_text('restore --verify')
        expect(p.locator('.task-target').first).not_to_contain_text('ARCNIX_DRILL_REPORT')
        history=app.config.with_suffix('.tasks.json');before=json.loads(history.read_text());identifier=before[0]['id']
        assert before[0]['backup']['kind']=='RecoveryDrill' and len(before[0]['drill']['checks'])==2
        passed('two-target preflight freezes snapshot checks and restricts policy scope without SFTP')
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Running','Running'],timeout=20000)
        app.stop();time.sleep(4.5);app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        expect(p.locator('.task-status')).to_have_text(['Result unknown','Result unknown']);query(app,0);query(app,1)
        for report in p.locator('.drill-report').all():
            expect(report).to_contain_text('Recovery acceptance checks passed')
            expect(report).to_contain_text('Source server: Alpha')
            expect(report).to_contain_text('Minimum bytes:')
        expect(p.locator('.task-output').first).not_to_contain_text('ARCNIX_DRILL_REPORT')
        p.locator('.drill-report').first.scroll_into_view_if_needed();app.screenshot('recovered-file-drill-report.png')
        records=json.loads(app.config.read_text())['config']['backup_evidence']
        successes={r['server']:r['last_success']['task'] for r in records if r['kind']=='RecoveryDrill' and r['policy']==site['id']}
        assert successes==dict(Alpha=identifier,Beta=identifier)
        assert all(r['last_success']['drill_checks']==2 for r in records)
        passed('server-owned drill and separate scoped confirmations survive desktop restart')
        app.stop();history.unlink();app.launch();p=app.page;p.locator('.rail-tasks').click()
        p.get_by_role('button',name='Server records',exact=True).click();d=p.get_by_role('dialog',name='Connect to server records',exact=True)
        d.get_by_role('button',name='Select trusted servers',exact=True).click()
        d.locator('.record-target-picker').nth(2).get_by_role('button').first.click()
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test')
        d.get_by_role('button',name='Read server records',exact=True).click()
        expect(p.locator('.server-record-group')).to_have_count(2,timeout=20000)
        group=p.locator('.server-record-group').first
        group.locator('.server-task-record').filter(has=p.get_by_text(identifier,exact=True)).get_by_role('button',name='View saved log',exact=True).click()
        d=p.get_by_role('dialog',name='Saved server task log',exact=True)
        expect(d.locator('.drill-report')).to_contain_text('Recovery acceptance checks passed',timeout=20000)
        expect(d.locator('.record-detail-status')).to_have_text('Succeeded')
        d.get_by_role('button',name='Close',exact=True).click()
        passed('independent server drill report remains after all local task history is deleted')
        for mode,status,title in [
            ('check-fail','Failed','Restore completed; acceptance checks did not all pass'),
            ('restore-fail','Failed','Recovery drill did not complete successfully'),
            ('incomplete','Result unknown','Recovery drill proof is incomplete or inconsistent'),
            ('wrong-task','Result unknown','Recovery drill proof is incomplete or inconsistent'),
            ('wrong-server','Result unknown','Recovery drill proof is incomplete or inconsistent'),
            ('interrupted','Failed','Recovery drill was interrupted'),
        ]:
            d=open_drill(app);file_input(d,'/restore/'+mode);p=prepare(app,d)
            flag=a.root/('drill-'+mode);flag.touch()
            p.get_by_role('button',name='Execute ready targets',exact=True).click()
            expect(p.locator('.task-status').first).to_have_text(status,timeout=20000)
            expect(p.locator('.task-target').first.locator('.drill-report')).to_contain_text(title)
            flag.unlink()
            records=json.loads(app.config.read_text())['config']['backup_evidence']
            current=next(r for r in records if r['kind']=='RecoveryDrill' and r['policy']==site['id'] and r['server']=='Alpha')
            assert current['last_success']['task']==identifier and current['latest']['status']==status.replace('Result unknown','Unknown')
            passed(mode+' never replaces the last successful drill')
            if mode in ['incomplete','wrong-server']:
                missing_id=json.loads(history.read_text())[0]['id']
                app.stop();history.unlink();app.launch();p=app.page;p.locator('.rail-tasks').click()
                p.get_by_role('button',name='Server records',exact=True).click()
                d=p.get_by_role('dialog',name='Connect to server records',exact=True)
                d.locator('.record-target-picker').first.get_by_role('button').first.click()
                d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test')
                d.get_by_role('button',name='Read server records',exact=True).click()
                row=p.locator('.server-task-record').filter(has=p.get_by_text(missing_id,exact=True))
                row.get_by_role('button',name='View saved log',exact=True).click()
                d=p.get_by_role('dialog',name='Saved server task log',exact=True)
                expect(d.locator('.record-detail-status')).to_have_text('Result unknown',timeout=20000)
                report=d.locator('.drill-report')
                expect(report).to_contain_text('Recovery drill request; no execution proof')
                expect(report).to_contain_text('/restore/'+mode)
                expect(report).to_contain_text('Source server: Alpha')
                expect(report).to_contain_text('Execution server: Alpha')
                expect(report).to_contain_text('/srv/site/index.html')
                expect(report).to_contain_text('Minimum bytes: 1')
                expect(report).not_to_contain_text('Recovery acceptance checks passed')
                if mode=='incomplete':app.screenshot('missing-report-saved-request.png')
                assert not history.exists(), 'Reading a saved request must not create or resume a task'
                d.get_by_role('button',name='Close',exact=True).click()
                passed(mode+' independently recovers frozen scope without local history or false success')
        p.locator('.rail-backups').click();card(p,'Postgres').get_by_role('button',name='Back up now',exact=True).click()
        d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        # The source backup seeds the fixture's logical PostgreSQL snapshot tag.
        d.locator('.target-choice').nth(1).click();d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test')
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status').first).to_have_text('Ready',timeout=20000)
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status').first).to_have_text('Succeeded',timeout=20000)
        d=open_drill(app,'Postgres');expect(d.get_by_role('textbox',name='Snapshot file path',exact=True)).to_have_count(0)
        d.get_by_role('textbox',name='New database name',exact=True).fill('drill_app')
        d.get_by_role('textbox',name='Table name',exact=True).fill('records;DROP')
        d.get_by_role('button',name='Review recovery drill…',exact=True).click();expect(d).to_contain_text('ASCII letters')
        d.get_by_role('textbox',name='Table name',exact=True).fill('records')
        d.get_by_role('textbox',name='Minimum rows',exact=True).fill('1')
        app.screenshot('postgres-acceptance-form.png');p=prepare(app,d)
        expect(p.locator('.task-target').first).to_contain_text('createdb')
        expect(p.locator('.task-target').first).to_contain_text('public.records')
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status').first).to_have_text('Succeeded',timeout=20000)
        expect(p.locator('.task-target').first.locator('.drill-report')).to_contain_text('Rows: 3')
        expect(p.locator('.task-target').first.locator('.drill-report')).to_contain_text('postgresql://backup@127.0.0.1:5432/drill_app')
        passed('PostgreSQL drill restores a new database and checks explicit table counts without free SQL')
        records=json.loads(app.config.read_text())['config']['backup_evidence']
        pg=next(r for r in records if r['kind']=='RecoveryDrill' and r['policy']==database['id'])
        assert pg['last_success']['drill_checks']==1
        assert not any(r['kind']=='DatabaseRestore' for r in records)
        passed('database drill success is separate from ordinary database restore confirmation')
        p.locator('.rail-backups').click();card(p,'MySQL').get_by_role('button',name='Browse snapshots…',exact=True).click()
        d=p.get_by_role('dialog',name='Backup snapshots',exact=True);d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test')
        d.get_by_role('button',name='Load snapshots',exact=True).click();expect(d.locator('.backup-snapshot')).to_have_count(1)
        expect(d.get_by_role('button',name='Run recovery drill…',exact=True)).to_have_count(0)
        d.get_by_role('button',name='Close',exact=True).click();passed('unsupported MySQL recovery drill is not offered')
        d=open_drill(app);file_input(d,'/restore/narrow')
        p.set_viewport_size(dict(width=900,height=700));assert p.evaluate('document.documentElement.scrollWidth <= innerWidth')
        box=d.get_by_role('button',name='Review recovery drill…',exact=True).bounding_box();assert box and box['y']+box['height']<=700
        app.screenshot('drill-form-narrow.png');d.get_by_role('button',name='Cancel',exact=True).click()
        app.settings('General');p.get_by_role('button',name='简体中文',exact=True).click();expect(p.locator('.page-header-title')).to_have_text('常规')
        p.locator('.rail-tasks').click();p.get_by_role('button',name='最近批次',exact=True).click();p.locator('.task-heading').first.click()
        expect(p.locator('.task-target').first.locator('.drill-report')).to_contain_text('恢复验收检查通过')
        app.screenshot('drill-report-chinese.png');passed('Chinese report and narrow form retain visible footer and wrapping')
        assert not any(s in history.read_text() for s in ['arcnix-test','fixture-admin-secret'])
        passed('task definitions reports and confirmation indexes contain no SSH or sudo credentials')
if os.environ.get('ARCNIX_TEST_TRACE_INPUT')=='1':
    routes=[json.loads(line.split(' ',1)[1]) for line in (app.out/'desktop.log').read_text(encoding='utf-8').splitlines() if line.startswith('ARCNIX_NAVIGATION ')]
    leaves=[event for event in routes if event['to']!='Backups' and event['backup_access_before']]
    assert leaves and all(not event['backup_access_after'] for event in leaves),leaves
    assert any(event['to']=='Tasks' and event['source']=='prepare-script' for event in leaves)
    passed('Loaded backup access is cleared by task-preparation navigation')
(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),ensure_ascii=False,indent=2),encoding='utf-8')
print(f'{len(checks)} recovery drill desktop groups passed')
