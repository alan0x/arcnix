"""Reviewed multi-server cleanup handoff, with owned mock storage only."""
import json,time
from desktop_fixture import Peer,Desktop,expect,ROOT

out=ROOT/'artifacts/record-cleanup-ui'
with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    for peer,name in [(a,'alpha'),(b,'beta')]:
        (peer.root/'record-storage-fixture.json').write_text(json.dumps(dict(task=[dict(id=name+'-candidate',title=name+' old record',modified=1700000000),dict(id=name+'-active',title=name+' active record',modified=1700000001,service='active')],configuration=[])))
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta'),b.profile('Unselected')],out)
    host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    with Desktop(out/f'desktop-{time.time_ns()}',config) as app:
        p=app.page;p.locator('.rail-tasks').click();p.get_by_role('button',name='Record storage',exact=True).click()
        d=p.get_by_role('dialog',name='Inspect record storage',exact=True)
        d.get_by_role('textbox',name='Minimum record age (days)',exact=True).fill('0');d.get_by_role('textbox',name='Keep latest records (per path for configuration originals)',exact=True).fill('0')
        d.locator('.record-target').nth(0).click();d.locator('.record-target').nth(1).click()
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test');d.get_by_role('textbox',name='Administrator password (optional)',exact=True).first.fill('fixture-admin-secret')
        d.get_by_role('button',name='Inspect storage',exact=True).click()
        groups=p.locator('.storage-server-group');expect(groups).to_have_count(2)
        for group in groups.all():expect(group.locator('.storage-state')).to_have_text('Storage query completed',timeout=30000)
        review=p.get_by_role('button',name='Review cleanup on servers…',exact=True);expect(review).to_be_disabled()
        for group in groups.all():
            for row in group.locator('.storage-record').all():row.get_by_role('button',name='Select record',exact=True).click()
            group.get_by_role('button',name='Preview selected records',exact=True).click();expect(group.locator('.storage-preview-summary')).to_contain_text('2 selected · 1 candidates',timeout=30000)
        expect(review).to_be_enabled();review.click()
        d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        expect(d).to_contain_text('irreversible server cleanup');expect(d.locator('.target-choice')).to_have_count(2);expect(d.locator('.target-choice.active')).to_have_count(2)
        expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first).to_have_value('')
        d.get_by_role('button',name='Cancel',exact=True).click();expect(p.locator('.task-entry')).to_have_count(0)
        assert not app.config.with_suffix('.tasks.json').exists()
        review.click();d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test');d.get_by_role('textbox',name='Administrator password (optional)',exact=True).first.fill('fixture-admin-secret')
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=30000)
        for target in p.locator('.task-target').all():
            expect(target).to_contain_text('No recursive deletion');expect(target).to_contain_text('/record.json');expect(target).not_to_contain_text('-active/output')
        app.screenshot('cleanup-review.png')
        before=[json.loads((peer.root/'record-storage-fixture.json').read_text()) for peer in [a,b]]
        assert all(len(store['task'])==2 for store in before)
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Running','Running'],timeout=20000)
        app.stop();time.sleep(4.5);app.launch()
        p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click();expect(p.locator('.task-status')).to_have_text(['Result unknown','Result unknown'])
        for target in p.locator('.task-target').all():
            target.get_by_role('button',name='Query server result',exact=True).click();d=p.get_by_role('dialog',name='Query server result',exact=True)
            d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');d.get_by_role('button',name='Query',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Succeeded','Succeeded'],timeout=20000)
        expect(p.locator('.task-target').first).to_contain_text('ARCNIX_CLEANUP_V1')
        for peer in [a,b]:
            store=json.loads((peer.root/'record-storage-fixture.json').read_text());assert len(store['task'])==1 and store['task'][0]['service']=='active'
        history_before=app.config.with_suffix('.tasks.json').read_text()
        task_id=json.loads(history_before)[0]['id']
        p.get_by_role('button',name='Cleanup reports',exact=True).click();d=p.get_by_role('dialog',name='Connect to cleanup reports',exact=True)
        d.locator('.cleanup-report-target').nth(0).click();d.locator('.cleanup-report-target').nth(1).click()
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test');d.get_by_role('textbox',name='Administrator password (optional)',exact=True).first.fill('fixture-admin-secret');d.get_by_role('button',name='Read cleanup reports',exact=True).click()
        for group in p.locator('.cleanup-report-group').all():
            expect(group.locator('.cleanup-query-state')).to_have_text('Cleanup report query completed',timeout=30000)
            expect(group.locator('.cleanup-report-id')).to_have_text(task_id)
            group.get_by_role('button',name='Inspect cleanup report',exact=True).click();expect(group.locator('.cleanup-detail-body')).to_contain_text('Saved: removed',timeout=30000)
        assert app.config.with_suffix('.tasks.json').read_text()==history_before
        history=app.config.with_suffix('.tasks.json').read_text();assert all(secret not in history for secret in ['arcnix-test','fixture-admin-secret','runner_sha256','request_sha256'])
        p.get_by_role('button',name='Record storage',exact=True).click();expect(p.get_by_role('button',name='Review cleanup on servers…',exact=True)).to_be_disabled()
        p.get_by_role('dialog',name='Inspect record storage',exact=True).get_by_role('button',name='Cancel',exact=True).click()
        app.settings('General');p.get_by_role('button',name='简体中文',exact=True).click();expect(p.locator('.page-header-title')).to_have_text('常规');p.locator('.rail-tasks').click()
        expect(p.get_by_role('button',name='审核服务器清理…',exact=True)).to_be_visible()
        p.set_viewport_size(dict(width=900,height=700));assert p.evaluate('document.documentElement.scrollWidth <= innerWidth');app.screenshot('cleanup-chinese.png')

checks=['preview required before review','two server candidate-only handoff','protected records excluded','targets pinned to original previews','fresh credentials','cancel does not create task history','preflight does not remove records','exact file review','separate explicit execution','multi-server systemd submission','desktop closure leaves server jobs independent','restart query does not replay cleanup','protected mock records survive','no credentials or frozen hashes in local history','navigation clears preview authority','Chinese narrow layout','produced server audit remains inspectable','reading audits leaves local success history unchanged']
(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),indent=2));print(f'{len(checks)} record cleanup UI checks passed')
