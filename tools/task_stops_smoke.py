"""Independent reviewed stop batches; synthetic SSH peers never run shell jobs."""
import json,time
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/task-stops-ui'/str(time.time_ns());checks=[]
def passed(text):checks.append(text);print(text,flush=True)
def row(group,name):return group.locator('.server-task-record').filter(has=group.page.get_by_text(name,exact=True))
def connect(app,fresh=False):
    p=app.page;p.locator('.rail-tasks').click();p.get_by_role('button',name='Server records',exact=True).click()
    if not fresh:p.get_by_role('button',name='Connect to server records…',exact=True).click()
    d=p.get_by_role('dialog',name='Connect to server records',exact=True);expect(d).to_be_visible()
    d.get_by_role('button',name='Select trusted servers',exact=True).click()
    d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test')
    d.get_by_role('button',name='Read server records',exact=True).click();expect(d).not_to_be_visible()
    groups=p.locator('.server-record-group');expect(groups).to_have_count(2)
    expect(groups.first.locator('.server-task-record').first).to_be_visible(timeout=20000)
    expect(groups.last.locator('.server-task-record').first).to_be_visible(timeout=20000)
    return groups
def review(p):
    p.get_by_role('button',name='Review selected stops…',exact=True).click()
    d=p.get_by_role('dialog',name='Stop server tasks',exact=True);expect(d).to_be_visible()
    for box in d.get_by_role('textbox').all():expect(box).to_have_value('')
    d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test')
    for box in d.get_by_role('textbox',name='sudo password (optional)',exact=True).all():box.fill('fixture-admin-secret')
    return d
def history(app):return eventually(lambda:json.loads(app.config.with_suffix('.tasks.json').read_text()))
def launched(peer):
    file=peer.root/'task-launches.jsonl'
    return [json.loads(line)['id'] for line in file.read_text().splitlines()] if file.exists() else []

