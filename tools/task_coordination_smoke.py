"""Cross-client changing-operation exclusion and read-only diagnostics in WebView."""
import json, time
from desktop_fixture import Peer, Desktop, expect, ROOT

out = ROOT/'artifacts/task-coordination-ui'


def prepare_install(app):
    p = app.page
    p.locator('.rail-scripts').click()
    p.locator('.context-links').get_by_role('button', name='Official scripts', exact=True).click()
    p.locator('.script-row').filter(has_text='Install Nginx').get_by_role('button', name='Run on servers…', exact=True).click()
    dialog = p.get_by_role('dialog', name='Run script on servers', exact=True)
    dialog.locator('.target-choice').first.click()
    dialog.get_by_role('button', name='Check targets', exact=True).click()


def query_selected(app):
    p = app.page
    p.locator('.task-target').first.get_by_role('button', name='Query server result', exact=True).click()
    p.get_by_role('dialog', name='Query server result', exact=True).get_by_role('button', name='Query', exact=True).click()


with Peer(out/'peer') as peer:
    config = peer.config([peer.profile('Alpha')], out)
    config['config']['snippets'] = [dict(id='hold', name='Hold operation', description='Fixture-held server task', command='echo fixture-hold-operation', administrator=False)]
    with Desktop(out/f'first-{time.time_ns()}', config) as first, Desktop(out/f'second-{time.time_ns()}', config, runtime=first.runtime) as second:
        prepare_install(second)
        expect(second.page.locator('.task-status')).to_have_text(['Ready'], timeout=20000)
        p = first.page
        p.locator('.rail-scripts').click()
        p.locator('.context-links').get_by_role('button', name='My scripts', exact=True).click()
        p.locator('.script-row').filter(has_text='Hold operation').get_by_role('button', name='Run on servers…', exact=True).click()
        dialog = p.get_by_role('dialog', name='Run script on servers', exact=True)
        expect(dialog).to_contain_text('Server locks survive client closure')
        dialog.locator('.target-choice').first.click()
        dialog.get_by_role('button', name='Check targets', exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready'], timeout=20000)
        p.get_by_role('button', name='Execute ready targets', exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Running'], timeout=20000)
        first.stop(); first.launch(); p = first.page
        p.locator('.rail-tasks').click(); p.locator('.task-heading').first.click()
        expect(p.locator('.task-status')).to_have_text(['Result unknown'])
        query_selected(first)
        expect(p.locator('.task-status')).to_have_text(['Running'], timeout=20000)
        # A fresh client cannot rely on its empty in-memory controller. The
        # server preflight must reject the operation still running after exit.
        prepare_install(first)
        expect(p.locator('.task-status')).to_have_text(['Failed'], timeout=20000)
        expect(p.locator('.task-output')).to_contain_text('Another Arcnix server operation is running')
        first.screenshot('restart-conflict.png')
        # The other client's already-approved plan must also be rejected at
        # actual execution, closing the gap between preflight and submission.
        second.page.get_by_role('button', name='Execute ready targets', exact=True).click()
        expect(second.page.locator('.task-status')).to_have_text(['Failed'], timeout=20000)
        expect(second.page.locator('.task-target')).to_contain_text('Exit code: 75')
        expect(second.page.locator('.task-output')).to_contain_text('Another Arcnix server operation is running')
        second.screenshot('prepared-race-rejected.png')
        p.locator('.rail-diagnostics').click()
        p.get_by_role('textbox', name='PEM certificate path (optional)', exact=True).fill('/etc/fixture.pem')
        p.get_by_role('button', name='Nginx', exact=True).click()
        p.get_by_role('button', name='Choose servers and review checks…', exact=True).click()
        dialog = p.get_by_role('dialog', name='Run script on servers', exact=True)
        dialog.locator('.target-choice').first.click()
        dialog.get_by_role('button', name='Check targets', exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready'], timeout=20000)
        p.get_by_role('button', name='Execute ready targets', exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Succeeded'], timeout=20000)
        p.locator('.task-heading').filter(has_text='Hold operation').click()
        query_selected(first)
        expect(p.locator('.task-status')).to_have_text(['Running'], timeout=20000)
        (peer.root/'release-held-operation').touch()
        query_selected(first)
        expect(p.locator('.task-status')).to_have_text(['Succeeded'], timeout=20000)
        prepare_install(first)
        expect(p.locator('.task-status')).to_have_text(['Ready'], timeout=20000)
        p.get_by_role('button', name='Execute ready targets', exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Succeeded'], timeout=20000)
        first.settings('General'); p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check()
        expect(p.locator('.page-header-title')).to_have_text('常规')
        p.locator('.rail-scripts').click()
        expect(p.locator('.page-header-title')).to_have_text('运维脚本')
        p.locator('.script-row').filter(has_text='Nginx').get_by_role('button', name='在服务器上运行…', exact=True).click()
        expect(p.get_by_role('dialog')).to_contain_text('客户端关闭后服务器锁仍有效')
        p.set_viewport_size(dict(width=900, height=700))
        assert p.evaluate('document.documentElement.scrollWidth <= innerWidth')
        first.screenshot('coordination-chinese.png')
        saved = first.config.read_text()+first.config.with_suffix('.tasks.json').read_text()
        assert 'fixture-admin-secret' not in saved
        assert len(json.loads(first.config.with_suffix('.tasks.json').read_text())) == 4

(out/'results.json').write_text(json.dumps(dict(passed=True, checks=['server lock hint', 'old result queried after restart', 'fresh preflight rejects running old task', 'second client prepared-plan race rejected', 'conflict has a confirmed nonzero result', 'diagnostics run alongside changing task', 'read-only task leaves original operation running', 'lock release permits a fresh reviewed operation', 'Chinese narrow layout and credential privacy']), indent=2))
print('9 task coordination UI checks passed')
