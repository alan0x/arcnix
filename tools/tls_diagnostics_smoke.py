"""Served-TLS targets, immutable scope, owned jobs and independent reports."""
import copy,json,time
from pathlib import Path
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/tls-diagnostics-ui'/str(time.time_ns());checks=[];reports=out/'reports';reports.mkdir(parents=True)
def passed(label):checks.append(label);print(label,flush=True)
def auth(peer):return Path(peer.data['auth_audit']).read_bytes()
def launches(peer):
    p=peer.root/'task-launches.jsonl';return p.read_bytes() if p.exists() else b''
def history(app):return json.loads(app.config.with_suffix('.tasks.json').read_text())
def endpoints(p):return p.locator('.diagnostic-tls-row')
def fill(p,index,host,port='443',name=''):
    r=endpoints(p).nth(index);r.get_by_role('textbox',name='TLS connect host',exact=True).fill(host);r.get_by_role('textbox',name='TLS port',exact=True).fill(port);r.get_by_role('textbox',name='TLS verification name (optional)',exact=True).fill(name)
def add(p,host,port='443',name=''):
    n=endpoints(p).count();p.get_by_role('button',name='Add TLS endpoint',exact=True).click();expect(endpoints(p)).to_have_count(n+1);fill(p,n,host,port,name)
def help(p,button,needle):
    button.scroll_into_view_if_needed();p.evaluate('() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');p.mouse.move(p.viewport_size['width']-10,p.viewport_size['height']-10)
    button.hover();expect(p.get_by_role('tooltip')).to_contain_text(needle);button.click();p.mouse.move(p.viewport_size['width']-10,p.viewport_size['height']-10);expect(p.get_by_role('tooltip')).to_contain_text(needle)
    box=p.get_by_role('tooltip').bounding_box();assert box['x']>=10 and box['x']+box['width']<=p.viewport_size['width']-10;assert box['y']>=10 and box['y']+box['height']<=p.viewport_size['height']-10;p.keyboard.press('Escape')
def export(p,name,record=False):
    if not record:p.locator('.diagnostic-export-action').first.click()
    d=p.get_by_role('dialog',name='Export diagnostic report',exact=True)
    d.get_by_role('textbox',name='Report folder',exact=True).fill(str(reports));d.get_by_role('textbox',name='HTML file name',exact=True).fill(name);d.get_by_role('button',name='Review report',exact=True).click()
    expect(d.locator('.diagnostic-export-preview')).to_contain_text('Collection confirmed');expect(d.locator('.diagnostic-export-preview')).to_contain_text('127.0.0.1:8443 → live.example.test')
    d.get_by_role('button',name='Save HTML report',exact=True).click();expect(d).not_to_be_visible();return (reports/name).read_text(encoding='utf-8')