with Peer(out/'alpha') as a,Peer(out/'beta') as b:
    (a.root/'records-fixture.json').write_text(json.dumps([dict(id=f'alpha-{n:04}',modified=1700000000+n,state='active',code=None,metadata=None,log='Original task output\n') for n in range(103)]))
    (b.root/'records-fixture.json').write_text(json.dumps([dict(id=name,modified=1700000000,state='active',code=None,metadata=None,log='Beta output\n') for name in ['beta-running','foreign-service']]))
    for peer in [a,b]:(peer.root/'trace-task-queries').touch()
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    config['config']['snippets']=[dict(id='hold',name='Unrelated running batch',description='',command='fixture-hold-operation',administrator=False)]
    with Desktop(out/'desktop',config) as app:
        p=app.page
        # Keep a normal batch active while inspecting and submitting stops.
        p.locator('.rail-scripts').click();p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click()
        p.locator('.script-row').filter(has_text='Unrelated running batch').get_by_role('button',name='Run on servers…',exact=True).click()
        d=p.get_by_role('dialog',name='Run script on servers',exact=True);d.locator('.target-choice').first.click();d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');d.get_by_role('button',name='Check targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready'],timeout=20000);p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Running'],timeout=20000)
        original=history(app)[0]['id']
        groups=connect(app,True);alpha_group=groups.first;beta_group=groups.last
        row(alpha_group,'alpha-0102').get_by_role('button',name='Select task',exact=True).click()
        alpha_group.get_by_role('button',name='Older records',exact=True).click();expect(row(alpha_group,'alpha-0000')).to_be_visible(timeout=20000)
        row(alpha_group,'alpha-0000').get_by_role('button',name='Select task',exact=True).click()
        for name in ['beta-running','foreign-service']:row(beta_group,name).get_by_role('button',name='Select task',exact=True).click()
        d=review(p);expect(d.get_by_role('button',name='Stop verified tasks',exact=True)).to_be_disabled()
        d.get_by_role('button',name='Inspect selected tasks',exact=True).click();expect(d.locator('.task-stop-inspection')).to_contain_text(['Ready to stop','Refused'],timeout=20000)
        expect(d).to_contain_text('Service does not match this account task');assert len(history(app))==1
        assert launched(a)==[original] and launched(b)==[]
        for box in d.get_by_role('textbox').all():expect(box).to_have_value('')
        app.screenshot('review.png')
        passed('Cross-page selection across two accounts uses fresh masked credentials; read-only inspection refuses foreign services while the normal batch stays active')
        d.get_by_role('button',name='Stop verified tasks',exact=True).click()
        stop_id=eventually(lambda:next((t['id'] for t in history(app) if t['script_id']=='builtin:task-stop'),None))
        eventually(lambda:stop_id in launched(a) and stop_id in launched(b))
        saved=next(t for t in history(app) if t['id']==stop_id)
        assert [d['count'] for d in saved['stops'].values()]==[2,1]
        assert next(t for t in history(app) if t['id']==original)['targets'][0]['status']=='Running'
        app.stop();time.sleep(4.3);app.launch();p=app.page
        p.locator('.rail-tasks').click();entry=p.locator('.task-entry').filter(has_text='Stop server tasks');entry.locator('.task-heading').click()
        expect(entry.locator('.task-status')).to_have_text(['Result unknown']*2)
        for target in entry.locator('.task-target').all():
            target.get_by_role('button',name='Query server result',exact=True).click();q=p.get_by_role('dialog',name='Query server result',exact=True)
            q.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test' if 'Alpha' in target.inner_text() else '')
            q.get_by_role('button',name='Query',exact=True).click();expect(q).not_to_be_visible()
        expect(entry.locator('.task-status')).to_have_text(['Succeeded']*2,timeout=20000)
        expect(entry.locator('.task-stop-report')).to_contain_text(['alpha-0000 · Stopped','beta-running · Stopped'])
        expect(entry.locator('.task-output')).not_to_contain_text(['ARCNIX_TASK_STOP_V1','ARCNIX_TASK_STOP_V1'])
        assert launched(a)==[original,stop_id] and launched(b)==[stop_id]
        assert 'foreign-service' not in entry.inner_text()
        app.screenshot('results.png')
        passed('Accepted control jobs survive desktop termination; restart queries the original IDs and requires per-task stop proof without replaying either batch')
        # Recover the stop report without any local batch history or workspace.
        app.stop();app.config.with_suffix('.tasks.json').unlink();app.launch();p=app.page
        groups=connect(app,True);row(groups.first,stop_id).get_by_role('button',name='View saved log',exact=True).click()
        d=p.get_by_role('dialog',name='Saved server task log',exact=True);expect(d.locator('.task-stop-report')).to_contain_text('alpha-0102 · Stopped',timeout=20000)
        expect(d.locator('.record-saved-output')).not_to_contain_text('ARCNIX_TASK_STOP_V1');d.get_by_role('button',name='Close',exact=True).click()
        passed('Public server metadata recovers readable stop results after local history is removed, with no SFTP or workspace connection')

        # A failed peer can be inspected again without discarding verified peers.
        row(groups.first,'alpha-0101').get_by_role('button',name='Select task',exact=True).click();row(groups.last,'foreign-service').get_by_role('button',name='Select task',exact=True).click()
        (b.root/'stops-inspect-denied').touch();d=review(p);d.get_by_role('button',name='Inspect selected tasks',exact=True).click()
        expect(d.locator('.task-stop-inspection').first).to_contain_text('Ready to stop',timeout=20000);expect(d.locator('.task-stop-peer').last).to_contain_text('denied',timeout=20000)
        expect(d).not_to_contain_text('fixture-admin-secret');expect(d.get_by_role('textbox',name='sudo password (optional)',exact=True)).to_have_value('')
        (b.root/'stops-inspect-denied').unlink();d.get_by_role('textbox',name='sudo password (optional)',exact=True).fill('fixture-admin-secret');d.get_by_role('button',name='Inspect selected tasks',exact=True).click()
        expect(d.locator('.task-stop-inspection')).to_contain_text(['Ready to stop','Refused'],timeout=20000);assert not app.config.with_suffix('.tasks.json').exists()
        d.get_by_role('button',name='Close',exact=True).click();p.get_by_role('button',name='Clear selected tasks',exact=True).click()
        passed('An authentication or permission failure can be retried for only the failed peer, preserving other verified snapshots and redacting credentials')

        # Cancellation before submission must never leave a queryable target.
        row(groups.first,'alpha-0101').get_by_role('button',name='Select task',exact=True).click();d=review(p);d.get_by_role('button',name='Inspect selected tasks',exact=True).click();expect(d.locator('.task-stop-inspection')).to_contain_text('Ready to stop',timeout=20000)
        (a.root/'stops-prepare-delay').touch();d.get_by_role('button',name='Stop verified tasks',exact=True).click();d.get_by_role('button',name='Close',exact=True).click()
        eventually(lambda:history(app)[0]['targets'][0]['status']=='Cancelled');assert history(app)[0]['targets'][0]['remote_directory']==''
        assert launched(a)==[original,stop_id];(a.root/'stops-prepare-delay').unlink()
        passed('Closing the review during preparation cancels the unsubmitted server and clears its query directory; no control launch occurs')
        # An exit zero with an incorrect report remains unknown.
        (a.root/'stops-result-wrong').touch();d=review(p);d.get_by_role('button',name='Inspect selected tasks',exact=True).click();expect(d.locator('.task-stop-inspection')).to_contain_text('Ready to stop',timeout=20000);d.get_by_role('button',name='Stop verified tasks',exact=True).click()
        bad_id=eventually(lambda:history(app)[0]['id'] if history(app)[0]['targets'][0]['status']!='Cancelled' else None);eventually(lambda:bad_id in launched(a));d.get_by_role('button',name='Close',exact=True).click()
        eventually(lambda:history(app)[0]['targets'][0]['status']=='Unknown',timeout=20)
        assert history(app)[0]['targets'][0]['exit_code']==0
        passed('An unbound final report cannot promote process exit zero into confirmed stop success')
        # A delayed inspection must not resurrect a dismissed review.
        (a.root/'stops-result-wrong').unlink();(a.root/'stops-inspect-slow').touch();d=review(p);d.get_by_role('button',name='Inspect selected tasks',exact=True).click();d.get_by_role('button',name='Close',exact=True).click();time.sleep(2.3);expect(p.get_by_role('dialog')).to_have_count(0)
        # Readability and help at a narrower Chinese viewport.
        app.settings('General');p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check();p.set_viewport_size(dict(width=900,height=780));p.locator('.rail-tasks').click();p.get_by_role('button',name='服务器记录',exact=True).click();p.get_by_role('button',name='连接服务器记录…',exact=True).click();c=p.get_by_role('dialog',name='连接服务器记录',exact=True);c.get_by_role('textbox',name='SSH 密码或私钥口令',exact=True).first.fill('arcnix-test');c.get_by_role('button',name='读取服务器记录',exact=True).click();expect(c).not_to_be_visible();g=p.locator('.server-record-group').first;expect(row(g,'alpha-0100')).to_be_visible(timeout=20000);row(g,'alpha-0100').get_by_role('button',name='选择任务',exact=True).click();p.get_by_role('button',name='审核选中任务的停止…',exact=True).click()
        d=p.get_by_role('dialog',name='停止服务器任务',exact=True);expect(d).to_be_visible();tip=d.get_by_role('button',name='帮助：停止服务器任务',exact=True);tip.hover();expect(p.get_by_role('tooltip')).to_contain_text('客户端关闭');tip.click();p.mouse.move(890,770);expect(p.get_by_role('tooltip')).to_be_visible()
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert not d.locator('*:visible').evaluate_all("els=>els.filter(el=>!el.children.length&&el.textContent.trim()&&parseFloat(getComputedStyle(el).fontSize)<12).map(el=>el.textContent)")
        app.screenshot('chinese-help.png')
        for file in [app.config,app.config.with_suffix('.tasks.json')]:
            if file.exists():assert not any(secret in file.read_text() for secret in ['arcnix-test','fixture-admin-secret'])
        passed('Late reads cannot reopen a closed review; Chinese help supports hover and click at 900 px with readable text and no persisted credentials')

    duplicate_config=a.config([alpha,a.profile('Other SSH account')],out)
    with Desktop(out/'duplicate-account',duplicate_config) as app:
        groups=connect(app,True);p=app.page
        for group in groups.all():row(group,'alpha-0100').get_by_role('button',name='Select task',exact=True).click()
        before=launched(a);p.get_by_role('button',name='Review selected stops…',exact=True).click()
        expect(p.locator('.server-records-page>.form-error')).to_contain_text('Choose one SSH account per server for each stop batch')
        expect(p.get_by_role('dialog',name='Stop server tasks',exact=True)).to_have_count(0)
        p.get_by_role('button',name='Clear selected tasks',exact=True).click();expect(p.locator('.task-stop-select.active')).to_have_count(0);expect(p.get_by_role('button',name='Review selected stops…',exact=True)).to_be_disabled()
        assert launched(a)==before and not app.config.with_suffix('.tasks.json').exists()
        passed('Two SSH accounts on one endpoint require separate stop batches; clearing selections removes cross-page choices without creating or launching work')

(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8')
print(f'{len(checks)} checks passed; artifacts: {out}',flush=True)
