"""System resolver scopes in multi-server diagnostics and independent reports."""
import copy
import json
import time
from pathlib import Path
from desktop_fixture import Peer, Desktop, expect, ROOT

out=ROOT/'artifacts/name-resolution-ui'/str(time.time_ns())
reports=out/'reports'
reports.mkdir(parents=True)
checks=[]

def passed(label):
    checks.append(label)
    print(label,flush=True)

def auth(peer):return Path(peer.data['auth_audit']).read_bytes()
def launches(peer):
    p=peer.root/'task-launches.jsonl'
    return p.read_bytes() if p.exists() else b''
def history(app):return json.loads(app.config.with_suffix('.tasks.json').read_text())
def rows(p):return p.locator('.diagnostic-resolution-row')
def fill(p,index,host,expected=''):
    r=rows(p).nth(index)
    r.get_by_role('textbox',name='Hostname to resolve',exact=True).fill(host)
    r.get_by_role('textbox',name='Expected IP addresses (optional, comma-separated)',exact=True).fill(expected)
def add(p,host,expected=''):
    count=rows(p).count()
    p.get_by_role('button',name='Add name resolution check',exact=True).click()
    expect(rows(p)).to_have_count(count+1)
    fill(p,count,host,expected)
def export(p,name,record=False):
    if not record:p.locator('.diagnostic-export-action').first.click()
    d=p.get_by_role('dialog',name='Export diagnostic report',exact=True)
    d.get_by_role('textbox',name='Report folder',exact=True).fill(str(reports))
    d.get_by_role('textbox',name='HTML file name',exact=True).fill(name)
    d.get_by_role('button',name='Review report',exact=True).click()
    expect(d.locator('.diagnostic-export-preview')).to_contain_text('Collection confirmed')
    expect(d.locator('.diagnostic-export-preview')).to_contain_text('good.test → 192.0.2.1')
    d.get_by_role('button',name='Save HTML report',exact=True).click()
    expect(d).not_to_be_visible()
    return (reports/name).read_text(encoding='utf-8')

