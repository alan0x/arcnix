"""Snapshot receive times stay truthful across queries, failures and restart."""
import json,time
from datetime import datetime,timezone
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/task-read-time-ui'/str(time.time_ns())
checks=[]
def passed(label):checks.append(label);print(label,flush=True)
def history(app):return eventually(lambda:json.loads(app.config.with_suffix('.tasks.json').read_text()),timeout=3)
def state(peer,**values):
    path=peer.root/'task-observation.json';temporary=path.with_suffix('.tmp')
    temporary.write_text(json.dumps(values));temporary.replace(path)
def query(app,index=0,password=''):
    p=app.page;p.locator('.task-target').nth(index).get_by_role('button',name='Query server result',exact=True).click()
    d=p.get_by_role('dialog',name='Query server result',exact=True)
    d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill(password)
    d.get_by_role('button',name='Query',exact=True).click();expect(d).not_to_be_visible()
def loaded(app,index,marker):
    p=app.page;target=p.locator('.task-target').nth(index)
    expect(target).to_contain_text(marker)
    expect(target.locator('.task-query-progress')).to_have_text('')
    value=history(app)[0]['targets'][index]['result_read_at']
    assert isinstance(value,int) and 0<value<=int(time.time())+1
    expect(target.locator('.task-result-read')).to_contain_text('Snapshot read:')
    return value

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    for peer in [a,b]:(peer.root/'trace-task-queries').touch()
    config=a.config([a.profile('Alpha'),b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1)
    config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    config['config']['snippets']=[dict(id='read-time',name='Snapshot age',description='',command='fixture-command',administrator=False)]
    state(a,before='active',saved=0,output='First active snapshot 中文')
    with Desktop(out/'desktop',config) as app:
        p=app.page;p.locator('.rail-scripts').click()
        p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click()
        expect(p.locator('.script-row')).to_have_count(1)
        p.get_by_role('button',name='Run on servers…',exact=True).click()
        d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        d.get_by_role('button',name='Select visible trusted servers',exact=True).click()
        d.get_by_role('button',name='Check targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready','Ready'])
        expect(p.locator('.task-result-age:visible')).to_have_count(0)
        assert all(t.get('result_read_at') is None for t in history(app)[0]['targets'])
        p.get_by_role('button',name='Execute ready targets',exact=True).click()
        first=loaded(app,0,'First active snapshot 中文')
        first_label=p.locator('.task-target').first.locator('.task-result-read').inner_text()
        expect(p.locator('.task-status').nth(1)).to_have_text('Succeeded',timeout=20000)
        beta=loaded(app,1,'Server-owned fixture job finished')
        assert history(app)[0]['targets'][0]['result_read_at']==first
        expect(p.locator('.task-target').first.locator('.task-result-read')).to_have_text(first_label)
        passed('Preflight has no fabricated read time; live snapshots record receipt and unchanged snapshots keep their original age')

        state(a,before='unavailable',saved=0,output='New unavailable snapshot 中文')
        second=loaded(app,0,'New unavailable snapshot 中文');assert second>first
        expect(p.locator('.task-status').first).to_have_text('Result unknown')
        expect(p.locator('.task-target').first.get_by_role('button',name='Query server result',exact=True)).to_be_enabled()
        label=p.locator('.task-target').first.locator('.task-result-read').inner_text()
        time.sleep(1.1)
        for options,marker in [(dict(weak_protocol=True),'Incomplete or unsupported server task observation'),(dict(wrong_id=True),'Server task query returned a different task')]:
            state(a,before='inactive',saved=0,output='Rejected fresh-looking bytes',**options)
            query(app);expect(p.locator('.task-target').first).to_contain_text(marker)
            expect(p.locator('.task-target').first.locator('.task-query-progress')).to_have_text('')
            assert history(app)[0]['targets'][0]['result_read_at']==second
            expect(p.locator('.task-target').first.locator('.task-result-read')).to_have_text(label)
            expect(p.locator('.task-target').first).not_to_contain_text('Rejected fresh-looking bytes')
        passed('Valid unavailable-state replies have a fresh read time; malformed and wrong-task replies retain the old snapshot and its time')

        identifier=history(app)[0]['id'];app.stop()
        saved=history(app);saved[0]['targets'][0].pop('result_read_at')
        app.config.with_suffix('.tasks.json').write_text(json.dumps(saved))
        document=json.loads(app.config.read_text());document['config']['servers'][1]['auth']='Password'
        app.config.write_text(json.dumps(document));app.launch();p=app.page
        p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        expect(p.locator('.task-target').first.locator('.task-result-read')).to_have_text('Read time unavailable')
        assert history(app)[0]['targets'][1]['result_read_at']==beta
        beta_label=p.locator('.task-target').nth(1).locator('.task-result-read').inner_text()
        query(app,1,'bad-read-time-secret')
        expect(p.locator('.task-target').nth(1)).to_contain_text('Query failed:')
        expect(p.locator('.task-target').nth(1).locator('.task-query-progress')).to_have_text('')
        assert history(app)[0]['targets'][1]['result_read_at']==beta
        expect(p.locator('.task-target').nth(1).locator('.task-result-read')).to_have_text(beta_label)
        passed('Legacy entries show unavailable time, other targets retain their time across restart, and failed authentication cannot refresh it')

        state(a,before='inactive',saved=0,output='Confirmed new snapshot 中文')
        query(app);latest_a=loaded(app,0,'Confirmed new snapshot 中文');assert latest_a>second
        query(app,1,'arcnix-test');latest_b=loaded(app,1,'Server-owned fixture job finished');assert latest_b>beta
        expect(p.locator('.task-status')).to_have_text(['Succeeded','Succeeded'])
        app.stop();app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        assert [t['result_read_at'] for t in history(app)[0]['targets']]==[latest_a,latest_b]
        passed('A successful manual query replaces the snapshot time and the exact values survive another restart')

        app.settings('Files');p.locator('.setting-time-zone:visible').get_by_role('radio', name='UTC', exact=True).check()
        p.locator('.rail-tasks').click()
        utc=datetime.fromtimestamp(latest_a,timezone.utc).isoformat().replace('+00:00','Z').replace('T',' ')
        expect(p.locator('.task-target').first.locator('.task-result-read')).to_have_text('Snapshot read: '+utc)
        assert [t['result_read_at'] for t in history(app)[0]['targets']]==[latest_a,latest_b]
        app.settings('Files');p.get_by_role('button',name='Local timezone',exact=True).click()
        p.locator('.rail-tasks').click()
        assert [t['result_read_at'] for t in history(app)[0]['targets']]==[latest_a,latest_b]
        passed('UTC and local display preferences reformat the same stored receive time without a new query')

        app.settings('General');p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check()
        expect(p.locator('.page-header-title')).to_have_text('常规')
        p.locator('.rail-tasks').click();p.set_viewport_size(dict(width=900,height=700))
        target=p.locator('.task-target').first
        expect(target.locator('.task-result-read')).to_contain_text('结果读取于')
        target.get_by_role('button',name='帮助：结果快照',exact=True).click()
        expect(p.get_by_role('tooltip')).to_contain_text('本机时钟')
        expect(p.get_by_role('tooltip')).to_contain_text('不是任务完成时间')
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert target.locator('.task-result-read').evaluate('el=>parseFloat(getComputedStyle(el).fontSize)>=12')
        app.screenshot('snapshot-time-chinese-narrow.png')
        p.keyboard.press('Escape')
        assert len(history(app))==1 and history(app)[0]['id']==identifier
        for peer in [a,b]:
            launches=[json.loads(line) for line in (peer.root/'task-launches.jsonl').read_text().splitlines()]
            assert len(launches)==1 and launches[0]['id']==identifier
        assert 'bad-read-time-secret' not in app.config.with_suffix('.tasks.json').read_text()
        passed('Chinese narrow layout keeps snapshot time and optional help readable; queries and restarts never resubmit the original batch')

(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps(dict(passed=len(checks),artifacts=str(out)),ensure_ascii=False),flush=True)
