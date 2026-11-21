"""Two explicitly reviewed server-owned residual operations on owned mock data."""
import json,re,time
from desktop_fixture import Peer,Desktop,expect,ROOT

out=ROOT/'artifacts/record-recovery-ui'
def seed(peer,name,legacy=False):
    (peer.root/'cleanup-reports-fixture.json').write_text(json.dumps([dict(id='source-'+name,modified=1700000000,phase='interrupted',scope='task',selected_count=1,processed_count=1,removed_files=0,pending_files=1,service='failed',issue=None,partial=True,legacy=legacy)]))
def connect_reports(app):
    p=app.page;p.locator('.rail-tasks').click();p.get_by_role('button',name='Cleanup reports',exact=True).click()
    p.wait_for_function('() => document.querySelector(\'dialog,[role="dialog"]\') || document.querySelector(".cleanup-report-group")')
    d=p.get_by_role('dialog',name='Connect to cleanup reports',exact=True)
    if not d.count():p.get_by_role('button',name='Connect to cleanup reports…',exact=True).click()
    expect(d).to_be_visible();expect(d.locator('.cleanup-report-target')).to_have_count(2)
    for target in d.locator('.cleanup-report-target').all():
        if 'active' not in (target.get_attribute('class') or '').split():target.click()
    d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test');d.get_by_role('textbox',name='Administrator password (optional)',exact=True).first.fill('fixture-admin-secret');d.get_by_role('button',name='Read cleanup reports',exact=True).click()
    expect(p.locator('.cleanup-report-group')).to_have_count(2)
    for group in p.locator('.cleanup-report-group').all():
        expect(group.locator('.cleanup-query-state')).to_have_text('Cleanup report query completed',timeout=30000)
        group.get_by_role('button',name='Inspect cleanup report',exact=True).click();expect(group.locator('.cleanup-detail-body')).to_contain_text('unlink pending',timeout=30000)