with Peer(out/'alpha') as a,Peer(out/'beta') as b:
    for peer in [a,b]:(peer.root/'trace-task-queries').touch()
    config=a.config([a.profile('Alpha'),b.profile('Beta')],out)
    host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    with Desktop(out/'desktop',config) as app:
        p=app.page;p.locator('.rail-diagnostics').click();before=(auth(a),auth(b))
        add(p,'https://example.test');p.get_by_role('button',name='Choose servers and review checks…',exact=True).click();expect(p.locator('.form-error')).to_contain_text('DNS name in ASCII')
        fill(p,0,'127.0.0.1','0','live.example.test');p.get_by_role('button',name='Choose servers and review checks…',exact=True).click();expect(p.locator('.form-error')).to_contain_text('TLS port')
        fill(p,0,'127.0.0.1','8443','live.example.test')
        add(p,'remove.test');add(p,'soon.test.');endpoints(p).nth(1).get_by_role('button',name='Remove TLS endpoint',exact=True).click();expect(endpoints(p)).to_have_count(2)
        expect(endpoints(p).nth(1).get_by_role('textbox',name='TLS connect host',exact=True)).to_have_value('soon.test.')
        add(p,'untrusted.test');add(p,'::1',name='timeout.test')
        add(p,'127.0.0.1','8443','LIVE.EXAMPLE.TEST.');p.get_by_role('button',name='Choose servers and review checks…',exact=True).click();expect(p.locator('.form-error')).to_contain_text('Duplicate TLS endpoint')
        endpoints(p).last.get_by_role('button',name='Remove TLS endpoint',exact=True).click();expect(endpoints(p)).to_have_count(4)
        help(p,p.get_by_role('button',name='Help: TLS endpoints',exact=True),'selected SSH server')
        assert (auth(a),auth(b))==before and not app.config.with_suffix('.tasks.json').exists()
        passed('URL and port errors plus canonical duplicate identities are rejected locally; adding/removing keyed rows preserves values and TLS usage stays in hover/click help')

        p.get_by_role('button',name='Choose servers and review checks…',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        d.get_by_role('button',name='Select visible trusted servers',exact=True).click();expect(d.locator('.target-choice.active')).to_have_count(2)
        assert (auth(a),auth(b))==before
        d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=20000)
        for value in ['ARCNIX_DIAG_START_V2','-verify_return_error','-verify_ip','-verify_hostname',"'[::1]:443' 'timeout.test'",'tls:3',"'soon.test.:443' 'soon.test' 'dns'"]:
            expect(p.locator('.task-target').first).to_contain_text(value)
        saved=history(app)[0];assert saved['diagnostic']['tls'][1]['host']=='soon.test.'
        assert len(saved['diagnostic']['tls'])==4 and saved['diagnostic']['tls'][0]==dict(host='127.0.0.1',port=8443,server_name='live.example.test')
        passed('Review performs no SSH; independent preflights freeze two exact targets and the direct-TLS address, verification identity, SNI/IP mode and complete v2 selection')

        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Running','Running'])
        app.stop();time.sleep(4.5);app.launch();p=app.page;p.locator('.rail-diagnostics').click();expect(p.locator('.diagnostic-summary')).to_have_text(['Result unknown','Result unknown'])
        p.get_by_role('button',name='View task and raw output',exact=True).click();p.locator('.task-batch-refresh:visible').click();d=p.get_by_role('dialog',name='Query batch results',exact=True);d.get_by_role('button',name='Query selected results',exact=True).click();expect(d).not_to_be_visible();expect(p.locator('.task-status')).to_have_text(['Succeeded','Succeeded'],timeout=20000)
        p.locator('.rail-diagnostics').click();expect(p.locator('.diagnostic-summary')).to_contain_text(['6 findings · 1 inconclusive','6 findings · 1 inconclusive'])
        for button in p.get_by_role('button',name='Findings and suggestions',exact=True).all():button.click()
        tls=p.locator('.diagnostic-finding').filter(has=p.get_by_text('TLS endpoint verification',exact=True));expect(tls).to_have_count(8)
        first=p.locator('.diagnostic-target').first
        for scope in ['127.0.0.1:8443 → live.example.test','soon.test.:443','untrusted.test:443','[::1]:443 → timeout.test']:
            expect(first.locator('.diagnostic-item-context').filter(has_text=scope)).to_have_count(1)
        expect(tls.filter(has_text='live.example.test').first.locator('.diagnostic-level')).to_have_text('Check passed');expect(tls.filter(has_text='soon.test').first.locator('.diagnostic-level')).to_have_text('Warning')
        expect(tls.filter(has_text='untrusted.test').first.locator('.diagnostic-level')).to_have_text('Critical');expect(tls.filter(has_text='timeout.test').first.locator('.diagnostic-level')).to_have_text('Unavailable or inconclusive')
        expect(tls.filter(has_text='live.example.test').first).to_contain_text('sha256 Fingerprint=AA:BB:CC')
        assert len(launches(a).splitlines())==1 and len(launches(b).splitlines())==1
        complete=history(app);identifier=complete[0]['id'];app.screenshot('tls-findings.png')
        p.locator('.rail-servers').click();expect(p.locator('.inventory-counts')).to_have_text('Matching servers 2 · Recorded findings 2 · Need verification 2');expect(p.locator('.inventory-diagnostic')).to_contain_text(['Findings 6 · Inconclusive 1','Findings 6 · Inconclusive 1']);p.locator('.rail-diagnostics').click()
        passed('Both server-owned jobs survive closing the desktop; explicit batch queries recover each saved endpoint and its separate pass, expiry warning, trust failure or inconclusive observation')

        before=(auth(a),auth(b));launch_before=(launches(a),launches(b));bytes_before=app.config.with_suffix('.tasks.json').read_bytes()
        text=export(p,'tls.html');assert 'TLS endpoint verification' in text and 'soon.test.:443' in text and 'live.example.test' in text
        assert 'sha256 Fingerprint=' not in text and 'ARCNIX_DIAG_' not in text and '<script' not in text
        assert (auth(a),auth(b))==before and (launches(a),launches(b))==launch_before and app.config.with_suffix('.tasks.json').read_bytes()==bytes_before
        passed('Offline report summaries preserve each saved TLS scope and mixed outcome without exporting raw scripts or unselected details, rewriting history or making network requests')

        app.stop();changed=copy.deepcopy(complete);changed[0]['diagnostic']['tls'][0]['server_name']='replacement.test';app.config.with_suffix('.tasks.json').write_text(json.dumps(changed));app.launch();p=app.page;p.locator('.rail-diagnostics').click()
        expect(p.locator('.diagnostic-run')).to_contain_text('Diagnostic results are not complete');expect(p.locator('.diagnostic-finding')).to_have_count(0)
        buttons=p.get_by_role('button',name='Findings and suggestions',exact=True);expect(buttons).to_have_count(2)
        for button in buttons.all():expect(button).to_be_disabled()
        assert (auth(a),auth(b))==before
        passed('Changing the saved TLS verification identity invalidates the complete report; an old pass cannot be rebound to another name')

        app.stop();app.config.with_suffix('.tasks.json').unlink();app.launch();p=app.page;p.locator('.rail-tasks').click();p.get_by_role('button',name='Server records',exact=True).click();d=p.get_by_role('dialog',name='Connect to server records',exact=True)
        d.get_by_role('button',name='Select trusted servers',exact=True).click();d.get_by_role('button',name='Read server records',exact=True).click()
        r=p.locator('.server-record-group').first.locator('.server-task-record').filter(has=p.get_by_text(identifier,exact=True));expect(r).to_be_visible(timeout=20000);r.get_by_role('button',name='View saved log',exact=True).click()
        d=p.get_by_role('dialog',name='Saved server task log',exact=True);d.get_by_role('button',name='Export diagnostic report…',exact=True).click();text=export(p,'server-tls.html',True)
        assert 'Server record' in text and '127.0.0.1:8443' in text and 'Collection confirmed' in text and not app.config.with_suffix('.tasks.json').exists()
        assert (launches(a),launches(b))==launch_before
        passed('Independent server metadata retains the exact TLS selection after local history is removed and can still verify and export it without a workspace, SFTP or repeated execution')

        app.settings('General');p.locator('.setting-theme:visible').get_by_role('radio', name='Dark', exact=True).check();p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check();p.locator('.rail-diagnostics').click();p.set_viewport_size(dict(width=900,height=780))
        p.get_by_role('button',name='添加 TLS 端点',exact=True).click();r=endpoints(p);r.get_by_role('textbox',name='TLS 连接主机',exact=True).fill('xn--fiqs8s.test');r.get_by_role('textbox',name='TLS 验证名称（可选）',exact=True).fill('name.test')
        help(p,p.get_by_role('button',name='帮助：TLS 端点',exact=True),'SNI')
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth');assert not p.locator('.diagnostic-settings *:visible').evaluate_all("els=>els.filter(el=>!el.children.length&&el.textContent.trim()&&parseFloat(getComputedStyle(el).fontSize)<12).map(el=>el.textContent)")
        app.screenshot('tls-chinese-dark-900.png');r.get_by_role('textbox',name='TLS 连接主机',exact=True).focus();p.keyboard.press('Tab');expect(r.get_by_role('textbox',name='TLS 端口',exact=True)).to_be_focused()
        p.locator('.settings-link').click();p.locator('.context-links').get_by_role('button',name='常规',exact=True).click();p.locator('.setting-language:visible').get_by_role('radio', name='English', exact=True).check();p.locator('.setting-theme:visible').get_by_role('radio', name='Light', exact=True).check();p.locator('.rail-diagnostics').click();expect(endpoints(p).get_by_role('textbox',name='TLS connect host',exact=True)).to_have_value('xn--fiqs8s.test')
        app.screenshot('tls-english-light-900.png');passed('Chinese/English and both themes retain edited TLS rows, hover/click help, keyboard order, at least 12 px text and no horizontal overflow at 900 px')

(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8');print(f'{len(checks)} groups passed; artifacts: {out}',flush=True)
