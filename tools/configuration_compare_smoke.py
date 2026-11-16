"""Current file snapshots, truthful comparisons and draft-only handoff in WebView."""
import json,time
from contextlib import ExitStack
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/configuration-compare-ui'/str(time.time_ns());checks=[];chinese=False
catalog={en:zh for line in (ROOT/'crates/arcnix/assets/translations.tsv').read_text(encoding='utf-8').splitlines() if '\t' in line for zh,en in [line.split('\t',1)]}
def n(en):return catalog.get(en,en) if chinese else en
def passed(label):checks.append(label);print(label,flush=True)
def fixture(peer,**values):(peer.root/'configuration-compare-fixture.json').write_text(json.dumps(values),encoding='utf-8')
def queries(peer):
    path=peer.root/'configuration-compare-queries.jsonl'
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
def start(p,admin=False,path='/etc/shared.conf'):
    p.get_by_role('button',name=n('Read configuration snapshots…'),exact=True).click();d=p.get_by_role('dialog',name=n('Read configuration snapshots'),exact=True);expect(d).to_be_visible()
    d.get_by_role('textbox',name=n('Configuration file path'),exact=True).fill(path)
    d.get_by_role('button',name=n('Select visible trusted servers'),exact=True).click()
    d.get_by_role('textbox',name=n('SSH password or key passphrase'),exact=True).first.fill('arcnix-test')
    button=d.get_by_role('button',name=n('Read as administrator'),exact=True)
    if ('active' in button.get_attribute('class').split())!=admin:button.click()
    if admin:
        expect(d.get_by_role('textbox',name=n('Administrator password (optional)'),exact=True)).to_have_count(2)
        for box in d.get_by_role('textbox',name=n('Administrator password (optional)'),exact=True).all():box.fill('fixture-admin-secret')
    d.get_by_role('button',name=n('Read snapshots'),exact=True).click();expect(d).not_to_be_visible();return d
def finish(p):expect(p.get_by_role('button',name=n('Read configuration snapshots…'),exact=True)).to_be_enabled(timeout=20000)
def read(p,**values):start(p,**values);finish(p)
def open_page(p):p.locator('.rail-configurations').click();p.get_by_role('button',name=n('Compare current files'),exact=True).click()

