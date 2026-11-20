"""Workspace package/service/command entries use durable global server tasks."""
import json, time
import re
from desktop_fixture import Peer, Desktop, expect, ROOT

out = ROOT/'artifacts/workspace-tasks-ui'

def enter_workspace(app, index=1, module='Operations'):
    app.servers()
    p=app.page
    p.locator('.host-select').nth(index).click()
    expect(p.locator('.host-row').nth(index)).to_have_class(re.compile('selected'))
    expect(p.locator('.connection-state')).to_be_visible()
    if p.locator('.overview-connect').is_visible():
        app.connect(index)
    p.get_by_role('button', name=module, exact=True).click()
    return p

def check(app, status='Ready'):
    dialog=app.page.get_by_role('dialog', name='Run script on servers', exact=True)
    dialog.get_by_role('button', name='Check targets', exact=True).click()
    expect(app.page.locator('.task-status')).to_have_text([status], timeout=20000)

def execute(app, status='Succeeded'):
    app.page.get_by_role('button', name='Execute ready targets', exact=True).click()
    expect(app.page.locator('.task-status')).to_have_text([status], timeout=20000)

def query(app):
    app.page.locator('.task-target').first.get_by_role('button', name='Query server result', exact=True).click()
    app.page.get_by_role('dialog', name='Query server result', exact=True).get_by_role('button', name='Query', exact=True).click()

def services(app):
    p=enter_workspace(app)
    p.get_by_role('button', name='Services', exact=True).click()
    p.get_by_role('button', name='Refresh services', exact=True).click()
    expect(p.locator('.service-row')).to_have_count(3)
    return p

