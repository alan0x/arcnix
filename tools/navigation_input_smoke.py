"""Correlate rapid locale/viewport transitions with rail input delivery."""
import argparse,json,os,re,time
from desktop_fixture import Desktop,expect,ROOT

parser=argparse.ArgumentParser();parser.add_argument('--rounds',type=int,default=60);parser.add_argument('--transition',choices=['locale','viewport','locale_viewport','theme_viewport'],default='locale_viewport');args=parser.parse_args()
assert 10<=args.rounds<=200
os.environ['ARCNIX_TEST_TRACE_INPUT']='1'
out=ROOT/'artifacts/navigation-input-ui'/str(time.time_ns());rounds=[]
config=dict(version=1,config=dict(servers=[],groups=[],trusted_hosts=[],local_directory=str(out),settings=dict(language='English',theme='Light',show_hidden_files=True,resource_interval_seconds=3)))
def active(p,name):expect(p.locator('.navigation-rail .active')).to_have_class(re.compile(r'.*\b'+name+r'\b.*'))
with Desktop(out/'desktop',config) as app:
    p=app.page
    p.evaluate("""()=>{window.__arcnixStableShell=[document.querySelector('.app'),document.querySelector('.navigation-rail'),...document.querySelectorAll('.navigation-rail button')]}""")
    for n in range(args.rounds):
        locale=args.transition.startswith('locale')
        p.locator('.settings-link').click();active(p,'settings-link')
        p.locator('.context-links').get_by_role('button',name='General' if not locale or n%2==0 else '常规',exact=True).click()
        if locale:p.get_by_role('button',name='简体中文' if n%2==0 else 'English',exact=True).click()
        if args.transition=='theme_viewport':p.get_by_role('button',name='Dark' if n%2==0 else 'Light',exact=True).click()
        if args.transition!='locale':p.set_viewport_size(dict(width=900 if n%2==0 else 1440,height=700 if n%2==0 else 980))
        p.locator('.rail-tasks').click();active(p,'rail-tasks')
        expect(p.get_by_role('button',name='清理报告' if locale and n%2==0 else 'Cleanup reports',exact=True)).to_be_visible()
        p.locator('.rail-backups').click();active(p,'rail-backups')
        p.locator('.rail-configurations').click();active(p,'rail-configurations')
        assert p.evaluate("()=>window.__arcnixStableShell.every(e=>e.isConnected)"),'Locale or navigation retired the live application shell'
        rounds.append(dict(round=n+1,language='Chinese' if locale and n%2==0 else 'English',width=p.viewport_size['width']))
        (out/'rounds.json').write_text(json.dumps(rounds),encoding='utf-8')
        print('round',n+1,'passed',flush=True)
    app.screenshot('final-navigation.png')
native=[json.loads(line.split(' ',1)[1]) for line in (out/'desktop/desktop.log').read_text(encoding='utf-8').splitlines() if line.startswith('ARCNIX_NAVIGATION ')]
events=[json.loads(line) for path in (out/'desktop').glob('input-*.jsonl') for line in path.read_text(encoding='utf-8').splitlines()]
actions=[json.loads(line) for path in (out/'desktop').glob('actions-*.jsonl') for line in path.read_text(encoding='utf-8').splitlines()]
clicks=[event for event in events if event['kind']=='click' and 'rail-item' in event['classes']]
routes=[event for event in native if event['source']=='rail']
assert len(routes)==len(clicks)==args.rounds*4,(len(routes),len(clicks),args.rounds)
names={'settings-link':'Settings','rail-tasks':'Tasks','rail-backups':'Backups','rail-configurations':'Configurations'}
assert [event['to'] for event in routes]==[next(name for key,name in names.items() if key in event['classes'].split()) for event in clicks]
assert len([event for event in events if event['kind']=='ipc-activate'])>=len(clicks)
starts={event['sequence']:event['at'] for event in actions if event['phase']=='start'}
intervals=[(starts[event['sequence']],event['at']) for event in actions if event['phase']=='end']
assert all(any(start-2<=event['at']<=end+2 for start,end in intervals) for event in clicks)
result=dict(rounds=args.rounds,transition=args.transition,rail_inputs=len(clicks),native_rail_callbacks=len(routes),artifacts=str(out))
(out/'summary.json').write_text(json.dumps(result,indent=2),encoding='utf-8');print(json.dumps(result),flush=True)