with Peer(out/'alpha') as a,Peer(out/'beta') as b:
    alpha=a.profile('Alpha');alpha.update(username='arcnix-nosftp',auth='Password',private_key=None)
    config=a.config([alpha,b.profile('Beta')],out);host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    for peer in [a,b]:(peer.root/'trace-task-queries').touch()
    fixture(a,content='workers=2;\nsecret=arcnix-test\n');fixture(b,content='workers=4;\nsecret=arcnix-test\n',mode=384)
    with Desktop(out/'desktop',config) as app:
        p=app.page;app.connect(1);open_page(p);assert not queries(a) and not queries(b)
        read(p);expect(p.locator('.configuration-comparison-status')).to_have_text(['Snapshot read']*2)
        expect(p.locator('.configuration-comparison-verdict')).to_have_text('File contents differ');expect(p.locator('.configuration-comparison-diff')).to_contain_text('-workers=2;');expect(p.locator('.configuration-comparison-diff')).to_contain_text('+workers=4;');expect(p.locator('.configuration-comparison-diff')).not_to_contain_text('arcnix-test')
        expect(p.locator('.configuration-comparison-result')).to_contain_text('Ownership or permissions differ');expect(p.get_by_role('button',name='Use baseline as draft',exact=True)).to_be_disabled()
        assert all(q==dict(path='/etc/shared.conf',administrator=False) for peer in [a,b] for q in queries(peer))
        app.servers();expect(p.locator('.connection-state')).to_have_text('SSH connected');expect(p.locator('.host-row.selected')).to_contain_text('Beta');open_page(p);expect(p.locator('.configuration-comparison-server')).to_have_count(0)
        passed('Independent ordinary-account/password/no-SFTP and private-key reads preserve Beta; full checksums, metadata differences, decoded secret masking and navigation clearing work')

        fixture(a,content='token=fixture-admin-secret\n');fixture(b,content='token=fixture-admin-secret\n',warning='Journal-free fixture warning fixture-admin-secret arcnix-test')
        read(p,admin=True);expect(p.locator('.configuration-comparison-verdict')).to_have_text('File contents match');expect(p.locator('.configuration-comparison-servers')).not_to_contain_text('fixture-admin-secret');expect(p.locator('.configuration-comparison-servers')).to_contain_text('[redacted]');assert all(queries(peer)[-1]['administrator'] for peer in [a,b])
        for issue,label in [('missing','File not found'),('denied','Permission denied'),('changed','File changed during reading; read snapshots again'),('binary','Configuration file is not UTF-8 text'),('too_large','Configuration file exceeds 128 KiB')]:
            fixture(b,issue=issue);read(p,admin=True);expect(p.locator('.configuration-comparison-status').last).to_have_text(label);expect(p.locator('.configuration-comparison-verdict')).to_have_text('');expect(p.locator('.configuration-comparison-server').last.get_by_role('button',name='Use as baseline',exact=True)).to_be_disabled()
        passed('Administrator reads require an explicit choice; matching content stays separate from metadata and missing/denied/changed/binary/oversized files never become matching snapshots')

        fixture(a,content='alpha=1;\n');fixture(b,content='beta=1;\n');read(p,admin=True)
        old=p.locator('.configuration-comparison-metadata').last.inner_text()
        for values in [dict(invalid_json=True),dict(sha256='0'*64),dict(wrong_path='/different'),dict(oversized=True)]:
            fixture(b,**values);read(p,admin=True);row=p.locator('.configuration-comparison-server').last;expect(row).to_contain_text('Previous snapshot; excluded from comparison');expect(row.locator('.configuration-comparison-metadata')).to_have_text(old);expect(row.get_by_role('button',name='Use as baseline',exact=True)).to_be_disabled();expect(p.locator('.configuration-comparison-verdict')).to_have_text('')
        passed('Malformed JSON, checksum mismatch, wrong scope and oversized responses retain a labelled previous snapshot and cannot reuse it in a new comparison')

        for peer in [a,b]:(peer.root/'configuration-compare-slow').touch()
        start(p,admin=True);expect(p.get_by_role('button',name='Read configuration snapshots…',exact=True)).to_be_disabled();p.get_by_role('button',name='Stop reading',exact=True).click();finish(p)
        expect(p.locator('.configuration-comparison-status')).to_have_text(['Snapshot reading stopped']*2);expect(p.locator('.configuration-comparison-verdict')).to_have_text('')
        for peer in [a,b]:(peer.root/'configuration-compare-slow').unlink()
        fixture(a,content='baseline_config=true;\n');fixture(b,content='baseline_config=false;\n');read(p)
        expect(p.get_by_role('button',name='Use baseline as draft',exact=True)).to_be_enabled();p.get_by_role('button',name='Use baseline as draft',exact=True).click()
        expect(p.get_by_role('textbox',name='Candidate configuration (UTF-8)',exact=True)).to_have_value('baseline_config=true;\n');expect(p.get_by_role('textbox',name='Configuration destination',exact=True)).to_have_value('/etc/shared.conf')
        expect(p.get_by_role('button',name='Custom application configuration',exact=True)).to_have_class(__import__('re').compile(r'\bactive\b'))
        p.get_by_role('button',name='Review configuration on servers…',exact=True).click();expect(p.locator('.form-error')).to_contain_text('Provide bounded validation, reload and health commands');expect(p.get_by_role('dialog',name='Run script on servers',exact=True)).to_have_count(0)
        open_page(p);read(p);expect(p.get_by_role('button',name='Use baseline as draft',exact=True)).to_be_disabled()
        for peer in [a,b]:assert not (peer.root/'task-launches.jsonl').exists()
        assert not app.config.with_suffix('.tasks.json').exists() and 'baseline_config' not in app.config.read_text()
        passed('Stopping discards late results; a clean baseline fills a custom draft without remote writes or commands, requires the normal review, and cannot overwrite an existing draft')

        app.settings('General');p.get_by_role('button',name='简体中文',exact=True).click();expect(p.locator('.page-header-title')).to_have_text('常规');chinese=True;open_page(p);read(p);p.set_viewport_size(dict(width=900,height=780))
        expect(p.locator('.configuration-comparison-verdict')).to_have_text('文件内容不同');tip=p.get_by_role('button',name='帮助：对比当前文件',exact=True);tip.hover();expect(p.get_by_role('tooltip')).to_contain_text('独立 SSH');tip.click();p.mouse.move(890,770);expect(p.get_by_role('tooltip')).to_be_visible();p.keyboard.press('Escape')
        assert p.evaluate('document.documentElement.scrollWidth<=innerWidth')
        assert p.locator('.configuration-comparison-layout').evaluate("el=>getComputedStyle(el).display==='grid'")
        assert p.locator('.configuration-comparison-diff').evaluate('el=>el.clientWidth>=320')
        assert not p.locator('.configuration-comparison-page *:visible').evaluate_all("els=>els.filter(el=>!el.children.length&&el.textContent.trim()&&parseFloat(getComputedStyle(el).fontSize)<12).map(el=>el.textContent)")
        app.screenshot('chinese.png');p.get_by_role('button',name='读取配置快照…',exact=True).click();d=p.get_by_role('dialog',name='读取配置快照',exact=True)
        for box in d.get_by_role('textbox',name=n('SSH password or key passphrase'),exact=True).all():expect(box).to_have_value('')
        button=d.get_by_role('button',name='读取快照',exact=True);assert button.bounding_box()['y']+button.bounding_box()['height']<=780;d.get_by_role('button',name=n('Cancel'),exact=True).click()
        passed('Chinese 900 px comparisons preserve hover/click help, readable typography, a visible dialog footer and fresh masked credentials')

chinese=False
with ExitStack() as stack:
    peers=[stack.enter_context(Peer(out/f'pool-{number}')) for number in range(6)]
    config=peers[0].config([peer.profile(f'Node {number}') for number,peer in enumerate(peers)],out)
    for peer in peers[1:]:
        host,port=peer.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=peer.data['host_key_rsa_fingerprint']))
    for peer in peers:(peer.root/'configuration-compare-slow').touch()
    with Desktop(out/'pool',config) as app:
        p=app.page;open_page(p);start(p);eventually(lambda:sum(len(queries(peer)) for peer in peers)==4)
        p.get_by_role('button',name='Stop reading',exact=True).click();finish(p);assert sum(len(queries(peer)) for peer in peers)==4 and not queries(peers[-1]) and not queries(peers[-2])
        expect(p.locator('.configuration-comparison-verdict')).to_have_text('')
        passed('Six requested targets use at most four simultaneous reads; stopping prevents queued servers from receiving requests and never confirms cancelled data')

(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8');print(f'{len(checks)} groups passed; artifacts: {out}',flush=True)
