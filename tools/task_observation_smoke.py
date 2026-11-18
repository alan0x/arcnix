"""Task result evidence in live monitoring, reconnects and server record views."""
import json,time
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/task-observation-ui'/str(time.time_ns())
checks=[]
def passed(label):checks.append(label);print(label,flush=True)
def state(peer,**values):
    path=peer.root/'task-observation.json';temporary=path.with_suffix('.tmp')
    temporary.write_text(json.dumps(values),encoding='utf-8');temporary.replace(path)
def history(app):return json.loads(app.config.with_suffix('.tasks.json').read_text())
def query(app,status,marker):
    p=app.page;target=p.locator('.task-target').first
    target.get_by_role('button',name='Query server result',exact=True).click()
    p.get_by_role('dialog',name='Query server result',exact=True).get_by_role('button',name='Query',exact=True).click()
    expect(target).to_contain_text(marker)
    expect(target.locator('.task-status')).to_have_text(status)
    return target

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    config=a.config([a.profile('Alpha'),b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1)
    config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    config['config']['snippets']=[dict(id='evidence',name='Result evidence task',description='',command='fixture-command',administrator=False)]
    state(a,before='active',saved=0,output='active snapshot 中文\n')
    with Desktop(out/'desktop',config) as app:
        p=app.page;p.locator('.rail-scripts').click()
        p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click()
        expect(p.locator('.script-row')).to_have_count(1)
        p.get_by_role('button',name='Run on servers…',exact=True).click()
        d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        d.get_by_role('button',name='Select visible trusted servers',exact=True).click()
        d.get_by_role('button',name='Check targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready','Ready'])
        p.get_by_role('button',name='Execute ready targets',exact=True).click()
        expect(p.locator('.task-target').first).to_contain_text('active snapshot 中文')
        expect(p.locator('.task-status').first).to_have_text('Running')
        expect(p.locator('.task-status').nth(1)).to_have_text('Succeeded',timeout=20000)
        eventually(lambda: history(app)[0]['targets'][0]['exit_code'] is None)
        passed('A saved zero cannot finish a live target; another server completes independently')
        state(a,before='deactivating',saved=0,output='stopping snapshot 中文\n')
        expect(p.locator('.task-status').first).to_have_text('Stopping')
        eventually(lambda: history(app)[0]['targets'][0]['status']=='Stopping')
        assert history(app)[0]['targets'][0]['exit_code'] is None
        app.screenshot('stopping.png')
        identifier=history(app)[0]['id'];app.stop();app.launch();p=app.page
        p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        expect(p.locator('.task-status')).to_have_text(['Result unknown','Succeeded'])
        assert history(app)[0]['id']==identifier
        passed('Stopping is explicit and persisted; restart makes its unconfirmed result unknown without replay')
        state(a,before='unavailable',saved=0,output='fresh unavailable snapshot 中文\n')
        query(app,'Result unknown','fresh unavailable snapshot 中文')
        assert history(app)[0]['targets'][0]['exit_code'] is None
        state(a,before='failed',saved=0,output='fresh conflict snapshot 中文\n')
        target=query(app,'Result unknown','fresh conflict snapshot 中文')
        expect(target).to_contain_text('conflicts with failed service state')
        passed('Manager lookup failure and failed/saved-zero conflicts retain fresh logs without confirming an exit')
        state(a,before='active',after='inactive',saved=0,output='state changed snapshot\n')
        query(app,'Result unknown','state changed snapshot')
        state(a,before='inactive',before_invocation='a'*32,invocation='b'*32,saved=0,output='invocation changed snapshot\n')
        query(app,'Result unknown','invocation changed snapshot')
        state(a,before='inactive',raw='0\0',output='invalid byte snapshot\n')
        target=query(app,'Result unknown','invalid byte snapshot')
        expect(target).to_contain_text('exit result is invalid')
        passed('State or invocation changes and malformed saved bytes cannot become successful results')
        state(a,before='inactive',saved=0,output='confirmed original snapshot 中文\n')
        query(app,'Succeeded','confirmed original snapshot 中文')
        for options,reason in [(dict(legacy=True,saved=0),'no runtime binding'),(dict(legacy=True,saved=7),'no runtime binding'),(dict(receipt_task='other-task',saved=0),'different task'),(dict(receipt_invocation='b'*32,saved=0),'different service invocation')]:
            state(a,before='inactive',output='unconfirmed receipt snapshot '+reason+'\n',**options)
            target=query(app,'Result unknown','unconfirmed receipt snapshot '+reason);expect(target).to_contain_text(reason)
            assert history(app)[0]['targets'][0]['exit_code'] is None
        state(a,before='not-found',invocation='-',saved=0,output='completed receipt after manager collection\n')
        query(app,'Succeeded','completed receipt after manager collection')
        passed('Legacy zeros/nonzeros and mismatched task/runtime receipts remain unknown; a complete bound receipt survives manager garbage collection')
        state(a,before='inactive',saved=0,output='confirmed original snapshot 中文\n');query(app,'Succeeded','confirmed original snapshot 中文')
        state(a,before='inactive',saved=0,wrong_id=True,output='untrusted other task bytes')
        target=query(app,'Result unknown','Server task query returned a different task')
        expect(target).to_contain_text('confirmed original snapshot 中文')
        expect(target).not_to_contain_text('untrusted other task bytes')
        assert history(app)[0]['targets'][0]['exit_code'] is None
        state(a,before='inactive',saved=0,weak_protocol=True,output='unversioned data')
        target=query(app,'Result unknown','Incomplete or unsupported server task observation')
        expect(target).not_to_contain_text('unversioned data')
        passed('Wrong-task and unversioned responses preserve the previous snapshot and clear stale confirmation')
        state(a,before='failed',saved=7,output='confirmed nonzero snapshot\n')
        query(app,'Failed','confirmed nonzero snapshot')
        assert history(app)[0]['targets'][0]['exit_code']==7
        state(a,before='deactivating',saved=0,output='stopping again snapshot\n')
        query(app,'Stopping','stopping again snapshot')
        p.get_by_role('button',name='Server records',exact=True).click()
        d=p.get_by_role('dialog',name='Connect to server records',exact=True)
        d.get_by_role('button',name='Select trusted servers',exact=True).click()
        d.get_by_role('button',name='Read server records',exact=True).click()
        groups=p.locator('.server-record-group');expect(groups).to_have_count(2)
        expect(groups.first.locator('.record-status')).to_have_text('Stopping')
        expect(groups.nth(1).locator('.record-status')).to_have_text('Succeeded')
        passed('A settled nonzero remains Failed; independent server records agree on Stopping and successful peers')
        state(a,before='inactive',legacy=True,saved=0,output='legacy independent snapshot\n')
        groups.first.get_by_role('button',name='Refresh records',exact=True).click()
        expect(groups.first.locator('.record-status')).to_have_text('Result unknown');expect(groups.first.locator('.record-reason')).to_contain_text('no runtime binding')
        groups.first.get_by_role('button',name='View saved log',exact=True).click();d=p.get_by_role('dialog',name='Saved server task log',exact=True)
        expect(d).to_contain_text('no runtime binding');expect(d).to_contain_text('legacy independent snapshot');expect(d).to_contain_text('No confirmed exit result');d.get_by_role('button',name='Close',exact=True).click()
        state(a,before='deactivating',saved=0,output='stopping again snapshot\n');groups.first.get_by_role('button',name='Refresh records',exact=True).click();expect(groups.first.locator('.record-status')).to_have_text('Stopping')
        passed('Independent record rows and saved logs apply the same legacy receipt rule without rewriting local task history')

        app.settings('General');p.get_by_role('button',name='简体中文',exact=True).click()
        expect(p.locator('.page-header-title')).to_have_text('常规')
        p.locator('.rail-tasks').click()
        p.get_by_role('button',name='最近批次',exact=True).click()
        expect(p.locator('.task-status').first).to_have_text('停止中')
        p.set_viewport_size(dict(width=900,height=700))
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        app.screenshot('stopping-chinese-narrow.png')
        saved=history(app);assert len(saved)==1 and saved[0]['id']==identifier and len(saved[0]['targets'])==2
        passed('Chinese narrow layout retains the same original batch and no observation submits another job')
(out/'results.json').write_text(json.dumps(dict(passed=len(checks),checks=checks),ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps(dict(passed=len(checks),artifacts=str(out)),ensure_ascii=False))