with Peer(out/'alpha') as a,Peer(out/'beta') as b:
    for peer in (a,b):(peer.root/'trace-task-queries').touch()
    config=a.config([a.profile('Alpha'),b.profile('Beta')],out,
                    updates=dict(automatic_checks=False,automatic_install=False))
    host,port=b.data['address'].rsplit(':',1)
    config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    with Desktop(out/'desktop',config) as app:
        p=app.page
        p.locator('.rail-diagnostics').click()
        before=(auth(a),auth(b))
        add(p,'https://example.test')
        for hostname,expected,error in [('https://example.test','','ASCII DNS hostname'),('0x7f000001','','ASCII DNS hostname'),('good.test','[::1]','comma-separated IPv4'),('good.test','::1,0:0:0:0:0:0:0:1','Duplicate expected IP')]:
            fill(p,0,hostname,expected)
            p.get_by_role('button',name='Choose servers and review checks…',exact=True).click()
            expect(p.locator('.form-error')).to_contain_text(error)
        fill(p,0,'good.test','192.0.2.1')
        add(p,'remove.test')
        add(p,'drift.test','192.0.2.2')
        rows(p).nth(1).get_by_role('button',name='Remove name resolution check',exact=True).click()
        expect(rows(p).nth(1).get_by_role('textbox',name='Hostname to resolve',exact=True)).to_have_value('drift.test')
        add(p,'GOOD.test')
        p.get_by_role('button',name='Choose servers and review checks…',exact=True).click()
        expect(p.locator('.form-error')).to_contain_text('Duplicate name resolution hostname')
        rows(p).nth(2).get_by_role('button',name='Remove name resolution check',exact=True).click()
        add(p,'timeout.test')
        add(p,'absolute.test.')
        assert before==(auth(a),auth(b)) and not app.config.with_suffix('.tasks.json').exists()
        passed('Invalid hosts, numeric aliases, expected literals and duplicate ranges fail locally; keyed removal preserves other edits without SSH or history')

        p.get_by_role('button',name='Add TLS endpoint',exact=True).click()
        tls=p.locator('.diagnostic-tls-row')
        tls.get_by_role('textbox',name='TLS connect host',exact=True).fill('tls-good.test')
        p.get_by_role('button',name='Choose servers and review checks…',exact=True).click()
        d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        assert before==(auth(a),auth(b))
        d.locator('.target-choice').nth(0).click()
        d.locator('.target-choice').nth(1).click()
        d.get_by_role('button',name='Check targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready','Ready'],timeout=20000)
        expect(p.locator('.task-target').first).to_contain_text("arcnix_name_check 'absolute.test.' '' 'resolve:3'")
        saved=history(app)[0]
        assert saved['diagnostic']['resolution'][1]==dict(host='drift.test',expected=['192.0.2.2'])
        assert len(saved['diagnostic']['resolution'])==4 and len(saved['diagnostic']['tls'])==1
        passed('Unconnected multi-server review freezes hostname, required-address subset and final dot; default checks, TLS and name resolution fit one request')

        p.get_by_role('button',name='Execute ready targets',exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Running','Running'])
        app.stop()
        time.sleep(4.5)
        app.launch()
        p=app.page
        p.locator('.rail-diagnostics').click()
        expect(p.locator('.diagnostic-summary')).to_have_text(['Result unknown','Result unknown'])
        p.get_by_role('button',name='View task and raw output',exact=True).click()
        p.locator('.task-batch-refresh:visible').click()
        d=p.get_by_role('dialog',name='Query batch results',exact=True)
        d.get_by_role('button',name='Query selected results',exact=True).click()
        expect(d).not_to_be_visible()
        expect(p.locator('.task-status')).to_have_text(['Succeeded','Succeeded'],timeout=20000)
        p.locator('.rail-diagnostics').click()
        expect(p.locator('.diagnostic-summary')).to_contain_text(['5 findings · 1 inconclusive','5 findings · 1 inconclusive'])
        for button in p.get_by_role('button',name='Findings and suggestions',exact=True).all():button.click()
        names=p.locator('.diagnostic-finding').filter(has=p.get_by_text('Name resolution',exact=True))
        expect(names).to_have_count(8)
        expect(names.filter(has_text='good.test').first.locator('.diagnostic-level')).to_have_text('Check passed · 2 resolved addresses')
        expect(names.filter(has_text='drift.test').first).to_contain_text('Missing expected addresses')
        expect(names.filter(has_text='timeout.test').first.locator('.diagnostic-level')).to_have_text('Unavailable or inconclusive')
        expect(names.filter(has_text='absolute.test.').first.locator('.diagnostic-item-context')).to_have_text('absolute.test.')
        assert all(len(launches(peer).splitlines())==1 for peer in (a,b))
        complete=history(app)
        identifier=complete[0]['id']
        app.screenshot('resolution-findings.png')
        p.locator('.rail-servers').click()
        expect(p.locator('.inventory-diagnostic')).to_contain_text(['Findings 5 · Inconclusive 1','Findings 5 · Inconclusive 1'])
        p.locator('.rail-diagnostics').click()
        passed('Accepted jobs survive desktop exit; explicit queries recover bound address counts, mismatches and inconclusive results once per server')

        before=(auth(a),auth(b))
        launch_before=(launches(a),launches(b))
        history_before=app.config.with_suffix('.tasks.json').read_bytes()
        text=export(p,'names.html')
        assert 'Name resolution' in text and 'absolute.test.' in text and 'drift.test → 192.0.2.2' in text
        assert 'Resolved addresses:' not in text and 'ARCNIX_DIAG_' not in text and '<script' not in text
        assert before==(auth(a),auth(b)) and history_before==app.config.with_suffix('.tasks.json').read_bytes()
        passed('Offline HTML retains required-address scope and conclusions while excluding raw lookup details and scripts by default, without network or history writes')

        app.stop()
        changed=copy.deepcopy(complete)
        changed[0]['diagnostic']['resolution'][0]['expected']=['192.0.2.99']
        app.config.with_suffix('.tasks.json').write_text(json.dumps(changed))
        app.launch()
        p=app.page
        p.locator('.rail-diagnostics').click()
        expect(p.locator('.diagnostic-run')).to_contain_text('Diagnostic results are not complete')
        expect(p.locator('.diagnostic-finding')).to_have_count(0)
        for button in p.get_by_role('button',name='Findings and suggestions',exact=True).all():expect(button).to_be_disabled()
        assert before==(auth(a),auth(b))
        passed('Changing a saved expected IP invalidates the report instead of rebinding an earlier pass to the new condition')

        app.stop()
        app.config.with_suffix('.tasks.json').unlink()
        app.launch()
        p=app.page
        p.locator('.rail-tasks').click()
        p.get_by_role('button',name='Server records',exact=True).click()
        d=p.get_by_role('dialog',name='Connect to server records',exact=True)
        d.get_by_role('button',name='Select trusted servers',exact=True).click()
        d.get_by_role('button',name='Read server records',exact=True).click()
        record=p.locator('.server-record-group').first.locator('.server-task-record').filter(has=p.get_by_text(identifier,exact=True))
        expect(record).to_be_visible(timeout=20000)
        record.get_by_role('button',name='View saved log',exact=True).click()
        p.get_by_role('dialog',name='Saved server task log',exact=True).get_by_role('button',name='Export diagnostic report…',exact=True).click()
        text=export(p,'server-names.html',True)
        assert 'Server record' in text and 'Collection confirmed' in text and not app.config.with_suffix('.tasks.json').exists()
        assert launch_before==(launches(a),launches(b))
        passed('Independent server records retain name queries and expected addresses after local history is removed, without SFTP or repeated execution')

        app.settings('General')
        p.locator('.setting-theme:visible').get_by_role('radio', name='Dark', exact=True).check()
        p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check()
        p.locator('.rail-diagnostics').click()
        expect(p.locator('.notice-text')).to_contain_text('诊断报告已保存至')
        p.set_viewport_size(dict(width=900,height=780))
        p.get_by_role('button',name='添加名称解析检查',exact=True).click()
        rows(p).get_by_role('textbox',name='待解析主机名',exact=True).fill('xn--fiqs8s.test.')
        rows(p).get_by_role('textbox',name='预期 IP 地址（可选，逗号分隔）',exact=True).fill('192.0.2.1,2001:db8::1')
        button=p.get_by_role('button',name='帮助：名称解析',exact=True)
        button.scroll_into_view_if_needed()
        p.evaluate('() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))')
        p.mouse.move(880,760)
        button.hover()
        expect(p.get_by_role('tooltip')).to_contain_text('末尾点号')
        button.click()
        p.mouse.move(880,760)
        expect(p.get_by_role('tooltip')).to_be_visible()
        box=p.get_by_role('tooltip').bounding_box()
        assert box['x']>=10 and box['y']>=10 and box['x']+box['width']<=890 and box['y']+box['height']<=770
        p.keyboard.press('Escape')
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert not p.locator('.diagnostic-settings *:visible').evaluate_all("els=>els.filter(e=>!e.children.length&&e.textContent.trim()&&parseFloat(getComputedStyle(e).fontSize)<12).map(e=>e.textContent)")
        rows(p).get_by_role('textbox',name='待解析主机名',exact=True).focus()
        p.keyboard.press('Tab')
        expect(rows(p).get_by_role('textbox',name='预期 IP 地址（可选，逗号分隔）',exact=True)).to_be_focused()
        app.screenshot('resolution-chinese-dark-900.png')
        app.settings('常规')
        p.locator('.setting-language:visible').get_by_role('radio', name='English', exact=True).check()
        p.locator('.setting-theme:visible').get_by_role('radio', name='Light', exact=True).check()
        p.locator('.rail-diagnostics').click()
        expect(rows(p).get_by_role('textbox',name='Hostname to resolve',exact=True)).to_have_value('xn--fiqs8s.test.')
        rows(p).scroll_into_view_if_needed()
        app.screenshot('resolution-english-light-900.png')
        passed('Chinese/English and both themes retain edited queries, hover/click help, keyboard order and readable labels without overflow at 900 px')

(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8')
print(f'{len(checks)} groups passed; artifacts: {out}',flush=True)
