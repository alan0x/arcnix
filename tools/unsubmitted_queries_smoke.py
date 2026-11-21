"""Cancelled preflight directories must not become server-result requests."""
import json,time
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/unsubmitted-queries-ui'/str(time.time_ns());checks=[]
def passed(label):checks.append(label);print(label,flush=True)
def history(app):return eventually(lambda:json.loads(app.config.with_suffix('.tasks.json').read_text()),timeout=3)
def prepare(app,concurrency=None):
    p=app.page;p.locator('.rail-scripts').click()
    p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click()
    expect(p.locator('.script-row')).to_have_count(1)
    p.get_by_role('button',name='Run on servers…',exact=True).click()
    d=p.get_by_role('dialog',name='Run script on servers',exact=True)
    if concurrency is not None:d.get_by_role('button',name=str(concurrency),exact=True).click()
    d.get_by_role('button',name='Select visible trusted servers',exact=True).click()
    d.get_by_role('button',name='Check targets',exact=True).click()
    expect(p.locator('.task-status')).to_have_text(['Ready','Ready'])
    assert all(t['remote_directory'] for t in history(app)[0]['targets'])
    return p
def recent(app):
    p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click();return p
def queries(peer):
    path=peer.root/'task-queries.jsonl'
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
def launches(peer):
    path=peer.root/'task-launches.jsonl'
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    for peer in [a,b]:(peer.root/'trace-task-queries').touch()
    config=a.config([a.profile('Alpha'),b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1)
    config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    config['config']['snippets']=[dict(id='unsubmitted',name='Submission boundary',description='',command='fixture-command',administrator=False)]
    with Desktop(out/'desktop',config) as app:
        p=prepare(app)
        p.get_by_role('button',name='Stop pending targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Cancelled','Cancelled'])
        expect(p.locator('.task-batch-refresh:visible')).to_be_disabled()
        for button in p.locator('.task-target').get_by_role('button',name='Query server result',exact=True).all():expect(button).to_be_disabled()
        assert not queries(a) and not queries(b) and not launches(a) and not launches(b)
        passed('Cancelled preflight keeps planned directories but disables single and batch result queries without contacting servers')

        app.stop();app.launch();p=recent(app)
        expect(p.locator('.task-status')).to_have_text(['Cancelled','Cancelled'])
        expect(p.locator('.task-batch-refresh:visible')).to_be_disabled()
        assert not queries(a) and not queries(b)
        passed('Restart preserves definite cancellation and does not turn a planned path into evidence of submission')

        (a.root/'task-launch-delay').touch()
        p=prepare(app,1)
        p.get_by_role('button',name='Execute ready targets',exact=True).click()
        expect(p.locator('.task-status').first).to_have_text('Submitting')
        p.get_by_role('button',name='Stop pending targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Succeeded','Cancelled'],timeout=20000)
        identifier=history(app)[0]['id']
        assert len(launches(a))==1 and not launches(b)
        expect(p.locator('.task-target').first.get_by_role('button',name='Query server result',exact=True)).to_be_enabled()
        expect(p.locator('.task-target').nth(1).get_by_role('button',name='Query server result',exact=True)).to_be_disabled()
        p.locator('.task-batch-refresh:visible').click();d=p.get_by_role('dialog',name='Query batch results',exact=True)
        expect(d.locator('.task-refresh-choice.active')).to_have_count(1)
        expect(d.locator('.task-refresh-choice').nth(1)).to_be_disabled()
        expect(d).to_contain_text('This task was never submitted to the server')
        d.get_by_role('button',name='Cancel',exact=True).click()
        passed('Stopping a one-slot submission queue leaves the accepted server running and its unsubmitted peer unqueryable')

        before_b=queries(b)
        p.locator('.task-batch-refresh:visible').click();d=p.get_by_role('dialog',name='Query batch results',exact=True)
        d.get_by_role('button',name='Query selected results',exact=True).click();expect(d).not_to_be_visible()
        expect(p.locator('.task-query-progress')).to_have_text(['',''])
        expect(p.locator('.task-status')).to_have_text(['Succeeded','Cancelled'])
        assert queries(b)==before_b and len(launches(a))==1 and not launches(b)
        passed('A mixed batch reads only its accepted target and keeps the cancelled target and all launch counts unchanged')

        app.stop();saved=history(app)
        saved[0]['targets'][0]['status']='Submitting';saved[0]['targets'][1]['status']='Queued'
        app.config.with_suffix('.tasks.json').write_text(json.dumps(saved))
        app.launch();p=recent(app)
        expect(p.locator('.task-status')).to_have_text(['Result unknown','Result unknown'])
        for button in p.locator('.task-target').get_by_role('button',name='Query server result',exact=True).all():expect(button).to_be_enabled()
        p.locator('.task-batch-refresh:visible').click();d=p.get_by_role('dialog',name='Query batch results',exact=True)
        expect(d.locator('.task-refresh-choice.active')).to_have_count(2)
        d.get_by_role('button',name='Query selected results',exact=True).click();expect(d).not_to_be_visible()
        expect(p.locator('.task-query-progress')).to_have_text(['',''])
        expect(p.locator('.task-status')).to_have_text(['Succeeded','Result unknown'])
        expect(p.locator('.task-target').nth(1)).to_contain_text('Query failed:')
        assert len(launches(a))==1 and not launches(b)
        assert history(app)[0]['targets'][1].get('result_read_at') is None
        passed('Uncertain queued/submitting history still permits read-only verification; an absent result stays unknown without replay')

        app.stop();saved=history(app);saved[0]['targets'][1]['status']='Ready'
        app.config.with_suffix('.tasks.json').write_text(json.dumps(saved));app.launch();p=recent(app)
        expect(p.locator('.task-status')).to_have_text(['Succeeded','Cancelled'])
        expect(p.locator('.task-target').nth(1).get_by_role('button',name='Query server result',exact=True)).to_be_disabled()
        app.settings('General');p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check()
        expect(p.locator('.page-header-title')).to_have_text('常规')
        p.locator('.rail-tasks').click();p.set_viewport_size(dict(width=900,height=700))
        p.locator('.task-batch-refresh:visible').click();d=p.get_by_role('dialog',name='查询批次结果',exact=True)
        expect(d.locator('.task-refresh-choice.active')).to_have_count(1)
        expect(d).to_contain_text('此任务从未提交到服务器')
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        app.screenshot('cancelled-query-chinese-narrow.png')
        d.get_by_role('button',name='取消',exact=True).click()
        assert len(history(app))==2 and history(app)[0]['id']==identifier
        assert all(t['status']=='Cancelled' for t in history(app)[1]['targets'])
        passed('Legacy ready history becomes cancelled; Chinese narrow review keeps the exact submitted and unsubmitted scopes')

(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps(dict(passed=len(checks),artifacts=str(out)),ensure_ascii=False),flush=True)
