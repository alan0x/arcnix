"""Separate audit retirement review, frozen targets and server-owned outcomes."""
import json,time
from desktop_fixture import Peer,Desktop,expect,ROOT

out=ROOT/'artifacts/audit-cleanup-ui'/str(time.time_ns());checks=[]
def passed(text):checks.append(text);print(text,flush=True)
def seed():
    rows=[dict(id=f'audit-{n:04}',modified=1700000000+n) for n in range(5)]
    rows[2]['reasons']=['referenced'];rows[3]['reasons']=['residuals']
    return dict(cleanup=rows,recovery=[dict(id='export-copy',modified=1700000000,action='export',source='audit-0002',reasons=['export_data','referenced'])])
def state(peer):return json.loads((peer.root/'audit-storage-fixture.json').read_text())
def connect(app,fresh=False):
    p=app.page;p.locator('.rail-tasks').click();p.get_by_role('button',name='Cleanup reports',exact=True).click()
    if not fresh:p.get_by_role('button',name='Connect to cleanup reports…',exact=True).click()
    d=p.get_by_role('dialog',name='Connect to cleanup reports',exact=True);expect(d).to_be_visible()
    if fresh:
        expect(d.locator('.cleanup-report-target')).to_have_count(3)
        for i in range(2):d.locator('.cleanup-report-target').nth(i).click()
    else:expect(d.locator('.cleanup-report-target.active')).to_have_count(2)
    expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True)).to_have_count(2)
    d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test')
    for box in d.get_by_role('textbox',name='Administrator password (optional)',exact=True).all():box.fill('fixture-admin-secret')
    d.get_by_role('button',name='Read cleanup reports',exact=True).click();expect(d).not_to_be_visible()
    expect(p.locator('.cleanup-query-state')).to_have_text(['Cleanup report query completed']*2)
    p.get_by_role('button',name='Audit and export storage…',exact=True).click()
    expect(p.locator('.audit-storage-state')).to_have_text(['Audit storage query completed']*2)
    p.get_by_role('textbox',name='Minimum audit age (days)',exact=True).fill('0');p.get_by_role('textbox',name='Keep latest audit records',exact=True).fill('1')
    return p
def preview(p):
    for group in p.locator('.audit-storage-group').all():
        group.get_by_role('button',name='Clear audit selection',exact=True).click()
        for name in ['audit-0000','audit-0001','audit-0002','audit-0003','audit-0004']:
            if group.get_by_role('button',name=name,exact=True).count():group.get_by_role('button',name=name,exact=True).click()
        group.get_by_role('button',name='Preview selected audit records',exact=True).click()
    expect(p.locator('.audit-preview-summary')).to_contain_text(['Retention candidates: 2']*2)
