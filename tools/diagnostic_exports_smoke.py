"""Frozen, local-only HTML reports with live task updates and offline rendering."""
import json,re,time
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT,sync_playwright

out=ROOT/'artifacts/diagnostic-export-ui'/str(time.time_ns());checks=[];reports=out/'reports';reports.mkdir(parents=True)
def passed(label):checks.append(label);print(label,flush=True)
def history(app):return json.loads(app.config.with_suffix('.tasks.json').read_text())
def launches(peer):
    file=peer.root/'task-launches.jsonl'
    return file.read_text().splitlines() if file.exists() else []
def open_export(p):
    p.locator('.rail-diagnostics').click();p.locator('.diagnostic-export-action').first.click()
    d=p.get_by_role('dialog',name='Export diagnostic report',exact=True);expect(d).to_be_visible();return d
def destination(d,name):
    d.get_by_role('textbox',name='Report folder',exact=True).fill(str(reports));d.get_by_role('textbox',name='HTML file name',exact=True).fill(name)
def preview(d):d.get_by_role('button',name='Review report',exact=True).click();expect(d.locator('.diagnostic-export-preview')).to_be_visible();return d.locator('.diagnostic-export-preview')
def save(d,name):
    d.get_by_role('button',name='Save HTML report',exact=True).click();expect(d).not_to_be_visible();return (reports/name).read_text(encoding='utf-8')

