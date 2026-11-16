"""Read original multi-server results after restart, with bounded query concurrency."""
from contextlib import ExitStack
import copy, json, re, time
from desktop_fixture import Peer, Desktop, expect, eventually, ROOT

out = ROOT/'artifacts/batch-refresh-ui'/str(time.time_ns())
checks = []

def passed(label):
    checks.append(label)
    print(label, flush=True)

def history(app):
    return json.loads(app.config.with_suffix('.tasks.json').read_text())

def audit(peers):
    rows = []
    for peer in peers:
        path = peer.root/'task-queries.jsonl'
        if path.exists():
            rows.extend(dict(peer=peer.profile('p')['port'], **json.loads(line)) for line in path.read_text().splitlines())
    return rows

def clear_audit(peers):
    for peer in peers:
        (peer.root/'task-queries.jsonl').unlink(missing_ok=True)

def peak(peers, expected):
    rows = audit(peers)
    assert sum(r['phase']=='start' for r in rows)==expected, rows
    assert sum(r['phase']=='end' for r in rows)==expected, rows
    active = maximum = 0
    for row in sorted(rows, key=lambda r:r['at']):
        active += 1 if row['phase']=='start' else -1
        maximum = max(maximum, active)
        assert active >= 0, rows
    assert active == 0
    return maximum

def open_query(p, name='Query batch results'):
    p.locator('.task-batch-refresh:visible').click()
    dialog = p.get_by_role('dialog', name=name, exact=True)
    expect(dialog).to_be_visible()
    return dialog

def submit_query(d):
    d.get_by_role('button',name='Query selected results',exact=True).click()
    expect(d).not_to_be_visible()

def no_pending(p):
    expect(p.locator('.task-query-progress')).to_have_text(['']*5)

