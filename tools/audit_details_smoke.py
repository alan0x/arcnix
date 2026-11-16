"""Read-only audit outcome details in an owned desktop and loopback peers."""
import json,re,time
from desktop_fixture import Peer,Desktop,expect,ROOT

out=ROOT/'artifacts/audit-details-ui'/str(time.time_ns());checks=[]
def passed(message):checks.append(message);print(message,flush=True)
def seed():
    return dict(cleanup=[],recovery=[],maintenance=[dict(id='audit-partial',source='source-0001',modified=1700000002,phase='interrupted',service='failed',reasons=['maintenance']),dict(id='audit-completed',modified=1700000001,phase='completed',reasons=['maintenance']),dict(id='audit-prepared',modified=1700000000,phase='prepared',reasons=['maintenance'])])
def connect(app,fresh=False,chinese=False):
    p=app.page;p.locator('.rail-tasks').click();expect(p.locator('.navigation-rail .active')).to_have_class(re.compile(r'.*\brail-tasks\b.*'));p.get_by_role('button',name='清理报告' if chinese else 'Cleanup reports',exact=True).click()
    if not fresh:p.get_by_role('button',name='连接清理报告…' if chinese else 'Connect to cleanup reports…',exact=True).click()
    d=p.get_by_role('dialog');expect(d).to_be_visible()
    if fresh:
        for target in d.locator('.cleanup-report-target').all():target.click()
    expect(d.locator('.cleanup-report-target.active')).to_have_count(2)
    d.get_by_role('textbox',name='SSH 密码或私钥口令' if chinese else 'SSH password or key passphrase',exact=True).first.fill('arcnix-test')
    for box in d.get_by_role('textbox',name='管理员密码（可选）' if chinese else 'Administrator password (optional)',exact=True).all():box.fill('fixture-admin-secret')
    d.get_by_role('button',name='读取清理报告' if chinese else 'Read cleanup reports',exact=True).click();expect(d).not_to_be_visible()
    expect(p.locator('.cleanup-query-state')).to_have_text(['清理报告查询完成' if chinese else 'Cleanup report query completed']*2)
    p.get_by_role('button',name='审计与导出存储…' if chinese else 'Audit and export storage…',exact=True).click()
    expect(p.locator('.audit-storage-state')).to_have_text(['审计存储查询完成' if chinese else 'Audit storage query completed']*2)
    p.get_by_role('button',name='审计清理结果' if chinese else 'Audit cleanup results',exact=True).click()
    expect(p.locator('.audit-storage-record')).to_have_count(6)
    return p
def inspect(group,name,chinese=False):
    group.locator('.audit-storage-record').filter(has=group.page.get_by_role('button',name=name,exact=True)).get_by_role('button',name='查看审计清理' if chinese else 'Inspect audit cleanup',exact=True).click()
    expect(group.locator('.audit-detail-files')).to_contain_text(name)