with Peer(out/'alpha') as a,Peer(out/'beta') as b:
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    for peer in [a,b]:
        (peer.root/'trace-task-queries').touch();(peer.root/'diagnostic-detail-secret').touch()
    with Desktop(out/'desktop',config) as app:
        p=app.page;p.locator('.rail-diagnostics').click()
        p.get_by_role('textbox',name='PEM certificate path (optional)',exact=True).fill('/etc/证书 <img src=x onerror=alert(1)>.pem')
        p.get_by_role('button',name='Nginx',exact=True).click();p.get_by_role('button',name='Add configuration check',exact=True).click()
        p.locator('.diagnostic-configuration-row').get_by_role('button',name='OpenSSH',exact=True).click()
        p.get_by_role('textbox',name='Additional configuration file (optional; default if empty)',exact=True).fill('/etc/ssh/sshd_config')
        p.get_by_role('button',name='Choose servers and review checks…',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        d.get_by_role('button',name='Select visible trusted servers',exact=True).click()
        d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test')
        for field in d.get_by_role('textbox',name='Administrator password (optional)',exact=True).all():field.fill('fixture-admin-secret')
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready']*2,timeout=20000)
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Running']*2,timeout=20000)
        d=open_export(p);destination(d,'running.html');body=preview(d);expect(body).to_contain_text('Running');expect(body).not_to_contain_text('Collection confirmed')
        eventually(lambda:all(t['status']=='Succeeded' for t in history(app)[0]['targets']),timeout=20)
        expect(body).to_contain_text('Running');expect(body).not_to_contain_text('Collection confirmed')
        text=save(d,'running.html');assert 'Running' in text and 'Collection confirmed' not in text
        passed('A report opened during collection freezes that saved snapshot while the live batch finishes; its unknown findings never become confirmed by later UI updates')

        d=open_export(p);expect(d.locator('.diagnostic-export-target.active')).to_have_count(2)
        destination(d,'summary.html');d.locator('.diagnostic-export-target').first.click();body=preview(d)
        expect(body).to_contain_text('Collection confirmed');expect(body).to_contain_text('df (GNU coreutils) 9.4');expect(body).to_contain_text('sshd version unavailable')
        expect(body).not_to_contain_text('detail-only marker');expect(body).to_contain_text('<img src=x');expect(body.locator('img')).to_have_count(0)
        saved_history=app.config.with_suffix('.tasks.json').read_bytes();before=[launches(a),launches(b)]
        text=save(d,'summary.html');assert 'Beta' in text and '>Alpha<' not in text and 'detail-only marker' not in text
        assert '<img' not in text and '&lt;img src=x' in text and '<script' not in text and 'ARCNIX_DIAG_' not in text
        assert 'default-src' in text and 'fixture-admin-secret' not in text and 'arcnix-test' not in text
        assert app.config.with_suffix('.tasks.json').read_bytes()==saved_history and [launches(a),launches(b)]==before
        passed('Selected-server summaries include captured environment, check scope, observation times and separate collection/findings states without scripts, details, secrets or network work')

        d=open_export(p);destination(d,'race.html');preview(d)
        d.get_by_role('button',name='Include finding details',exact=True).click();expect(d.get_by_role('button',name='Save HTML report',exact=True)).to_be_disabled();body=preview(d)
        expect(body).to_contain_text('detail-only marker');expect(body).to_contain_text('[redacted]');expect(body).not_to_contain_text('fixture-admin-secret')
        (reports/'race.html').write_bytes(b'Existing user file')
        d.get_by_role('button',name='Save HTML report',exact=True).click();expect(d.locator('.form-error')).to_be_visible();assert (reports/'race.html').read_bytes()==b'Existing user file'
        d.get_by_role('textbox',name='HTML file name',exact=True).fill('detailed.html');expect(d.get_by_role('button',name='Save HTML report',exact=True)).to_be_disabled();preview(d);text=save(d,'detailed.html');assert 'detail-only marker' in text and '[redacted]' in text
        assert not list(reports.glob('.arcnix-report-*.tmp'))
        passed('Detail opt-in and destination edits invalidate review; a destination created after preview is preserved and a fully reviewed report saves without temporary-file residue')

        d=open_export(p);expect(d.get_by_role('textbox',name='Report folder',exact=True)).to_have_value('');expect(d.locator('.diagnostic-export-details')).not_to_have_class(re.compile(r'\bactive\b'))
        destination(d,'cancelled.html');preview(d);d.get_by_role('button',name='Cancel',exact=True).click();assert not (reports/'cancelled.html').exists()
        d=open_export(p);destination(d,'../unsafe.html');d.get_by_role('button',name='Review report',exact=True).click();expect(d.locator('.form-error')).to_contain_text('plain HTML file name');expect(d.get_by_role('button',name='Save HTML report',exact=True)).to_be_disabled();d.get_by_role('button',name='Cancel',exact=True).click()
        complete_history=history(app)
        passed('Closing clears selected options, paths and previews; cancelled previews create no file and unsafe output names cannot reach save')

        # Independent account logs remain exportable after local history loss.
        app.stop();app.config.with_suffix('.tasks.json').unlink();app.launch();p=app.page;p.locator('.rail-tasks').click();p.get_by_role('button',name='Server records',exact=True).click()
        d=p.get_by_role('dialog',name='Connect to server records',exact=True);d.get_by_role('button',name='Select trusted servers',exact=True).click();d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).first.fill('arcnix-test');d.get_by_role('button',name='Read server records',exact=True).click()
        row=p.locator('.server-record-group').first.locator('.server-task-record').filter(has=p.get_by_text(complete_history[0]['id'],exact=True));expect(row).to_be_visible(timeout=20000);row.get_by_role('button',name='View saved log',exact=True).click()
        d=p.get_by_role('dialog',name='Saved server task log',exact=True);d.get_by_role('button',name='Export diagnostic report…',exact=True).click()
        d=p.get_by_role('dialog',name='Export diagnostic report',exact=True);destination(d,'server-record.html');body=preview(d)
        expect(body).to_contain_text('Collection confirmed');expect(body).to_contain_text('Server record');expect(body).to_contain_text('/etc/ssh/sshd_config')
        save(d,'server-record.html');assert not app.config.with_suffix('.tasks.json').exists();assert [launches(a),launches(b)]==before
        passed('Server metadata preserves the original diagnostic selection and verifies an export without SFTP, a workspace or local history; old missing selections remain explicitly unverified')

    # An older collector has no environment and no recorded selection.
    zh_config=json.loads(json.dumps(config));zh_config['config']['settings']['language']='SimplifiedChinese'
    legacy=json.loads(json.dumps(complete_history));legacy[0]['diagnostic']=None
    for target in legacy[0]['targets']:target['output']=re.sub(r'ARCNIX_DIAG_ENV_START_V1\t.*?ARCNIX_DIAG_ENV_END_V1\t[^\n]*\n','',target['output'],flags=re.S)
    with Desktop(out/'chinese',zh_config) as app:
        app.stop();app.config.with_suffix('.tasks.json').write_text(json.dumps(legacy),encoding='utf-8');app.launch();p=app.page;p.set_viewport_size(dict(width=900,height=780));p.locator('.rail-diagnostics').click();p.get_by_role('button',name='导出报告…',exact=True).click()
        d=p.get_by_role('dialog',name='导出诊断报告',exact=True);tip=d.get_by_role('button',name='帮助：诊断报告导出',exact=True);tip.hover();expect(p.get_by_role('tooltip')).to_contain_text('离线');tip.click();p.mouse.move(890,770);expect(p.get_by_role('tooltip')).to_be_visible();p.keyboard.press('Escape')
        d.get_by_role('textbox',name='报告保存目录',exact=True).fill(str(reports));d.get_by_role('textbox',name='HTML 文件名',exact=True).fill('中文.html');d.get_by_role('button',name='预览报告',exact=True).click()
        expect(d.locator('.diagnostic-export-preview')).to_contain_text('未记录环境版本');expect(d.locator('.diagnostic-export-preview')).to_contain_text('尚未核验')
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert not d.locator('*:visible').evaluate_all("els=>els.filter(el=>!el.children.length&&el.textContent.trim()&&parseFloat(getComputedStyle(el).fontSize)<12).map(el=>el.textContent)")
        save_button=d.get_by_role('button',name='保存 HTML 报告',exact=True);assert save_button.bounding_box()['y']+save_button.bounding_box()['height']<=780
        app.screenshot('chinese-preview.png');save_button.click();expect(d).not_to_be_visible();assert '服务器诊断报告' in (reports/'中文.html').read_text(encoding='utf-8')
        passed('Legacy reports keep missing environment and scope visible; Chinese 900 px review preserves hover/click help, readable type and an accessible save footer')

# Render exported artifacts in an owned headless browser, with networking disabled.
with sync_playwright() as runtime:
    browser=runtime.chromium.launch(channel='msedge',headless=True)
    page=browser.new_page(viewport=dict(width=1000,height=900));page.context.set_offline(True);requests=[];page.on('request',lambda request:requests.append(request.url))
    for name in ['summary.html','detailed.html','server-record.html','中文.html']:
        page.goto((reports/name).resolve().as_uri());expect(page.locator('h1')).to_be_visible()
        assert page.locator('script,img,iframe,link,form').count()==0 and page.locator('[onerror],[onclick]').count()==0
        assert page.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert not any(url.startswith(('http:','https:')) for url in requests)
        page.screenshot(path=str(out/(name+'.png')),full_page=True)
    browser.close()
passed('Exported English, Chinese, detailed and unverified HTML files render offline without active content, external requests or horizontal overflow')
(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8');print(f'{len(checks)} groups passed; artifacts: {out}',flush=True)
