"""Local evidence overview, explicit batch review and workspace preservation."""
import base64, copy, hashlib, json, time
from pathlib import Path
from desktop_fixture import Peer, Desktop, expect, eventually, ROOT

out = ROOT/'artifacts/server-inventory-ui'/str(time.time_ns())
checks = []
now = int(time.time())

def passed(label):
    checks.append(label); print(label, flush=True)

def row(p, name):
    return p.locator('.inventory-row').filter(has=p.get_by_role('checkbox', name=name, exact=True))

def audit(peer):
    path = Path(peer.data['auth_audit'])
    return path.read_bytes() if path.exists() else b''

def report(sampled, keys=('disk','inodes','services','ports')):
    values = dict(disk=('warning','92%'), inodes=('pass','45%'), services=('pass','0'), ports=('info','0'))
    lines = [f'ARCNIX_DIAG_START_V1\t{sampled}\t{len(keys)}']
    for key in keys:
        level, value = values[key]
        lines.append(f'ARCNIX_DIAG_V1\t{key}\t{level}\t{value}\t'+base64.b64encode(b'local saved detail').decode())
    return '\n'.join(lines+[f'ARCNIX_DIAG_END_V1\t{len(keys)}',''])

def task(profile, fingerprint, identifier, sampled=now-100, keys=('disk','inodes','services','ports'), **changes):
    target = dict(id=profile['id'],name=profile['name'],endpoint=f"{profile['username']}@{profile['host']}:{profile['port']}",
        fingerprint=fingerprint,status='Succeeded',preview='',output=report(sampled,keys),exit_code=0,
        remote_directory='/home/arcnix/.local/state/arcnix/tasks/'+identifier,result_read_at=now-10)
    target.update(changes)
    return dict(id=identifier,name='Saved diagnostics',script_id='builtin:diagnostics:v1',revision='saved',created=now-200,
        diagnostic={key:key in keys for key in ['disk','inodes','services','ports']},targets=[target])

def policy(identifier, servers):
    # Field order matches Policy serialization, including its default null database.
    return dict(id=identifier,name=identifier,repository='/backup/'+identifier,password_file='/secrets/restic',source='/srv/site',
        database=None,excludes=[],servers=servers,administrator=False,keep_last=7)

