"""Private export ZIPs through real desktop SSH streams, without SFTP."""
import base64,json,time,zipfile
from pathlib import Path
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT
out=ROOT/'artifacts/export-download-ui'/str(time.time_ns());checks=[]
payload=bytes(range(256))*4096
def passed(message):checks.append(message);print(message,flush=True)
def seed(peer,name):
    exports=[dict(id='copy-'+name,source='source-'+name,data=base64.b64encode(payload).decode())]
    store=dict(cleanup=[],maintenance=[],recovery=[dict(id='copy-'+name,source='source-'+name,action='export',phase='completed',service='inactive',modified=1700000002,reasons=['export_data']),dict(id='active-'+name,source='source-'+name,action='export',phase='exporting',service='active',modified=1700000001,reasons=['export_data']),dict(id='release-'+name,source='source-'+name,action='release',phase='completed',service='inactive',modified=1700000000,reasons=['export_data'])])
    (peer.root/'export-download-fixture.json').write_text(json.dumps(exports));(peer.root/'audit-storage-fixture.json').write_text(json.dumps(store))
    (peer.root/'cleanup-reports-fixture.json').write_text(json.dumps([dict(id='source-'+name,modified=1700000000,phase='interrupted',scope='task',selected_count=1,processed_count=1,removed_files=2,pending_files=0,service='failed',issue=None,partial=False,released=True)]))
    return exports,store
def connect(app,fresh=False,zh=False):
    p=app.page;p.locator('.rail-tasks').click();p.get_by_role('button',name='清理报告' if zh else 'Cleanup reports',exact=True).click()
    if not fresh:p.get_by_role('button',name='连接清理报告…' if zh else 'Connect to cleanup reports…',exact=True).click()
    d=p.get_by_role('dialog');expect(d).to_be_visible()
    if fresh:
        for target in d.locator('.cleanup-report-target').all():target.click()
    expect(d.locator('.cleanup-report-target.active')).to_have_count(2)
    d.get_by_role('textbox',name='SSH 密码或私钥口令' if zh else 'SSH password or key passphrase',exact=True).first.fill('arcnix-test')
    for box in d.get_by_role('textbox',name='管理员密码（可选）' if zh else 'Administrator password (optional)',exact=True).all():box.fill('fixture-admin-secret')
    d.get_by_role('button',name='读取清理报告' if zh else 'Read cleanup reports',exact=True).click();expect(d).not_to_be_visible()
    expect(p.locator('.cleanup-query-state')).to_have_text(['清理报告查询完成' if zh else 'Cleanup report query completed']*2)
    p.locator('.audit-storage-open').click();expect(p.locator('.audit-storage-state')).to_have_text(['审计存储查询完成' if zh else 'Audit storage query completed']*2)
    p.get_by_role('button',name='残留导出存储' if zh else 'Residual export storage',exact=True).click();expect(p.locator('.audit-storage-record')).to_have_count(6)
    return p
def open_export(p,name='alpha',zh=False):
    p.locator('.audit-storage-record').filter(has=p.get_by_role('button',name='copy-'+name,exact=True)).get_by_role('button',name='下载导出…' if zh else 'Download export…',exact=True).click()
    d=p.get_by_role('dialog',name='下载残留导出' if zh else 'Download residual export',exact=True);expect(d).to_be_visible();return d
def ready(d,zh=False):expect(d.locator('.export-download-state')).to_have_text('导出可下载' if zh else 'Export ready to download')
def download(d,path):d.get_by_role('textbox',name='Local ZIP path',exact=True).fill(str(path));d.get_by_role('button',name='Download ZIP',exact=True).click()
def valid(path):
    with zipfile.ZipFile(path) as archive:
        assert archive.testzip() is None and len(archive.namelist())==7
        assert archive.read('files/record-1/result')==payload
        assert json.loads(archive.read('files/record-1/metadata-result.json'))['uid']==1001
