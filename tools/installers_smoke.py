"""Typed installation scope, per-step checks and recovery through owned SSH peers."""
import json,re,time
from desktop_fixture import Peer,Desktop,expect,ROOT

out=ROOT/'artifacts/installers-ui'
checks=[]
def passed(name):checks.append(name)
def choose(app,upgrade=False):
    p=app.page;p.locator('.rail-scripts').click()
    p.locator('.context-links').get_by_role('button',name='Official scripts',exact=True).click()
    p.locator('.script-row').filter(has=p.get_by_text('Install Nginx',exact=True)).get_by_role('button',name='Run on servers…',exact=True).click()
    d=p.get_by_role('dialog',name='Run script on servers',exact=True)
    if upgrade:d.get_by_role('button',name='Allow reviewed upgrades',exact=True).click()
    return d
def select(d,index):
    target=d.locator('.script-target-picker').nth(index);target.locator('.target-choice').click()
    target.get_by_role('textbox',name='Administrator password (optional)',exact=True).fill('fixture-admin-secret')
def query(app,index,expected):
    p=app.page;p.locator('.task-target').nth(index).get_by_role('button',name='Query server result',exact=True).click()
    p.get_by_role('dialog',name='Query server result',exact=True).get_by_role('button',name='Query',exact=True).click()
    expect(p.locator('.task-status').nth(index)).to_have_text(expected,timeout=20000)

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    config=a.config([a.profile('Alpha'),b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1)
    config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    config['config']['snippets']=[dict(id='builtin:nginx',name='My builtin-prefixed script',description='User content',command='fixture-command',administrator=False)]
    with Desktop(out/f'desktop-{time.time_ns()}',config) as app:
        p=app.page
        assert 'rail-servers' in p.locator('.navigation-rail button').first.get_attribute('class')
        p.locator('.rail-scripts').click()
        expect(p.locator('.script-row')).to_have_count(11)
        nginx=p.locator('.script-row').filter(has=p.get_by_text('Install Nginx',exact=True))
        tip=nginx.get_by_role('button',name='Help: Installation checks',exact=True);tip.scroll_into_view_if_needed()
        p.evaluate('() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');p.mouse.move(1400,40);tip.hover()
        expect(p.get_by_role('tooltip')).to_contain_text('HTTP and external reachability are not checked')
        p.mouse.move(1400,950)
        assert re.search(r'Version [a-f0-9]{16}',nginx.inner_text())
        passed('official revision and explicit applicability scope')
        p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click()
        expect(p.locator('.script-row')).to_have_count(1)
        expect(p.locator('.script-row')).to_contain_text('Shell')
        expect(p.locator('.script-row').get_by_role('button',name='Edit',exact=True)).to_be_visible()
        expect(p.locator('.script-row .help-trigger')).to_have_count(0)
        passed('user IDs cannot acquire official installer presentation')
        d=choose(app);app.screenshot('installation-policy.png')
        d.get_by_role('button',name='Help: Installation policy',exact=True).click()
        expect(p.get_by_role('tooltip')).to_contain_text('changed installed state or dpkg transaction requires a new review')
        p.keyboard.press('Escape');expect(d).to_be_visible()
        expect(d).to_contain_text('no automatic rollback or removal')
        select(d,0);select(d,1);d.get_by_role('button',name='Check targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=30000)
        for row in p.locator('.task-target').all():
            expect(row).to_contain_text('ubuntu 24.04 · amd64')
            expect(row).to_contain_text('Existing package state')
            expect(row).to_contain_text('Transaction SHA-256:')
            expect(row).to_contain_text('Installed-state SHA-256:')
            expect(row).to_contain_text('Not installed')
            expect(row).to_contain_text('Nginx configuration')
            expect(row).not_to_contain_text('ARCNIX_INSTALL_START')
            expect(row).not_to_contain_text('/tmp/arcnix-install.')
        passed('two-host environment existing versions and human review')
        history=json.loads(app.config.with_suffix('.tasks.json').read_text());identifier=history[0]['id']
        assert history[0]['installation']['kind']=='nginx'
        assert history[0]['installation']['version']==2 and history[0]['installation']['policy']=='preserve_installed'
        assert all(t['status']=='Ready' for t in history[0]['targets'])
        passed('frozen installation scope persists before execution')
        (b.root/'installer-check-fail').touch()
        p.get_by_role('button',name='Execute ready targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Running','Running'],timeout=20000)
        app.stop();time.sleep(4.5);app.launch();p=app.page
        p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        expect(p.locator('.task-status')).to_have_text(['Result unknown','Result unknown'])
        query(app,0,'Succeeded');query(app,1,'Failed')
        report=p.locator('.task-target').first.locator('.installation-report')
        expect(report).to_contain_text('Installation checks passed')
        expect(report).to_contain_text('Passed · Nginx configuration')
        failed_report=p.locator('.task-target').nth(1).locator('.installation-report')
        expect(failed_report).to_contain_text('Requested packages are present; installation or checks failed')
        expect(failed_report).to_contain_text('Failed · Nginx configuration')
        expect(failed_report).to_contain_text('Passed · Nginx service')
        assert not p.locator('.task-output').filter(has_text='ARCNIX_INSTALL_').count()
        app.screenshot('two-server-installation-checks.png')
        passed('desktop closure independent completion and separate package application results')
        data=app.config.with_suffix('.tasks.json').read_text()
        assert 'fixture-admin-secret' not in data
        passed('installation evidence contains no credentials')
        (b.root/'installer-check-fail').unlink()
        app.stop();app.config.with_suffix('.tasks.json').unlink();app.launch();p=app.page
        p.locator('.rail-tasks').click();expect(p.locator('.task-entry')).to_have_count(0)
        p.get_by_role('button',name='Server records',exact=True).click()
        d=p.get_by_role('dialog',name='Connect to server records',exact=True)
        d.get_by_role('button',name='Select trusted servers',exact=True).click()
        d.get_by_role('button',name='Read server records',exact=True).click()
        expect(p.locator('.server-record-group')).to_have_count(2)
        for index,title in [(0,'Installation checks passed'),(1,'Requested packages are present; installation or checks failed')]:
            group=p.locator('.server-record-group').nth(index)
            group.locator('.server-task-record').filter(has=group.page.get_by_text(identifier,exact=True)).get_by_role('button',name='View saved log',exact=True).click()
            d=p.get_by_role('dialog',name='Saved server task log',exact=True)
            expect(d.locator('.installation-report')).to_contain_text(title,timeout=20000)
            expect(d.locator('.record-saved-output')).not_to_contain_text('ARCNIX_INSTALL_')
            d.get_by_role('button',name='Close',exact=True).click()
        passed('server record report survives loss of all local history')
        # Fresh tasks inspect the real fixture's changed existing-package state.
        d=choose(app);select(d,0);d.get_by_role('button',name='Check targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready'],timeout=20000)
        expect(p.locator('.task-target')).to_contain_text('Installed · 1.24.0')
        p.get_by_role('button',name='Stop pending targets',exact=True).click()
        passed('repeated preflight displays installed version')
        for mode,needle in [
            ('environment-change','Skipped · APT installation'),
            ('apt-fail','Failed · APT installation'),
            ('service-fail','Failed · Nginx service'),
            ('report-incomplete','Installation verification is incomplete or inconsistent'),
            ('report-wrong-id','Installation verification is incomplete or inconsistent'),
        ]:
            d=choose(app,upgrade=mode=='apt-fail');select(d,0);d.get_by_role('button',name='Check targets',exact=True).click()
            expect(p.locator('.task-status')).to_have_text(['Ready'],timeout=20000)
            flag=a.root/('installer-'+mode);flag.touch()
            p.get_by_role('button',name='Execute ready targets',exact=True).click()
            expected='Failed' if mode in ['environment-change','apt-fail','service-fail'] else 'Result unknown'
            expect(p.locator('.task-status').first).to_have_text(expected,timeout=20000)
            expect(p.locator('.task-target').first.locator('.installation-report')).to_contain_text(needle)
            if mode.startswith('report-'):expect(p.locator('.task-target').first.locator('.installation-report')).not_to_contain_text('Installation checks passed')
            flag.unlink();passed(mode+' is reported without inventing verification')
        for mode,needle in [('unsupported','Installation checks require Debian/Ubuntu'),('before-truncated','Incomplete installed package response')]:
            flag=a.root/('installer-'+mode);flag.touch();d=choose(app);select(d,0)
            d.get_by_role('button',name='Check targets',exact=True).click()
            expect(p.locator('.task-status').first).to_have_text('Failed',timeout=20000)
            expect(p.locator('.task-target').first).to_contain_text(needle)
            expect(p.get_by_role('button',name='Execute ready targets',exact=True)).to_have_count(0)
            flag.unlink();passed(mode+' preflight prevents submission')
        for mode,needle in [('dependency-upgrade','select Allow reviewed upgrades'),('plan-remove','removes existing packages'),('plan-malformed','target architecture'),('transaction-truncated','Incomplete APT simulation')]:
            flag=a.root/('installer-'+mode);flag.touch();d=choose(app);select(d,0);d.get_by_role('button',name='Check targets',exact=True).click()
            expect(p.locator('.task-status').first).to_have_text('Failed',timeout=20000);expect(p.locator('.task-target').first).to_contain_text(needle);flag.unlink();passed(mode+' cannot silently change the reviewed scope')
        flag=a.root/'installer-dependency-upgrade';flag.touch();d=choose(app,upgrade=True);select(d,0);d.get_by_role('button',name='Check targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready'],timeout=20000);expect(p.locator('.task-target')).to_contain_text('Inst dependency [1.0] (2.0')
        assert json.loads(app.config.with_suffix('.tasks.json').read_text())[0]['installation']['policy']=='allow_upgrades'
        p.get_by_role('button',name='Stop pending targets',exact=True).click();flag.unlink();passed('explicit upgrade policy reviews dependency versions and persists the selected policy')
        flag=a.root/'installer-downgrade';flag.touch();d=choose(app,upgrade=True);select(d,0);d.get_by_role('button',name='Check targets',exact=True).click()
        expect(p.locator('.task-status').first).to_have_text('Failed',timeout=20000);expect(p.locator('.task-target').first).to_contain_text('downgrade');flag.unlink();passed('explicit upgrade permission does not permit a downgrade')
        for mode,needle,expected in [('state-change','Failed · Reviewed APT transaction','Failed'),('plan-change','Failed · Reviewed APT transaction','Failed'),('protocol-v2','Failed · Reviewed APT transaction','Failed'),('unconfirmed-plan','Installation verification is incomplete or inconsistent','Result unknown')]:
            d=choose(app,upgrade=True);select(d,0);d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready'],timeout=20000)
            flag=a.root/('installer-'+mode);flag.touch();p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status').first).to_have_text(expected,timeout=20000)
            expect(p.locator('.task-target').first.locator('.installation-report')).to_contain_text(needle);flag.unlink();passed(mode+' blocks a false successful installation result')
        app.stop()
        settings=json.loads(app.config.read_text(encoding='utf-8'))
        profile=a.profile('Credential word');profile.update(username='arcnix-word',auth='Password',private_key=None)
        settings['config']['servers'].append(profile);app.config.write_text(json.dumps(settings),encoding='utf-8')
        app.launch();p=app.page;d=choose(app);select(d,2)
        d.locator('.script-target-picker').nth(2).get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('passed')
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status').first).to_have_text('Ready',timeout=20000)
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status').first).to_have_text('Succeeded',timeout=20000)
        report=p.locator('.task-target').first.locator('.installation-report')
        expect(report).to_contain_text('Installation checks passed')
        expect(report).to_contain_text('Fixture check [redacted]')
        expect(report).not_to_contain_text('Fixture check passed')
        app.stop();app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click()
        p.locator('.task-target').first.get_by_role('button',name='Query server result',exact=True).click()
        d=p.get_by_role('dialog',name='Query server result',exact=True);d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('passed')
        d.get_by_role('button',name='Query',exact=True).click();expect(p.locator('.task-status').first).to_have_text('Succeeded',timeout=20000)
        expect(p.locator('.task-target').first.locator('.installation-report')).to_contain_text('Fixture check [redacted]')
        passed('status-word credentials redact decoded details and preserve restart verification')
        app.settings('General');p.get_by_role('button',name='简体中文',exact=True).click()
        expect(p.locator('.page-header-title')).to_have_text('常规')
        p.locator('.rail-scripts').click();expect(p.locator('.script-row').first).to_contain_text('检查所需软件包')
        p.set_viewport_size(dict(width=900,height=700))
        assert p.evaluate('document.documentElement.scrollWidth <= innerWidth')
        app.screenshot('installation-chinese-narrow.png')
        p.locator('.script-row').filter(has=p.get_by_text('安装 Nginx',exact=True)).get_by_role('button',name='在服务器上运行…',exact=True).click()
        d=p.get_by_role('dialog',name='选择服务器运行脚本',exact=True);d.locator('.installation-options').scroll_into_view_if_needed()
        expect(d.get_by_role('button',name='仅安装缺失的软件包',exact=True)).to_be_visible();expect(d.get_by_role('button',name='允许审核过的升级',exact=True)).to_be_visible()
        assert d.evaluate('(e)=>e.scrollWidth<=e.clientWidth');app.screenshot('installation-policy-chinese-narrow.png');d.get_by_role('button',name='取消',exact=True).click()
        passed('Chinese scope and narrow layout')
        assert not any(s in app.config.with_suffix('.tasks.json').read_text() for s in ['fixture-admin-secret','arcnix-test'])
        passed('no sensitive data persisted after all error paths')
(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),ensure_ascii=False,indent=2),encoding='utf-8')
print(f'{len(checks)} installation desktop groups passed')