def evidence(policy, executor, fingerprint, kind, origin=None):
    revision=hashlib.sha256(json.dumps(policy,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()[:16]
    c=dict(task='proof-'+kind,started=now-300,confirmed=now-200,status='Succeeded',exit_code=0,
        endpoint=f"{executor['username']}@{executor['host']}:{executor['port']}",fingerprint=fingerprint,revision=revision,repository=policy['repository'],
        snapshot='a'*64,destination='/restore/new',paths=[],origin_server=origin,drill_checks=1,drill_revision='abcdef0123456789')
    return dict(policy=policy['id'],server=executor['id'],kind=kind,latest=copy.deepcopy(c),last_success=c)

def restart(app, config=None, history=None):
    app.stop()
    if config is not None: app.config.write_text(json.dumps(config),encoding='utf-8')
    if history is not None: app.config.with_suffix('.tasks.json').write_text(json.dumps(history),encoding='utf-8')
    app.launch(); return app.page

def help_text(p, button, needle):
    button.scroll_into_view_if_needed()
    p.evaluate('() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))')
    p.mouse.move(p.viewport_size['width']-10,p.viewport_size['height']-10)
    button.hover(); expect(p.get_by_role('tooltip')).to_contain_text(needle)
    button.click(); p.mouse.move(p.viewport_size['width']-10,p.viewport_size['height']-10)
    expect(p.get_by_role('tooltip')).to_contain_text(needle)
    box=p.get_by_role('tooltip').bounding_box()
    assert box['x']>=10 and box['x']+box['width']<=p.viewport_size['width']-10
    assert box['y']>=10 and box['y']+box['height']<=p.viewport_size['height']-10
    p.keyboard.press('Escape')

def metric_queries(peer):
    path=peer.root/'metrics-queries.jsonl'
    return path.read_bytes() if path.exists() else b''

with Peer(out/'alpha') as a, Peer(out/'beta') as b:
    for peer in [a,b]: (peer.root/'trace-task-queries').touch()
    local=out/'local'; local.mkdir(parents=True)
    (local/'overview.txt').write_text('overview preserves SFTP\n',encoding='utf-8')
    alpha=a.profile('Alpha',environment='Production',tags=['Web'])
    beta=b.profile('Beta',environment='Staging',tags=['WebApp'])
    extra=[]
    for n,name in enumerate(['Missing','Pending','Clock','Partial','Broken']):
        item=a.profile(name,environment='Production',tags=['Web']);item.update(host=f'127.0.0.{n+3}',port=22);extra.append(item)
    untrusted=a.profile('Untrusted'); untrusted.update(host='127.0.0.20',port=22)
    alias=dict(alpha,id='Alias',name='Alias',username='other')
    config=a.config([alpha,beta,*extra,untrusted,alias],local)
    host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    fp=a.data['host_key_rsa_fingerprint']
    config['config']['trusted_hosts'] += [dict(host=x['host'],port=x['port'],fingerprint=fp) for x in extra]
    policies=[policy('Website',['Alpha']),policy('Uploads',['Alpha'])]
    backup=evidence(policies[0],alpha,fp,'Backup');backup['latest'].update(task='failed-later',started=now-100,confirmed=now-50,status='Failed',exit_code=1)
    config['config']['backup_policies']=policies
    config['config']['backup_evidence']=[backup,evidence(policies[0],alpha,fp,'IntegrityCheck'),evidence(policies[0],beta,b.data['host_key_rsa_fingerprint'],'RecoveryDrill','Alpha')]
    history=[task(alpha,fp,'alpha-good'),task(beta,b.data['host_key_rsa_fingerprint'],'beta-old',sampled=now-9*86400),
        task(extra[1],fp,'pending-old'),task(extra[1],fp,'pending-new',status='Running'),task(extra[2],fp,'clock',sampled=now+86400),
        task(extra[3],fp,'partial',keys=('disk',)),task(extra[4],fp,'broken',output='ARCNIX_DIAG_START_V1\t1\t4\n')]
    history[3]['created']+=1
    with Desktop(out/'desktop',config) as app:
        p=restart(app,history=history)
        before=(audit(a),audit(b))
        expect(p.locator('.server-inventory')).to_be_visible();expect(p.locator('.inventory-row')).to_have_count(9)
        expect(row(p,'Alpha').locator('.inventory-metrics')).to_have_text('Disk 92% · Inodes 45% · Failed services 0')
        expect(row(p,'Beta')).to_contain_text('Record outdated')
        expect(row(p,'Missing')).to_contain_text('No matching diagnostic record')
        expect(row(p,'Pending')).to_contain_text('Diagnostic result unconfirmed');expect(row(p,'Pending').locator('.inventory-metrics')).to_have_count(0)
        expect(row(p,'Clock')).to_contain_text('Diagnostic record time needs review')
        expect(row(p,'Partial')).to_contain_text('Disk 92% · Inodes — · Failed services —');expect(row(p,'Partial')).to_contain_text('Partial core checks')
        expect(row(p,'Broken')).to_contain_text('Diagnostic report incomplete or mismatched');expect(row(p,'Broken').locator('.inventory-metrics')).to_have_count(0)
        expect(row(p,'Untrusted').get_by_role('checkbox')).to_be_disabled()
        expect(p.locator('.inventory-counts')).to_have_text('Matching servers 9 · Recorded findings 3 · Need verification 9')
        p.get_by_role('button',name='30 days',exact=True).click();expect(row(p,'Beta').get_by_text('Record outdated',exact=True)).to_have_count(0)
        p.get_by_role('button',name='7 days',exact=True).click();p.get_by_role('button',name='Refresh local overview',exact=True).click()
        help_text(p,p.get_by_role('button',name='Help: Server overview',exact=True),'local records only')
        assert (audit(a),audit(b))==before
        passed('Opening, refreshing and changing age remain local; exact numeric observations, missing coverage, stale records, pending newest runs, future clocks and malformed reports remain distinct')

        expect(row(p,'Alpha').locator('.inventory-backup-coverage')).to_have_text('Recorded backups: 1/2')
        expect(row(p,'Alpha')).to_contain_text('A latest backup attempt failed')
        help_text(p,row(p,'Alpha').get_by_role('button',name='Help: Backup records',exact=True),'Recovery drill:')
        row(p,'Alpha').get_by_role('button',name='Help: Backup records',exact=True).click()
        expect(p.get_by_role('tooltip')).to_contain_text('Beta');expect(p.get_by_role('tooltip')).to_contain_text('Uploads')
        p.keyboard.press('Escape')
        row(p,'Alpha').get_by_role('button',name='View diagnostic task',exact=True).click()
        expect(p.locator('.task-target')).to_have_count(1);expect(p.locator('.task-target')).to_contain_text('Alpha')
        assert (audit(a),audit(b))==before
        p.locator('.rail-servers').click()
        changed=copy.deepcopy(config);changed['config']['trusted_hosts'][0]['fingerprint']=b.data['host_key_rsa_fingerprint']
        p=restart(app,changed,history)
        expect(row(p,'Alpha')).to_contain_text('No matching diagnostic record');expect(row(p,'Alpha').locator('.inventory-backup-coverage')).to_have_text('Recorded backups: 0/2')
        p=restart(app,config,history)
        passed('Every assigned policy counts, later failed backups remain visible, checks and explicit cross-server drills remain separate, task shortcuts are local, and SSH fingerprint drift invalidates saved proof')

        p.get_by_role('textbox',name='Environment filter',exact=True).fill('production')
        p.get_by_role('textbox',name='Tag filter',exact=True).fill('web')
        expect(p.locator('.inventory-row')).to_have_count(7)
        row(p,'Alpha').get_by_role('checkbox').check();p.get_by_role('textbox',name='Search',exact=True).fill('Beta')
        expect(p.locator('.inventory-selected')).to_have_text('Selected 1 · Hidden selections 1')
        expect(p.locator('.inventory-empty')).to_be_visible()
        p.get_by_role('button',name='Review diagnostics…',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        expect(d.locator('.target-choice.active')).to_have_count(1);expect(d.locator('.script-target-picker')).to_have_count(1);expect(d).to_contain_text('Alpha')
        assert (audit(a),audit(b))==before
        d.get_by_role('button',name='Cancel',exact=True).click()
        p.get_by_role('button',name='Clear overview filters',exact=True).click()
        row(p,'Alias').get_by_role('checkbox').click();expect(p.locator('.inventory-error')).to_contain_text('one account per server endpoint')
        expect(row(p,'Alias').get_by_role('checkbox')).not_to_be_checked();expect(p.locator('.inventory-selected')).to_have_text('Selected 1 · Hidden selections 0')
        p.get_by_role('button',name='Clear selection',exact=True).click()
        p.get_by_role('button',name='Recorded findings',exact=True).click();expect(p.locator('.inventory-row')).to_have_count(3)
        p.get_by_role('button',name='All matches',exact=True).click()
        key=row(p,'Alpha').get_by_role('checkbox');key.focus();key.press('Space');expect(key).to_be_checked();key.press('Space');expect(key).not_to_be_checked()
        passed('Exact environment/tag filters combine with search; hidden selections survive and review reveals their complete scope before SSH; duplicate endpoints reject atomically and finding categories keep their own counts')

        # Fresh runs use real SSH request paths, then survive terminating the desktop.
        row(p,'Alpha').get_by_role('checkbox').check();row(p,'Beta').get_by_role('checkbox').check()
        p.get_by_role('button',name='Review diagnostics…',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        expect(d.locator('.target-choice.active')).to_have_count(2);d.get_by_role('button',name='Check targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=20000)
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Running','Running'])
        app.stop();time.sleep(4.5);app.launch();p=app.page
        expect(row(p,'Alpha')).to_contain_text('Diagnostic result unconfirmed')
        row(p,'Alpha').get_by_role('button',name='View diagnostic task',exact=True).click()
        for n in range(2):
            p.locator('.task-target').nth(n).get_by_role('button',name='Query server result',exact=True).click()
            p.get_by_role('dialog',name='Query server result',exact=True).get_by_role('button',name='Query',exact=True).click()
            expect(p.locator('.task-status').nth(n)).to_have_text('Succeeded',timeout=20000)
        p.locator('.rail-servers').click()
        expect(row(p,'Alpha').locator('.inventory-metrics')).to_have_text('Disk 92% · Inodes 98% · Failed services 1')
        expect(row(p,'Beta').locator('.inventory-metrics')).to_have_text('Disk 92% · Inodes 98% · Failed services 1')
        assert len((a.root/'task-launches.jsonl').read_text().splitlines())==1 and len((b.root/'task-launches.jsonl').read_text().splitlines())==1
        passed('Default four-check batch reviews and submits exactly the selected two servers over SSH, survives desktop termination and only updates the overview after explicit server-result queries')

        row(p,'Alpha').get_by_role('button',name='Open workspace',exact=True).click()
        p.locator('.overview-connect').click();p.get_by_role('dialog',name='Connect to server',exact=True).get_by_role('button',name='Connect',exact=True).click()
        expect(p.locator('.connection-state')).to_have_text('SSH connected')
        p.get_by_role('button',name='Terminal',exact=True).click()
        reader=p.locator('.terminal-surface .xterm-accessibility-tree');expect(reader).to_contain_text('Arcnix terminal fixture')
        p.locator('.xterm-helper-textarea').focus();p.keyboard.type('echo overview-session-preserved');p.keyboard.press('Enter');expect(reader).to_contain_text('overview-session-preserved')
        before_auth=audit(a)
        p.locator('.server-inventory-open').click();expect(p.locator('.terminal-module')).to_be_hidden();expect(p.locator('.connection-state')).to_have_count(0)
        p.get_by_role('button',name='Current workspace',exact=True).click();expect(reader).to_contain_text('overview-session-preserved');assert audit(a)==before_auth
        p.get_by_role('button',name='Files',exact=True).click();p.locator('.local-slot').get_by_role('button',name='overview.txt',exact=True).click();p.locator('.local-slot').get_by_role('button',name='Upload',exact=True).click()
        expect(p.locator('.queue-row').last).to_contain_text('Completed');assert (a.root/'overview.txt').read_text()=='overview preserves SFTP\n'
        (a.root/'metrics-fixture.json').write_text(json.dumps(dict(trace=True)))
        p.get_by_role('button',name='Resources',exact=True).click();eventually(lambda:len(json.loads(p.locator('.resource-trend-data').text_content())['points'])>=1)
        p.get_by_role('button',name='Refresh resource values',exact=True).click();expect(p.get_by_role('button',name='Refresh resource values',exact=True)).to_be_enabled()
        p.locator('.server-inventory-open').click();count=metric_queries(a);time.sleep(3.8);assert metric_queries(a)==count
        p.get_by_role('button',name='Current workspace',exact=True).click();eventually(lambda:metric_queries(a)!=count)
        points=json.loads(p.locator('.resource-trend-data').text_content())['points'];assert points[-1]['break_before'] and points[-1]['cpu'] is None
        p.get_by_role('button',name='Pause sampling',exact=True).click()
        passed('Overview navigation preserves the same live terminal and functional SFTP; resource sampling stops while hidden and resumes with a gap rather than bridging the interruption')

        app.settings('General');p.get_by_role('button',name='Dark',exact=True).click();p.get_by_role('button',name='简体中文',exact=True).click()
        p.locator('.rail-servers').click();p.locator('.server-inventory-open').click();p.set_viewport_size(dict(width=900,height=700))
        expect(p.locator('.page-header-title')).to_have_text('服务器总览')
        expect(row(p,'Alpha').locator('.inventory-metrics')).to_contain_text('故障服务 1')
        help_text(p,p.get_by_role('button',name='帮助：服务器总览',exact=True),'本地最近十批')
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert p.locator('.server-inventory').evaluate('el=>el.scrollWidth<=el.clientWidth')
        assert not p.locator('.server-inventory *:visible').evaluate_all("els=>els.filter(el=>!el.children.length&&el.textContent.trim()&&parseFloat(getComputedStyle(el).fontSize)<12).map(el=>el.textContent)")
        assert row(p,'Alpha').locator('.inventory-select').evaluate("el=>getComputedStyle(el).color===getComputedStyle(document.querySelector('.app')).color")
        box=row(p,'Alpha').get_by_role('checkbox').bounding_box();assert abs(box['width']-18)<.05 and abs(box['height']-18)<.05
        app.screenshot('inventory-chinese-dark-900.png')
        p.locator('.settings-link').click();p.locator('.context-links').get_by_role('button',name='常规',exact=True).click();p.get_by_role('button',name='English',exact=True).click();p.get_by_role('button',name='Light',exact=True).click();p.locator('.rail-servers').click()
        app.screenshot('inventory-english-light-900.png')
        passed('Chinese and English, dark and light, a 900 px window and hover/click help retain readable type, bounded tooltips and no horizontal overflow')

        many=copy.deepcopy(config);many['config']['servers']=[];many['config']['trusted_hosts']=[];many['config']['backup_policies']=[];many['config']['backup_evidence']=[]
        for n in range(55):
            x=a.profile(f'Node {n:02}',environment='Production' if n<32 else 'Staging');x.update(host=f'node-{n}.test',port=22)
            if n==0:x['name']+=' — '+('中文 long name '*10)
            many['config']['servers'].append(x);many['config']['trusted_hosts'].append(dict(host=x['host'],port=22,fingerprint=fp))
        p=restart(app,many,[]);expect(p.locator('.inventory-row')).to_have_count(50);expect(p.locator('.inventory-page-range')).to_have_text('1–50 / 55')
        p.get_by_role('button',name='Select matching trusted servers',exact=True).click();expect(p.locator('.inventory-error')).to_contain_text('32');expect(p.locator('.inventory-selected')).to_have_text('Selected 0 · Hidden selections 0')
        p.get_by_role('button',name='More servers',exact=True).click();expect(p.locator('.inventory-row')).to_have_count(5);expect(p.locator('.inventory-page-range')).to_have_text('51–55 / 55')
        row(p,'Node 54').get_by_role('checkbox').check();p.get_by_role('button',name='Previous servers',exact=True).click();expect(p.locator('.inventory-selected')).to_have_text('Selected 1 · Hidden selections 1')
        p.get_by_role('button',name='Review selected servers',exact=True).click();expect(p.locator('.inventory-row')).to_have_count(1);expect(row(p,'Node 54').get_by_role('checkbox')).to_be_checked()
        p.get_by_role('button',name='Clear selection',exact=True).click();p.get_by_role('button',name='Clear overview filters',exact=True).click()
        p.get_by_role('textbox',name='Environment filter',exact=True).fill('Production');p.get_by_role('button',name='Select matching trusted servers',exact=True).click()
        expect(p.locator('.inventory-selected')).to_have_text('Selected 32 · Hidden selections 0')
        p.get_by_role('button',name='Review diagnostics…',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        expect(d.locator('.target-choice.active')).to_have_count(32);d.get_by_role('button',name='Cancel',exact=True).click()
        passed('Pagination bounds rendering to 50 rows, preserves selection across pages and atomically rejects an over-limit select-all; 32 exact trusted targets remain reviewable without contacting synthetic hosts')

        empty=copy.deepcopy(many);empty['config']['servers']=[];empty['config']['trusted_hosts']=[]
        p=restart(app,empty,[]);expect(p.locator('.server-landing')).to_be_visible();expect(p.locator('.server-inventory')).to_have_count(0)
        p.locator('.server-inventory-open').click();expect(p.locator('.server-inventory')).to_be_visible();expect(p.locator('.inventory-empty')).to_be_visible()
        expect(p.get_by_role('button',name='Review diagnostics…',exact=True)).to_be_disabled()
        passed('A new empty configuration keeps the existing add-server landing; an explicitly opened empty overview remains usable and cannot submit a batch')

(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8')
print(f'{len(checks)} groups passed; artifacts: {out}',flush=True)