def review(p):
    p.get_by_role('button',name='Review audit cleanup…',exact=True).click()
    d=p.get_by_role('dialog',name='Run script on servers',exact=True);expect(d).to_be_visible()
    expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True)).to_have_count(2)
    for box in d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).all():expect(box).to_have_value('')
    expect(d.locator('.target-choice')).to_have_count(2)
    expect(d.locator('.target-choice').filter(has_text='Gamma')).to_have_count(0)
    d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test')
    for box in d.get_by_role('textbox',name='Administrator password (optional)',exact=True).all():box.fill('fixture-admin-secret')
    return d

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b,Peer(out/'peer-c') as c:
    for peer in [a,b,c]:(peer.root/'audit-storage-fixture.json').write_text(json.dumps(seed()))
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta'),c.profile('Gamma')],out)
    for peer in [b,c]:
        host,port=peer.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=peer.data['host_key_rsa_fingerprint']))
    with Desktop(out/'desktop',config) as app:
        p=connect(app,True);expect(p.get_by_role('button',name='Review audit cleanup…',exact=True)).to_be_disabled();preview(p)
        before=[state(a),state(b)];d=review(p);expect(d).to_contain_text('Deletion cannot be undone')
        d.get_by_role('button',name='Cancel',exact=True).click();assert [state(a),state(b)]==before
        assert not app.config.with_suffix('.tasks.json').exists()
        passed('Only verified candidate previews open a separate irreversible review with fresh credentials and pinned original targets; cancellation makes no task')

        p=connect(app);preview(p);(a.root/'audit-cleanup-preflight-denied').touch();d=review(p)
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Failed','Ready'])
        expect(p.locator('.task-target').first).to_contain_text('Audit objects changed')
        p.get_by_role('button',name='Stop pending targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Failed','Cancelled'])
        (a.root/'audit-cleanup-preflight-denied').unlink();assert [state(a)['cleanup'],state(b)['cleanup']]==[before[0]['cleanup'],before[1]['cleanup']]
        passed('Independent stale preflight excludes its target; cancelling a prepared peer never removes audit files')

        p=connect(app);preview(p);(a.root/'audit-cleanup-wrong-request').touch();d=review(p)
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Failed','Ready'])
        expect(p.locator('.task-target').first).to_contain_text('Audit preparation does not match')
        p.get_by_role('button',name='Stop pending targets',exact=True).click();(a.root/'audit-cleanup-wrong-request').unlink()
        passed('A preparation response for another frozen request cannot become ready even when its task identifier matches')

        p=connect(app);preview(p);d=review(p);d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready','Ready'])
        for target in p.locator('.task-target').all():
            for text in ['audit-0000/request.json','audit-0001/runner.py']:
                expect(target).to_contain_text(text)
            assert all(value not in target.inner_text() for value in ['audit-0002/','audit-0003/','audit-0004/'])
        assert [state(a)['cleanup'],state(b)['cleanup']]==[before[0]['cleanup'],before[1]['cleanup']]
        app.screenshot('exact-audit-review.png');passed('Preflight freezes only two exact candidates per server and leaves retained, referenced and residual audits unchanged until execution')
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Running','Running'])
        app.stop();time.sleep(4.5);app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click();expect(p.locator('.task-status')).to_have_text(['Result unknown','Result unknown'])
        p.locator('.task-batch-refresh:visible').click();q=p.get_by_role('dialog',name='Query batch results',exact=True)
        q.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test')
        q.get_by_role('button',name='Query selected results',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Succeeded','Succeeded'])
        for peer in [a,b]:
            assert {r['id'] for r in state(peer)['cleanup']}=={'audit-0002','audit-0003','audit-0004'}
            assert state(peer)['recovery']==before[0]['recovery']
        assert state(c)==seed()
        saved=json.loads(app.config.with_suffix('.tasks.json').read_text());latest=saved[0];assert latest['script_id']=='builtin:audit-cleanup'
        assert 'root_identity' not in json.dumps(saved) and 'references_token' not in json.dumps(saved)
        passed('Accepted multi-server audit cleanup survives client closure and is queried without replay; exported data, protected audits and the unreviewed server remain intact')

        p=connect(app,True);p.get_by_role('button',name='Audit cleanup results',exact=True).click();expect(p.locator('.audit-storage-record')).to_have_count(5)
        for group in p.locator('.audit-storage-group').all():
            for protection in group.locator('.audit-storage-protection').all():expect(protection).to_contain_text('Audit cleanup evidence remains protected')
            expect(group.locator('.audit-storage-record').filter(has=group.page.get_by_role('button',name=latest['id'],exact=True)).locator('.audit-storage-record-size')).to_contain_text('Completed')
        expect(p.get_by_role('button',name='Review audit cleanup…',exact=True)).to_be_disabled()
        app.settings('General');p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check();expect(p.locator('.page-header-title')).to_have_text('常规');p.locator('.rail-tasks').click();p.set_viewport_size(dict(width=900,height=700))
        expect(p.get_by_role('button',name='审计清理结果',exact=True)).to_be_visible()
        p.get_by_role('button',name='连接清理报告…',exact=True).click();d=p.get_by_role('dialog',name='连接清理报告',exact=True)
        expect(d.get_by_role('textbox',name='SSH 密码或私钥口令',exact=True)).to_have_count(2)
        d.get_by_role('textbox',name='SSH 密码或私钥口令',exact=True).first.fill('arcnix-test')
        for box in d.get_by_role('textbox',name='管理员密码（可选）',exact=True).all():box.fill('fixture-admin-secret')
        d.get_by_role('button',name='读取清理报告',exact=True).click();expect(p.locator('.audit-storage-record')).to_have_count(5)
        expect(p.locator('.audit-storage-record-size').filter(has_text='已完成')).to_have_count(2)
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        app.screenshot('audit-cleanup-results-chinese.png')
        assert all(secret not in app.config.read_text()+app.config.with_suffix('.tasks.json').read_text() for secret in ['arcnix-test','fixture-admin-secret'])
        passed('Private cleanup results are discoverable independently and protected from another cleanup; Chinese narrow layout and credential boundaries remain usable')

(out/'results.json').write_text(json.dumps(dict(passed=len(checks),checks=checks),ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps(dict(passed=len(checks),artifacts=str(out)),ensure_ascii=False),flush=True)