def no_partial(local):return not list(local.glob('.arcnix-export-*.partial'))
def receiving(local):return any(path.stat().st_size>8192 for path in local.glob('.arcnix-export-*.partial'))

with Peer(out/'peer-a') as a,Peer(out/'peer-b') as b:
    original={a:seed(a,'alpha'),b:seed(b,'beta')};local=out/'local 中文';local.mkdir(parents=True)
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta')],local);host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    with Desktop(out/'desktop',config) as app:
        p=connect(app,True)
        for name in ['active-alpha','release-alpha']:
            expect(p.locator('.audit-storage-record').filter(has=p.get_by_role('button',name=name,exact=True)).get_by_role('button',name='Download export…',exact=True)).to_be_disabled()
        d=open_export(p);ready(d);expect(d.locator('.export-download-files')).to_contain_text('metadata-result.json');d.get_by_role('button',name='Cancel',exact=True).click();assert not (a.root/'export-download-streams').exists() and no_partial(local)
        passed('Independent export inventory needs no local task history or live source quarantine; active exports and release records cannot download, and closing review sends no stream')
        for flag in ['denied','wrong-id','wrong-source','wrong-path','missing-metadata']:
            marker=a.root/f'export-download-{flag}';marker.touch();d=open_export(p);expect(d.locator('.export-download-state')).to_have_text('Export download failed');expect(d.get_by_role('button',name='Download ZIP',exact=True)).to_be_disabled();assert 'fixture-admin-secret' not in d.inner_text();d.get_by_role('button',name='Cancel',exact=True).click();marker.unlink()
        passed('Denied and malformed inventory frames cannot start downloads; mismatched identity, source, escaped paths and omitted metadata are refused with secrets redacted')
        for name in ['alpha','beta']:
            d=open_export(p,name);ready(d);path=local/f'{name}.zip';download(d,path);expect(d.locator('.export-download-state')).to_have_text('Export download verified');valid(path);d.get_by_role('button',name='Close',exact=True).click()
        assert no_partial(local) and not app.config.with_suffix('.tasks.json').exists()
        passed('Two independently authenticated servers download byte-verified portable ZIPs through SSH; binary data, metadata, audit entries and Unicode local paths survive without SFTP')
        existing=local/'existing.zip';existing.write_bytes(b'keep existing file');d=open_export(p);ready(d);download(d,existing);expect(d.locator('.export-download-state')).to_have_text('Export download failed');assert existing.read_bytes()==b'keep existing file' and no_partial(local);d.get_by_role('button',name='Cancel',exact=True).click()
        (a.root/'export-download-slow').touch();racing=local/'racing.zip';d=open_export(p);ready(d);download(d,racing);eventually(lambda:receiving(local));racing.write_bytes(b'created by another writer');expect(d.locator('.export-download-state')).to_have_text('Export download failed');assert racing.read_bytes()==b'created by another writer' and no_partial(local);d.get_by_role('button',name='Cancel',exact=True).click();(a.root/'export-download-slow').unlink()
        for flag in ['stale','bad-checksum','bad-request','truncated','no-receipt','duplicate-receipt','final-error','bad-magic','oversized']:
            d=open_export(p);ready(d);marker=a.root/f'export-download-{flag}';marker.touch();path=local/f'failed-{flag}.zip';download(d,path);expect(d.locator('.export-download-state')).to_have_text('Export download failed');assert not path.exists() and no_partial(local)
            if flag=='oversized':expect(d).to_contain_text('Export stream exceeds its reviewed limit')
            if flag=='bad-magic':expect(d).to_contain_text('Export stream is not a ZIP archive')
            d.get_by_role('button',name='Cancel',exact=True).click();marker.unlink()
        passed('Existing targets are preserved; stale exports, wrong hashes or request receipts, truncation, absent/duplicate receipts and a nonzero final exit never publish a ZIP')
        (a.root/'export-download-slow').touch();d=open_export(p);ready(d);path=local/'cancelled.zip';download(d,path);expect(d.locator('.export-download-state')).to_have_text('Downloading export…');eventually(lambda:receiving(local));d.get_by_role('button',name='Cancel',exact=True).click();eventually(lambda:no_partial(local));time.sleep(.3);assert not path.exists();(a.root/'export-download-slow').unlink()
        (a.root/'export-download-inspect-slow').touch();d=open_export(p);d.get_by_role('button',name='Cancel',exact=True).click();time.sleep(2.3);expect(p.get_by_role('dialog')).to_have_count(0);(a.root/'export-download-inspect-slow').unlink()
        passed('Cancellation stops active transfer and removes its private partial file; late inventory replies cannot reopen a closed dialog or publish an archive')
        (a.root/'export-download-slow').touch();d=open_export(p);ready(d);path=local/'restart.zip';download(d,path);eventually(lambda:receiving(local));app.stop();assert not path.exists();(a.root/'export-download-slow').unlink();app.launch();p=connect(app,True)
        d=open_export(p);ready(d);download(d,path);expect(d.locator('.export-download-state')).to_have_text('Export download verified');valid(path);d.get_by_role('button',name='Close',exact=True).click()
        for partial in local.glob('.arcnix-export-*.partial'):assert partial.parent==local;partial.unlink()
        passed('Forced client closure never creates a final ZIP or changes the server copy; a fresh authenticated session can download it again without replaying an export task')
        p.get_by_role('button',name='Back to cleanup reports',exact=True).click();ga=p.locator('.cleanup-report-group').first;ga.get_by_role('button',name='Inspect cleanup report',exact=True).click();expect(ga.locator('.residual-recovery-row')).to_have_count(1);ga.locator('.residual-recovery-row').get_by_role('button',name='Download export…',exact=True).click();d=p.get_by_role('dialog');ready(d);d.get_by_role('button',name='Cancel',exact=True).click()
        app.settings('General');p.get_by_role('button',name='简体中文',exact=True).click();expect(p.locator('.page-header-title')).to_have_text('常规');p.set_viewport_size(dict(width=900,height=700));p=connect(app,False,True);d=open_export(p,zh=True);ready(d,True)
        tip=d.get_by_role('button',name='帮助：下载导出',exact=True);tip.hover();expect(p.get_by_role('tooltip')).to_contain_text('服务器副本保留');p.keyboard.press('Escape');tip.click();expect(p.get_by_role('tooltip')).to_be_visible();p.keyboard.press('Escape');expect(p.get_by_role('tooltip')).not_to_be_visible();assert p.evaluate('document.documentElement.scrollWidth<=innerWidth');assert d.evaluate('e=>e.scrollWidth<=e.clientWidth');assert not d.evaluate("e=>[...e.querySelectorAll('*')].filter(x=>!x.children.length&&x.textContent.trim()&&x.getClientRects().length&&parseFloat(getComputedStyle(x).fontSize)>0&&parseFloat(getComputedStyle(x).fontSize)<12).map(x=>x.textContent)");app.screenshot('download-chinese-narrow.png');d.get_by_role('button',name='取消',exact=True).click()
        assert not app.config.with_suffix('.tasks.json').exists()
        for peer in [a,b]:assert json.loads((peer.root/'export-download-fixture.json').read_text())==original[peer][0] and json.loads((peer.root/'audit-storage-fixture.json').read_text())==original[peer][1]
        for file in app.out.glob('config*.json'):assert 'fixture-admin-secret' not in file.read_text() and 'arcnix-test' not in file.read_text()
        passed('The cleanup report also opens downloads after source release; Chinese 900 × 700 uses hover/click help without overflow, persistent secrets, task history or server mutation')
(out/'summary.json').write_text(json.dumps(dict(passed=len(checks),checks=checks),ensure_ascii=False,indent=2),encoding='utf-8');print(out,flush=True)
