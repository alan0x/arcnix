"""Correlate navigation callbacks and WebView input while task views change."""
import argparse, json, os, re, time
from desktop_fixture import Peer, Desktop, expect, eventually, ROOT

parser=argparse.ArgumentParser()
parser.add_argument('--rounds',type=int,default=12)
args=parser.parse_args()
assert 12<=args.rounds<=200
os.environ['ARCNIX_TEST_TRACE_INPUT']='1'
out=ROOT/'artifacts/task-navigation-ui'/str(time.time_ns())

def active(p,page):
    name='settings-link' if page=='settings' else 'rail-'+page
    expect(p.locator('.navigation-rail .active')).to_have_class(re.compile(r'.*\b'+name+r'\b.*'))

def navigate(p,page):
    p.locator('.rail-'+page).click();active(p,page)

def history(app):
    return eventually(lambda: json.loads(app.config.with_suffix('.tasks.json').read_text()),timeout=3)

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    config=a.config([a.profile('Alpha'),b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1)
    config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    config['config']['snippets']=[dict(id='navigation',name='Navigation stress',description='',command="printf '%s\\n' {{message}}",administrator=False)]
    with Desktop(out/'desktop',config) as app:
        p=app.page;app.connect()
        for n in range(args.rounds):
            navigate(p,'scripts')
            p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click()
            expect(p.locator('.script-row')).to_have_count(1)
            p.get_by_role('button',name='Run on servers…',exact=True).click()
            d=p.get_by_role('dialog',name='Run script on servers',exact=True)
            d.get_by_role('textbox',name='message',exact=True).fill('task navigation '+str(n)+' 中文')
            d.get_by_role('button',name='Select visible trusted servers',exact=True).click()
            expect(d.locator('.target-choice.active')).to_have_count(2)
            d.get_by_role('button',name='Check targets',exact=True).click()
            expect(p.locator('.task-status')).to_have_text(['Ready','Ready'])
            active(p,'tasks')
            p.get_by_role('button',name='Execute ready targets',exact=True).click()
            expect(p.locator('.task-status')).to_have_text(['Running','Running'])
            identifier=history(app)[0]['id']

            if n%3==0:
                navigate(p,'backups');expect(p.locator('.backup-page')).to_be_visible()
                navigate(p,'diagnostics')
                p.get_by_role('textbox',name='PEM certificate path (optional)',exact=True).fill('/tmp/navigation-'+str(n)+'.pem')
                active(p,'diagnostics')
            elif n%3==1:
                navigate(p,'scripts')
                p.get_by_role('button',name='New script',exact=True).click()
                d=p.get_by_role('dialog',name='Save command',exact=True)
                d.get_by_role('textbox',name='Command name',exact=True).fill('Unsaved navigation draft '+str(n))
                d.get_by_role('textbox',name='Shell command',exact=True).fill('printf draft-'+str(n)+'\n')
                d.get_by_role('button',name='Cancel',exact=True).click()
                expect(d).not_to_be_visible();active(p,'scripts')
                expect(p.locator('.script-row')).to_have_count(1)
            else:
                navigate(p,'servers')
                expect(p.locator('.connection-state')).to_have_text('SSH connected')
                expect(p.locator('.module-tab')).to_have_count(6)
                app.settings('General');active(p,'settings')
                p.locator('.setting-theme:visible').get_by_role('radio', name='Dark' if n%2 else 'Light', exact=True).check()
                active(p,'settings')

            navigate(p,'tasks')
            end=time.monotonic()+20
            while True:
                active(p,'tasks')
                values=p.locator('.task-status').all_text_contents()
                if values==['Succeeded','Succeeded']:break
                assert time.monotonic()<end,dict(round=n,status=values)
                p.wait_for_timeout(50)
            assert history(app)[0]['id']==identifier
            p.wait_for_timeout(150);active(p,'tasks')
            if n%4==0:
                p.locator('.task-batch-refresh:visible').click()
                d=p.get_by_role('dialog',name='Query batch results',exact=True)
                d.get_by_role('button',name='Query selected results',exact=True).click()
                expect(d).not_to_be_visible()
                navigate(p,'backups')
                p.wait_for_timeout(150);active(p,'backups')
                navigate(p,'tasks')
                expect(p.locator('.task-query-progress')).to_have_text(['',''])
                expect(p.locator('.task-status')).to_have_text(['Succeeded','Succeeded'])
            with (out/'rounds.jsonl').open('a',encoding='utf-8') as output:
                output.write(json.dumps(dict(round=n+1,task=identifier,targets=2,status='Succeeded'))+'\n')
            print('round',n+1,'passed',flush=True)
        saved=history(app);assert len(saved)==10
        assert len({task['id'] for task in saved})==10
        navigate(p,'servers');expect(p.locator('.connection-state')).to_have_text('SSH connected')
        app.screenshot('connected-workspace-after-task-churn.png')
        navigate(p,'tasks');app.screenshot('tasks-after-history-churn.png')

    native=[]
    for line in (out/'desktop/desktop.log').read_text(encoding='utf-8').splitlines():
        if line.startswith('ARCNIX_NAVIGATION '):native.append(json.loads(line.split(' ',1)[1]))
    traces=list((out/'desktop').glob('input-*.jsonl'));assert len(traces)==1
    events=[json.loads(line) for line in traces[0].read_text(encoding='utf-8').splitlines()]
    clicks=[e for e in events if e['kind']=='click' and 'rail-item' in e['classes']]
    ipc=[e for e in events if e['kind']=='ipc-activate']
    actions=[json.loads(line) for path in (out/'desktop').glob('actions-*.jsonl') for line in path.read_text().splitlines()]
    starts={e['sequence']:e['at'] for e in actions if e['phase']=='start'}
    intervals=[(starts[e['sequence']],e['at']) for e in actions if e['phase']=='end']
    unmatched=[e for e in events if e['kind']=='click' and not any(start-2<=e['at']<=end+2 for start,end in intervals)]
    assert not unmatched,dict(unrequested_clicks=unmatched)
    assert ipc and len(ipc)>=len(clicks),'The IPC trace must observe native activations'
    routes=[e for e in native if e['source']=='rail']
    assert len(routes)==len(clicks),(len(routes),len(clicks))
    assert sum(e['source']=='prepare-script' for e in native)==args.rounds
    names={'Servers':'Servers','Scripts':'Scripts','Tasks':'Tasks','Backups':'Backups','Diagnostics':'Diagnostics','Settings':'Settings'}
    assert [e['to'] for e in routes]==[names[e['name']] for e in clicks]
    assert all(b['sequence']==a['sequence']+1 for a,b in zip(native,native[1:]))
    result=dict(passed=True,rounds=args.rounds,targets_per_round=2,retained_batches=10,
        rail_inputs=len(clicks),native_rail_callbacks=len(routes),ipc_activations=len(ipc),unrequested_clicks=len(unmatched),artifacts=str(out))
    (out/'results.json').write_text(json.dumps(result,indent=2),encoding='utf-8')
    print(json.dumps(result),flush=True)