def authenticate_plan(p,count):
    d=p.get_by_role('dialog',name='Run script on servers',exact=True);expect(d.locator('.target-choice.active')).to_have_count(count)
    if d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.count():
        expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first).to_have_value('')
    if d.locator('.target-choice').first.inner_text().startswith('Alpha'):
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test');d.get_by_role('textbox',name='Administrator password (optional)',exact=True).first.fill('fixture-admin-secret')
    d.get_by_role('button',name='Check targets',exact=True).click()

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    seed(a,'alpha');seed(b,'beta',legacy=True)
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    with Desktop(out/f'desktop-{time.time_ns()}',config) as app:
        p=app.page;connect_reports(app)
        export=p.get_by_role('button',name='Review residual exports…',exact=True);remove=p.get_by_role('button',name='Review quarantine removal…',exact=True)
        expect(export).to_be_enabled();expect(remove).to_be_disabled()
        export.click();d=p.get_by_role('dialog',name='Run script on servers',exact=True);expect(d.locator('.target-choice')).to_have_count(1);expect(d.locator('.target-choice')).to_contain_text('Alpha');d.get_by_role('button',name='Cancel',exact=True).click()
        assert not app.config.with_suffix('.tasks.json').exists()
        seed(b,'beta',legacy=False);beta=p.locator('.cleanup-report-group').filter(has=p.get_by_text('Beta',exact=True));beta.get_by_role('button',name='Refresh report details',exact=True).click();expect(beta.locator('.cleanup-detail-body')).to_contain_text('identity is proven',timeout=30000)
        (b.root/'recovery-preflight-denied').touch();export.click();authenticate_plan(p,2)
        expect(p.locator('.task-status')).to_have_text(['Ready','Failed'],timeout=30000);expect(p.locator('.task-target').last).to_contain_text('Residual evidence changed')
        p.get_by_role('button',name='Stop pending targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Cancelled','Failed'])
        (b.root/'recovery-preflight-denied').unlink();connect_reports(app)
        p.get_by_role('button',name='Review residual exports…',exact=True).click();authenticate_plan(p,2)
        expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=30000)
        for target in p.locator('.task-target').all():
            expect(target).to_contain_text('Preserve the source quarantine');expect(target).to_contain_text('2 residual files');expect(target).not_to_contain_text('-source/output ·')
        app.screenshot('export-review.png')
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Running','Running'],timeout=20000)
        app.stop();time.sleep(4.5);app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        expect(p.locator('.task-status')).to_have_text(['Result unknown','Result unknown'])
        for index in range(2):
            p.locator('.task-target').nth(index).get_by_role('button',name='Query server result',exact=True).click();d=p.get_by_role('dialog',name='Query server result',exact=True)
            d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');d.get_by_role('button',name='Query',exact=True).click();expect(p.locator('.task-status').nth(index)).to_have_text('Succeeded',timeout=20000)
        export_id=json.loads(app.config.with_suffix('.tasks.json').read_text())[0]['id']
        for peer in [a,b]:assert json.loads((peer.root/'cleanup-reports-fixture.json').read_text())[0]['partial']
        connect_reports(app)
        for group in p.locator('.cleanup-report-group').all():
            copy=group.locator('.residual-recovery-row').filter(has=p.get_by_text('Residual export · '+export_id,exact=True));expect(copy.locator('.residual-recovery-state')).to_contain_text('completed')
            copy.get_by_role('button',name='Use this verified export',exact=True).click()
        expect(p.get_by_role('button',name='Review quarantine removal…',exact=True)).to_be_enabled();app.screenshot('verified-exports.png')
        p.get_by_role('button',name='Review quarantine removal…',exact=True).click();authenticate_plan(p,2)
        expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=30000)
        for target in p.locator('.task-target').all():expect(target).to_contain_text('Exported copies remain intact');expect(target).to_contain_text('No recursive deletion')
        (b.root/'recovery-fail-runtime').touch();p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Succeeded','Failed'],timeout=20000)
        assert json.loads((a.root/'cleanup-reports-fixture.json').read_text())[0]['released']
        assert json.loads((b.root/'cleanup-reports-fixture.json').read_text())[0]['partial']
        (b.root/'recovery-fail-runtime').unlink();connect_reports(app)
        alpha_group=p.locator('.cleanup-report-group').filter(has=p.get_by_text('Alpha',exact=True));beta=p.locator('.cleanup-report-group').filter(has=p.get_by_text('Beta',exact=True))
        for choice in alpha_group.locator('.residual-release-select').all():expect(choice).to_be_disabled()
        beta.locator('.residual-recovery-row').filter(has=p.get_by_text('Residual export · '+export_id,exact=True)).get_by_role('button',name='Use this verified export',exact=True).click()
        p.get_by_role('button',name='Review quarantine removal…',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        expect(d.locator('.target-choice')).to_have_count(1);expect(d.locator('.target-choice')).to_contain_text('Beta');authenticate_plan(p,1)
        expect(p.locator('.task-status')).to_have_text(['Ready'],timeout=30000);p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Succeeded'],timeout=20000)
        assert json.loads((b.root/'cleanup-reports-fixture.json').read_text())[0]['released']
        history=app.config.with_suffix('.tasks.json').read_text();assert all(secret not in history for secret in ['arcnix-test','fixture-admin-secret','export_manifest_identity','runner_sha256','_RECOVERY_SOURCE'])
        app.settings('General');p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check();expect(p.locator('.page-header-title')).to_have_text('常规');p.locator('.rail-tasks').click();p.get_by_role('button',name='清理报告',exact=True).click()
        expect(p.get_by_role('button',name='审核残留导出…',exact=True)).to_be_disabled();expect(p.get_by_role('button',name='审核暂存文件移除…',exact=True)).to_be_disabled()
        p.set_viewport_size(dict(width=900,height=700));assert p.evaluate('document.documentElement.scrollWidth <= innerWidth');app.screenshot('recovery-chinese.png')

checks=['proven residuals required','unproven legacy source excluded','cancel creates no task','fresh credentials for export','independent preflight failure','cancel prepared export before execution','multi-server exact residual review','already missing files excluded','explicit export submission','desktop exit does not stop exports','restart queries without replay','exports preserve both quarantines','verified export selection is separate','explicit checksum/removal review','independent removal failure retains its source','new review retries only remaining server','source resolution leaves exported evidence','no credentials or frozen proof persistence','navigation revokes authority','Chinese narrow layout']
(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),indent=2));print(f'{len(checks)} residual recovery UI checks passed')