with ExitStack() as stack:
    peers = [stack.enter_context(Peer(out/('peer-'+str(n)))) for n in range(4)]
    for peer in peers:
        (peer.root/'trace-task-queries').touch()
    profiles = [peer.profile(name) for peer,name in zip(peers,['Alpha','Beta','Gamma','Delta'])]
    cancelled = dict(profiles[0], id='never', name='Never submitted', port=9)
    config = peers[0].config(profiles+[cancelled], out)
    for peer in peers[1:]:
        host,port = peer.data['address'].rsplit(':',1)
        config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=peer.data['host_key_rsa_fingerprint']))
    config['config']['snippets'] = [dict(id='query-batch',name='Original batch',description='',command='fixture-command',administrator=False)]
    with Desktop(out/'desktop', config) as app:
        p=app.page
        p.locator('.rail-scripts').click()
        p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click()
        expect(p.locator('.script-row')).to_have_count(1)
        p.get_by_role('button',name='Run on servers…',exact=True).click()
        d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        for n in range(4):d.locator('.target-choice').nth(n).click()
        expect(d.locator('.target-choice.active')).to_have_count(4)
        d.get_by_role('button',name='Check targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready']*4)
        expect(p.locator('.task-batch-refresh:visible')).to_be_disabled()
        p.get_by_role('button',name='Execute ready targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Running']*4)
        identifier=history(app)[0]['id']
        app.stop()
        time.sleep(4.2)
        saved=history(app)
        never=dict(saved[0]['targets'][0],id='never',name='Never submitted',endpoint='arcnix@127.0.0.1:9',status='Cancelled',exit_code=None,output='Never submitted fixture')
        saved[0]['targets'].append(never)
        app.config.with_suffix('.tasks.json').write_text(json.dumps(saved),encoding='utf-8')
        original=json.loads(app.config.read_text())
        changed=copy.deepcopy(original)
        changed['config']['servers'][1]['username']='different-account'
        gamma=profiles[2]
        next(t for t in changed['config']['trusted_hosts'] if t['port']==gamma['port'])['fingerprint']=peers[0].data['host_key_rsa_fingerprint']
        app.config.write_text(json.dumps(changed),encoding='utf-8')
        for peer in peers:(peer.root/'task-query-delay').write_text('0.6')
        clear_audit(peers)
        app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        expect(p.locator('.task-status')).to_have_text(['Result unknown']*4+['Cancelled'])
        d=open_query(p)
        choices=d.locator('.task-refresh-choice')
        expect(choices).to_have_count(5)
        expect(d.locator('.task-refresh-choice.active')).to_have_count(2)
        for n in [1,2,4]:expect(choices.nth(n)).to_be_disabled()
        expect(d).to_contain_text('Server address or account changed')
        expect(d).to_contain_text('host fingerprint is no longer trusted')
        expect(d).to_contain_text('never submitted')
        d.get_by_role('button',name='Clear selection',exact=True).click()
        expect(d.get_by_role('button',name='Query selected results',exact=True)).to_be_disabled()
        d.get_by_role('button',name='Select queryable targets',exact=True).click()
        expect(d.locator('.task-refresh-choice.active')).to_have_count(2)
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('discard-this-secret')
        choices.first.click();expect(d.locator('.task-refresh-choice.active')).to_have_count(1)
        choices.first.click();expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first).to_have_value('')
        d.get_by_role('button',name='Cancel',exact=True).click()
        assert not audit(peers)
        d=open_query(p)
        expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first).to_have_value('')
        app.screenshot('identity-blocks.png')
        submit_query(d)
        expect(p.locator('.task-status')).to_have_text(['Succeeded','Result unknown','Result unknown','Succeeded','Cancelled'])
        no_pending(p)
        assert peak(peers,2)==2
        passed('Original endpoints, current host trust and submission scope gate queries; deselect/cancel discard credentials')

        app.stop();app.config.write_text(json.dumps(original),encoding='utf-8');clear_audit(peers)
        app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        d=open_query(p);expect(d.locator('.task-refresh-choice.active')).to_have_count(4)
        d.locator('.task-refresh-concurrency').filter(has_text='3').click()
        submit_query(d)
        expect(p.locator('.task-status')).to_have_text(['Succeeded']*4+['Cancelled'])
        no_pending(p);parallel=peak(peers,4);assert 2<=parallel<=3,parallel
        passed('One action reads four original servers with at most three concurrent queries')
        clear_audit(peers)
        d=open_query(p);d.locator('.task-refresh-concurrency').filter(has_text='1').click()
        submit_query(d)
        expect(p.locator('.task-query-progress').filter(has_text='Reading server result…')).to_have_count(4)
        no_pending(p);assert peak(peers,4)==1
        passed('A single query slot queues targets and completes all four without concurrent SSH reads')

        app.stop()
        password=copy.deepcopy(original);password['config']['servers'][1]['auth']='Password'
        app.config.write_text(json.dumps(password),encoding='utf-8')
        (peers[2].root/'task-observation.json').write_text(json.dumps(dict(before='unavailable',saved=0,output='Fresh but unavailable manager')),encoding='utf-8')
        clear_audit(peers);app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        d=open_query(p)
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).nth(1).fill('wrong-batch-secret')
        submit_query(d)
        no_pending(p)
        expect(p.locator('.task-status')).to_have_text(['Succeeded','Result unknown','Result unknown','Succeeded','Cancelled'])
        expect(p.locator('.task-target').nth(1)).to_contain_text('Query failed:')
        expect(p.locator('.task-target').nth(2)).to_contain_text('Fresh but unavailable manager')
        assert peak(peers,3)<=3
        d=open_query(p)
        expect(d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).nth(1)).to_have_value('')
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).nth(1).fill('arcnix-test')
        (peers[2].root/'task-observation.json').unlink()
        submit_query(d);no_pending(p)
        expect(p.locator('.task-status')).to_have_text(['Succeeded']*4+['Cancelled'])
        passed('Authentication and manager failures remain per target; a new credentialed query recovers the same batch')

        clear_audit(peers)
        for n,peer in enumerate(peers):
            (peer.root/'task-observation.json').write_text(json.dumps(dict(before='inactive',saved=0,output='Navigation query completed '+str(n))),encoding='utf-8')
        d=open_query(p)
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).nth(1).fill('arcnix-test')
        submit_query(d)
        p.locator('.rail-backups').click();expect(p.locator('.rail-backups')).to_have_class(re.compile(r'.* active$'))
        eventually(lambda: all(t['status']=='Succeeded' and 'Navigation query completed' in t['output'] for t in history(app)[0]['targets'][:4]))
        assert peak(peers,4)<=3
        p.wait_for_timeout(1100)
        expect(p.locator('.rail-backups')).to_have_class(re.compile(r'.* active$'))
        p.locator('.rail-tasks').click();no_pending(p)
        passed('Query completions update history while preserving the page the user chose')

        app.settings('General');p.get_by_role('button',name='简体中文',exact=True).click()
        expect(p.locator('.page-header-title')).to_have_text('常规')
        p.locator('.rail-tasks').click();p.set_viewport_size(dict(width=900,height=700))
        d=open_query(p,'查询批次结果')
        expect(d.locator('.task-refresh-count')).to_have_text('已选目标: 4 / 32')
        expect(d.get_by_role('button',name='查询所选结果',exact=True)).to_be_visible()
        d.get_by_role('button',name='帮助：查询批次结果',exact=True).click()
        expect(p.get_by_role('tooltip')).to_contain_text('不提交、重试')
        p.keyboard.press('Escape');expect(d).to_be_visible()
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        box=d.get_by_role('button',name='查询所选结果',exact=True).bounding_box();assert box and box['y']+box['height']<=700
        app.screenshot('batch-query-chinese-narrow.png')
        d.get_by_role('button',name='取消',exact=True).click()
        passed('Chinese narrow query form keeps help optional and the action footer visible')
        current=history(app);assert len(current)==1 and current[0]['id']==identifier and len(current[0]['targets'])==5
        for peer in peers:
            launches=[json.loads(x) for x in (peer.root/'task-launches.jsonl').read_text().splitlines()]
            assert len(launches)==1 and launches[0]['id']==identifier
        persisted=app.config.read_text()+app.config.with_suffix('.tasks.json').read_text()
        assert not any(v in persisted for v in ['wrong-batch-secret','discard-this-secret','arcnix-test'])
        passed('All reconnects keep one original task per server and persist no query credentials')

(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps(dict(passed=len(checks),artifacts=str(out)),ensure_ascii=False),flush=True)