with Peer(out/'peer-a') as a, Peer(out/'peer-b') as b:
    profiles=[a.profile('Alpha'),b.profile('Beta'),a.profile('No systemd')]
    profiles[-1]['username']='arcnix-nosystemd'
    config=a.config(profiles,out)
    host,port=b.data['address'].rsplit(':',1)
    config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    config['config']['snippets']=[dict(id='hold',name='Hold operation',description='Fixture server task',command='echo fixture-hold-operation',administrator=False),dict(id='param',name='Safe parameter',description='Literal input',command="printf '%s\\n' {{message}}",administrator=False)]
    with Desktop(out/f'desktop-{time.time_ns()}',config) as app:
        p=enter_workspace(app,0,'App Center')
        p.get_by_role('textbox',name='Search packages',exact=True).fill('nginx')
        p.get_by_role('button',name='Search',exact=True).click()
        p.get_by_role('button',name='Preview install nginx',exact=True).click()
        dialog=p.get_by_role('dialog',name='Run script on servers',exact=True)
        expect(dialog.locator('.target-choice.active')).to_have_count(1)
        expect(dialog.locator('.target-choice.active')).to_contain_text('Alpha')
        dialog.locator('.target-choice').nth(1).click()
        dialog.get_by_role('button',name='Check targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=20000)
        expect(p.locator('.task-target').first).to_contain_text('Inst nginx')
        app.screenshot('package-plan.png')
        p.get_by_role('button',name='Execute ready targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Running','Running'],timeout=20000)
        app.servers()
        expect(p.get_by_role('button',name='Disconnect',exact=True)).to_be_enabled()
        p.get_by_role('button',name='Disconnect',exact=True).click()
        expect(p.locator('.host-select').nth(1)).to_be_enabled()
        p.locator('.host-select').nth(1).click()
        app.stop();app.launch();p=app.page
        p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        expect(p.locator('.task-status')).to_have_text(['Result unknown','Result unknown'])
        for index in range(2):
            deadline=time.monotonic()+20
            while True:
                p.locator('.task-target').nth(index).get_by_role('button',name='Query server result',exact=True).click()
                p.get_by_role('dialog',name='Query server result',exact=True).get_by_role('button',name='Query',exact=True).click()
                expect(p.locator('.task-status').nth(index)).not_to_have_text('Result unknown')
                if p.locator('.task-status').nth(index).inner_text()=='Succeeded':break
                assert time.monotonic()<deadline,'Original server task did not finish'
                time.sleep(.25)
        p=enter_workspace(app,1,'App Center')
        p.get_by_role('textbox',name='Search packages',exact=True).fill('nginx')
        p.get_by_role('button',name='Search',exact=True).click()
        expect(p.get_by_role('button',name='Installed nginx',exact=True)).to_be_visible()
        p.get_by_role('button',name='Update package index',exact=True).click()
        dialog=p.get_by_role('dialog',name='Run script on servers',exact=True)
        expect(dialog.locator('.target-choice.active')).to_contain_text('Beta')
        expect(dialog).to_contain_text('APT::Update::Error-Mode=any update')
        check(app);execute(app)
        p=services(app)
        p.get_by_role('textbox',name='Filter services',exact=True).fill('worker')
        expect(p.locator('.service-row')).to_have_count(1)
        p.get_by_role('textbox',name='Filter services',exact=True).fill('')
        p.get_by_role('button',name='worker.service',exact=True).click()
        p.get_by_role('button',name='Status',exact=True).click()
        expect(p.locator('.operations-output')).to_contain_text('ActiveState=inactive')
        p.get_by_role('button',name='Logs',exact=True).click()
        expect(p.get_by_role('textbox',name='Log service unit',exact=True)).to_have_value('worker.service')
        p=services(app);p.get_by_role('button',name='worker.service',exact=True).click()
        p.get_by_role('button',name='Start…',exact=True).click()
        dialog=p.get_by_role('dialog',name='Run script on servers',exact=True)
        expect(dialog).to_contain_text('worker.service')
        expect(dialog).to_contain_text('may interrupt connections')
        dialog.get_by_role('button',name='Cancel',exact=True).click()
        p.get_by_role('button',name='Status',exact=True).click()
        expect(p.locator('.operations-output')).to_contain_text('ActiveState=inactive')
        for label,state in [('Start…','active'),('Stop…','inactive'),('Restart…','active')]:
            p.get_by_role('button',name=label,exact=True).click()
            check(app);execute(app)
            p=services(app)
            expect(p.get_by_role('button',name='worker.service',exact=True)).to_contain_text(state+' ·')
            p.get_by_role('button',name='worker.service',exact=True).click()
        app.screenshot('service-state.png')
        p.get_by_role('button',name='Commands',exact=True).click()
        p.get_by_role('button',name='New command',exact=True).click()
        editor=p.get_by_role('dialog',name='Save command',exact=True)
        editor.get_by_role('button',name='Save command',exact=True).click()
        expect(editor.locator('.form-error')).to_be_visible()
        editor.get_by_role('textbox',name='Command name',exact=True).fill('Added command')
        editor.get_by_role('textbox',name='Shell command',exact=True).fill('fixture-command')
        editor.get_by_role('button',name='Save command',exact=True).click()
        row=p.locator('.snippet-row').filter(has_text='Added command')
        row.get_by_role('button',name='Run command…',exact=True).click()
        expect(p.get_by_role('dialog',name='Run script on servers',exact=True).locator('.target-choice.active')).to_contain_text('Beta')
        check(app);execute(app)
        p=enter_workspace(app);p.get_by_role('button',name='Commands',exact=True).click()
        row=p.locator('.snippet-row').filter(has_text='Added command');row.hover()
        row.get_by_role('button',name='Edit command',exact=True).click()
        editor=p.get_by_role('dialog',name='Save command',exact=True)
        editor.get_by_role('textbox',name='Shell command',exact=True).fill('fixture-command-fail')
        editor.get_by_role('button',name='Save command',exact=True).click()
        row.get_by_role('button',name='Run command…',exact=True).click()
        check(app);execute(app,'Failed')
        expect(p.locator('.task-target')).to_contain_text('Exit code: 7')
        p=enter_workspace(app);p.get_by_role('button',name='Commands',exact=True).click()
        row=p.locator('.snippet-row').filter(has_text='Added command');row.hover()
        row.get_by_role('button',name='Delete command',exact=True).click()
        p.get_by_role('dialog',name='Delete command',exact=True).get_by_role('button',name='Cancel',exact=True).click()
        expect(row).to_have_count(1)
        row.hover();row.get_by_role('button',name='Delete command',exact=True).click()
        p.get_by_role('dialog',name='Delete command',exact=True).get_by_role('button',name='Delete command',exact=True).click()
        expect(row).to_have_count(0)
        p.locator('.snippet-row').filter(has_text='Safe parameter').get_by_role('button',name='Run command…',exact=True).click()
        dialog=p.get_by_role('dialog',name='Run script on servers',exact=True)
        dialog.get_by_role('textbox',name='message',exact=True).fill("literal '; $(id)")
        check(app)
        expect(p.locator('.task-target')).to_contain_text("literal '\\''; $(id)")
        p.get_by_role('button',name='Stop pending targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Cancelled'])
        p=enter_workspace(app);p.get_by_role('button',name='Commands',exact=True).click()
        p.locator('.snippet-row').filter(has_text='Hold operation').get_by_role('button',name='Run command…',exact=True).click()
        check(app)
        p.get_by_role('button',name='Execute ready targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Running'],timeout=20000)
        p=services(app);p.get_by_role('button',name='worker.service',exact=True).click()
        expect(p.get_by_role('button',name='Status',exact=True)).to_be_enabled()
        expect(p.get_by_role('button',name='Restart…',exact=True)).to_be_disabled()
        p.get_by_role('button',name='Status',exact=True).click()
        expect(p.locator('.operations-output')).to_contain_text('ActiveState=active')
        app.stop();app.launch();p=app.page
        p=enter_workspace(app,1,'App Center')
        p.get_by_role('button',name='Update package index',exact=True).click()
        check(app,'Failed');expect(p.locator('.task-output')).to_contain_text('Another Arcnix server operation is running')
        p=services(app);p.get_by_role('button',name='worker.service',exact=True).click()
        p.get_by_role('button',name='Stop…',exact=True).click()
        check(app,'Failed');expect(p.locator('.task-output')).to_contain_text('Another Arcnix server operation is running')
        p=enter_workspace(app);p.get_by_role('button',name='Commands',exact=True).click()
        p.locator('.snippet-row').filter(has_text='Hold operation').get_by_role('button',name='Run command…',exact=True).click()
        check(app,'Failed');expect(p.locator('.task-output')).to_contain_text('Another Arcnix server operation is running')
        (b.root/'release-held-operation').touch()
        p.locator('.task-heading').filter(has_text='Hold operation').last.click()
        query(app);expect(p.locator('.task-status')).to_have_text(['Succeeded'],timeout=20000)
        p=enter_workspace(app,2)
        p.get_by_role('button',name='Commands',exact=True).click()
        p.locator('.snippet-row').filter(has_text='Hold operation').get_by_role('button',name='Run command…',exact=True).click()
        check(app,'Failed');expect(p.locator('.task-output')).to_contain_text('require a running systemd')
        app.settings('General');p.get_by_role('button',name='简体中文',exact=True).click()
        expect(p.locator('.page-header-title')).to_have_text('常规')
        p.locator('.rail-servers').click()
        p.get_by_role('button',name='运维操作',exact=True).click()
        expect(p.locator('.operations-page')).to_contain_text('服务变更和保存命令由服务器任务执行')
        p.set_viewport_size(dict(width=900,height=700))
        assert p.evaluate('document.documentElement.scrollWidth <= innerWidth')
        app.screenshot('workspace-tasks-chinese.png')
        data=json.loads(app.config.read_text());assert len(data['config']['snippets'])==2
        assert 'fixture-admin-secret' not in app.config.read_text()+app.config.with_suffix('.tasks.json').read_text()

checks=['workspace install preselects its server','install expands to multiple servers','typed APT simulation','workspace can disconnect while server job runs','switch servers while job runs','restart queries original installation results','installed query observes server completion','index update uses typed package action','service filter and status','service journal navigation','service cancellation does not change state','start stop restart via hosted jobs','saved command validation and creation','plain commands use global task form','command edits and nonzero result persist','command deletion cancellation and confirmation','parameter literal expansion remains shared','read queries remain enabled during hosted work','software entry cannot bypass an old server lock','service entry cannot bypass an old server lock','command entry cannot bypass an old server lock','non-systemd server rejects hosted execution','Chinese narrow layout and credentials excluded']
(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),indent=2))
print(f'{len(checks)} workspace task UI checks passed')