def readable(p):
    assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
    assert p.locator('.main-panel').evaluate('e=>e.scrollWidth<=e.clientWidth')
    assert not p.evaluate("""()=>[...document.querySelectorAll('body *')].filter(e=>!e.children.length&&e.textContent.trim()&&e.getClientRects().length&&getComputedStyle(e).visibility!=='hidden'&&parseFloat(getComputedStyle(e).fontSize)>0&&parseFloat(getComputedStyle(e).fontSize)<12).map(e=>e.textContent.slice(0,60))""")

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    for peer in [a,b]:(peer.root/'audit-storage-fixture.json').write_text(json.dumps(seed()))
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta')],out);host,port=b.data['address'].rsplit(':',1)
    config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    with Desktop(out/'desktop',config) as app:
        p=connect(app,True);groups=p.locator('.audit-storage-group');ga=groups.nth(0);gb=groups.nth(1)
        for group in [ga,gb]:
            inspect(group,'audit-partial');body=group.locator('.audit-detail-files')
            for text in ['report.json · Removal recorded','request.json · Removal pending confirmation','runner.py · No removal recorded','Directory identity verified','Matches reviewed file']:expect(body).to_contain_text(text)
        assert not app.config.with_suffix('.tasks.json').exists()
        app.screenshot('partial-details.png');passed('Independent discovery on two servers separates stored unlink acknowledgements, pending confirmation and current residual locations; reads create no task')

        before=ga.locator('.audit-detail-files').inner_text();sample=ga.locator('.audit-detail-summary').inner_text()
        for flag in ['denied','wrong-id','wrong-path','omitted-file']:
            marker=a.root/f'audit-details-{flag}';marker.touch();ga.get_by_role('button',name='Refresh audit cleanup details',exact=True).click()
            expect(ga.locator('.audit-storage-state')).to_have_text('Report query failed; previous snapshots are retained')
            expect(ga.locator('.audit-detail-files')).to_have_text(before);expect(ga.locator('.audit-detail-summary')).to_have_text(sample)
            assert 'fixture-admin-secret' not in ga.inner_text();marker.unlink()
        passed('Failed or malformed refreshes retain the previous receipt and file snapshot; mismatched identity, escaped paths and omitted observations are refused and credentials redacted')

        (a.root/'audit-details-changed').touch();ga.get_by_role('button',name='Refresh audit cleanup details',exact=True).click();expect(ga.locator('.audit-detail-summary')).to_contain_text('Changed during inspection');(a.root/'audit-details-changed').unlink()
        inspect(ga,'audit-prepared');expect(ga.locator('.audit-detail-files')).to_contain_text('No recorded action');expect(ga.locator('.audit-detail-files')).not_to_contain_text('Removal recorded')
        inspect(gb,'audit-completed');expect(gb.locator('.audit-detail-summary')).to_contain_text('Completed');expect(gb.locator('.audit-detail-files')).not_to_contain_text('Removal pending confirmation')
        passed('Changed snapshots remain explicit; prepared and completed records show their own evidence without inferring completion from missing files')

        (a.root/'audit-details-slow').touch();ga.get_by_role('button',name='Refresh audit cleanup details',exact=True).click();expect(ga.locator('.audit-storage-state')).to_have_text('Reading audit storage…');ga.get_by_role('button',name='Close audit cleanup details',exact=True).click()
        time.sleep(2.4);expect(ga.locator('.audit-cleanup-details')).to_have_count(0);expect(gb.locator('.audit-cleanup-details')).to_have_count(1)
        ga.locator('.audit-storage-record').filter(has=p.get_by_role('button',name='audit-partial',exact=True)).get_by_role('button',name='Inspect audit cleanup',exact=True).click();expect(ga.locator('.audit-storage-state')).to_have_text('Reading audit storage…');p.get_by_role('button',name='Forget report credentials',exact=True).click()
        time.sleep(2.4);expect(p.locator('.audit-cleanup-details')).to_have_count(0);(a.root/'audit-details-slow').unlink()
        p=connect(app);ga=p.locator('.audit-storage-group').first;inspect(ga,'audit-partial')
        p.get_by_role('button',name='Cleanup audit storage',exact=True).click();expect(p.locator('.audit-cleanup-details')).to_have_count(0)
        p.get_by_role('button',name='Audit cleanup results',exact=True).click();expect(p.locator('.audit-cleanup-details')).to_have_count(0)
        (a.root/'audit-details-slow').touch();ga.locator('.audit-storage-record').filter(has=p.get_by_role('button',name='audit-partial',exact=True)).get_by_role('button',name='Inspect audit cleanup',exact=True).click();p.locator('.rail-scripts').click();time.sleep(2.4);p.locator('.rail-tasks').click();expect(p.locator('.audit-cleanup-details')).to_have_count(0);(a.root/'audit-details-slow').unlink()
        passed('Closing, forgetting credentials, changing storage scope and leaving the page discard detail state; delayed reads cannot restore closed details or affect the other server')

        app.settings('General');p.get_by_role('button',name='简体中文',exact=True).click();expect(p.locator('.page-header-title')).to_have_text('常规');p.set_viewport_size(dict(width=900,height=700))
        p=connect(app,False,True);ga=p.locator('.audit-storage-group').first;inspect(ga,'audit-partial',True)
        expect(ga.locator('.audit-detail-files')).to_contain_text('移除结果待确认');readable(p)
        tip=ga.locator('.audit-cleanup-details').get_by_role('button',name='帮助：审计清理详情',exact=True)
        tip.hover();expect(p.get_by_role('tooltip')).to_contain_text('仅文件不存在不能证明已删除');box=p.get_by_role('tooltip').bounding_box();assert box and box['x']>=0 and box['x']+box['width']<=900 and box['y']>=0 and box['y']+box['height']<=700
        p.keyboard.press('Escape');tip.click();expect(p.get_by_role('tooltip')).to_be_visible();p.keyboard.press('Escape');expect(p.get_by_role('tooltip')).not_to_be_visible();ga.locator('.audit-cleanup-details').scroll_into_view_if_needed();readable(p);app.screenshot('partial-chinese-narrow.png')
        assert not app.config.with_suffix('.tasks.json').exists()
        for file in app.out.glob('config*.json'):assert 'fixture-admin-secret' not in file.read_text() and 'arcnix-test' not in file.read_text()
        assert all(json.loads((peer.root/'audit-storage-fixture.json').read_text())==seed() for peer in [a,b])
        passed('Chinese 900 × 700 details use readable type and hover/click help without horizontal overflow; no task history, credential persistence, server mutation or retry')
(out/'summary.json').write_text(json.dumps(dict(passed=len(checks),checks=checks),ensure_ascii=False,indent=2),encoding='utf-8');print(out,flush=True)
