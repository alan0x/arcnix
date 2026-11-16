"""Full local snapshots, reviewed replacement, busy guards and corrupt recovery."""
import copy,hashlib,json,time
from pathlib import Path
from desktop_fixture import Peer,Desktop,expect,eventually,ROOT

out=ROOT/'artifacts/configuration-backup-ui'/str(time.time_ns());checks=[];exports=out/'exports';exports.mkdir(parents=True)
def passed(label):checks.append(label);print(label,flush=True)
def saved(app):return json.loads(app.config.read_text(encoding='utf-8'))['config']
def auth(peer):return Path(peer.data['auth_audit']).read_bytes()
def launches(peer):
    p=peer.root/'task-launches.jsonl';return p.read_bytes() if p.exists() else b''
def export_dialog(app,name):
    app.settings('General');p=app.page;p.get_by_role('button',name='Back up configuration…',exact=True).click();d=p.get_by_role('dialog',name='Back up configuration',exact=True)
    d.get_by_role('textbox',name='Backup folder',exact=True).fill(str(exports));d.get_by_role('textbox',name='Backup file name',exact=True).fill(name);return d
def restore_dialog(app,path):
    app.settings('General');p=app.page;p.get_by_role('button',name='Restore configuration…',exact=True).click();d=p.get_by_role('dialog',name='Restore configuration',exact=True)
    d.get_by_role('textbox',name='Configuration backup file',exact=True).fill(str(path));d.get_by_role('button',name='Read configuration backup',exact=True).click();return d
def review(d):d.get_by_role('button',name='Review configuration replacement',exact=True).click();expect(d.locator('.configuration-restore-warning')).to_be_visible()
def cancel(d):d.get_by_role('button',name='Cancel',exact=True).click();expect(d).not_to_be_visible()
def connect_alpha(p):
    p.locator('.rail-servers').click();p.locator('.host-row').filter(has=p.get_by_text('Alpha',exact=True)).locator('.host-select').click();p.locator('.overview-connect').click();d=p.get_by_role('dialog',name='Connect to server',exact=True)
    d.get_by_role('textbox',name='Password or key passphrase',exact=True).fill('arcnix-test');d.get_by_role('button',name='Connect',exact=True).click();expect(p.locator('.connection-state')).to_have_text('SSH connected')
def help(p,button,needle):
    button.scroll_into_view_if_needed();p.evaluate('() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');p.mouse.move(p.viewport_size['width']-10,p.viewport_size['height']-10)
    button.hover();expect(p.get_by_role('tooltip')).to_contain_text(needle);button.click();p.mouse.move(p.viewport_size['width']-10,p.viewport_size['height']-10);expect(p.get_by_role('tooltip')).to_contain_text(needle);p.keyboard.press('Escape')

with Peer(out/'alpha',io_delay=.04) as a,Peer(out/'beta') as b:
    local=out/'local';local.mkdir();payload=bytes(range(256))*65536;(local/'active.bin').write_bytes(payload)
    alpha=a.profile('Alpha',group_id='g',environment='Production',tags=['Web','中文']);alpha.update(auth='Password',private_key=None)
    beta=b.profile('Beta',group_id='g',environment='Staging',tags=['Database'])
    config=a.config([alpha,beta],local,updates=dict(automatic_checks=False,automatic_install=False));config['config']['groups']=[dict(id='g',name='Servers 中文')]
    host,port=b.data['address'].rsplit(':',1);config['config']['trusted_hosts'].append(dict(host=host,port=int(port),fingerprint=b.data['host_key_rsa_fingerprint']))
    config['config']['snippets']=[dict(id='snapshot-script',name='Snapshot fixture',description='Saved script',command="printf '%s\\n' {{directory}}",administrator=False,parameters=[dict(name='directory',kind='path',default_value='/srv/default',allow_preset=True,required=True)])]
    config['config']['script_favorites']=[dict(source='user',script='snapshot-script'),dict(source='official',script='builtin:nginx')]
    policy=dict(id='website',name='Website',repository='/backup/site',password_file='/secrets/restic',source='/srv/site',database=None,excludes=[],servers=['Alpha','Beta'],administrator=False,keep_last=7);config['config']['backup_policies']=[policy]
    now=int(time.time());c=dict(task='saved-backup',started=now-500,confirmed=now-400,status='Succeeded',exit_code=0,endpoint=f"arcnix@{alpha['host']}:{alpha['port']}",fingerprint=a.data['host_key_rsa_fingerprint'],revision=hashlib.sha256(json.dumps(policy,separators=(',',':')).encode()).hexdigest()[:16],repository=policy['repository'])
    config['config']['backup_evidence']=[dict(policy='website',server='Alpha',kind='Backup',latest=c,last_success=c)]
    for peer in [a,b]:(peer.root/'trace-task-queries').touch()
    with Desktop(out/'desktop',config) as app:
        p=app.page;p.locator('.rail-scripts').click();p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click();p.locator('.script-row').filter(has_text='Snapshot fixture').get_by_role('button',name='Run on servers…',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True)
        d.get_by_role('textbox',name='directory',exact=True).fill('/srv/preset');d.get_by_role('textbox',name='Preset name',exact=True).fill('Saved site');d.get_by_role('button',name='Save new preset',exact=True).click();eventually(lambda:len(saved(app).get('script_presets',[]))==1)
        d.locator('.target-choice').first.click();d.get_by_role('textbox',name='SSH password or key passphrase',exact=True).fill('arcnix-test');d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready']);p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Succeeded'],timeout=20000)
        history=app.config.with_suffix('.tasks.json');history_before=history.read_bytes();launch_before=(launches(a),launches(b))
        connect_alpha(p);p.get_by_role('button',name='Terminal',exact=True).click();reader=p.locator('.terminal-surface .xterm-accessibility-tree');expect(reader).to_contain_text('Arcnix terminal fixture');p.locator('.xterm-helper-textarea').focus();p.keyboard.type('echo snapshot-session-kept');p.keyboard.press('Enter');expect(reader).to_contain_text('snapshot-session-kept')
        p.locator('.rail-configurations').click();p.get_by_role('button',name='Prepare configuration change…',exact=True).click();p.get_by_role('textbox',name='Candidate configuration (UTF-8)',exact=True).fill('DRAFT_NOT_BACKED_UP\n')
        before=(auth(a),auth(b));d=export_dialog(app,'snapshot.json');expect(d.get_by_role('button',name='Save configuration backup',exact=True)).to_be_disabled()
        d.get_by_role('button',name='Review configuration backup',exact=True).click();expect(d.locator('.configuration-backup-counts')).to_contain_text('Servers 2 · Groups 1 · Scripts 1');expect(d.locator('.configuration-backup-counts')).to_contain_text('Backup policies 1 · Saved presets 1 · Backup confirmations 1')
        digest=d.locator('.configuration-backup-digest').text_content();expected=saved(app);d.get_by_role('button',name='Show configuration preview',exact=True).click();expect(d.locator('.configuration-backup-json')).to_contain_text('/srv/preset');d.get_by_role('button',name='Save configuration backup',exact=True).click();expect(d).not_to_be_visible()
        snapshot=exports/'snapshot.json';assert digest=='Snapshot SHA-256: '+hashlib.sha256(snapshot.read_bytes()).hexdigest();text=snapshot.read_text(encoding='utf-8');assert json.loads(text)['config']==expected
        assert 'arcnix-test' not in text and 'PRIVATE KEY' not in text and 'DRAFT_NOT_BACKED_UP' not in text and '.tasks.json' not in text
        assert (auth(a),auth(b))==before and history.read_bytes()==history_before and (launches(a),launches(b))==launch_before
        p.locator('.rail-servers').click();expect(reader).to_contain_text('snapshot-session-kept');expect(p.locator('.connection-state')).to_have_text('SSH connected')
        passed('A private snapshot restores the complete saved model including labels, path references, typed presets, policies and confirmations; exporting preserves SSH/terminal and never collects key contents, session credentials, task logs or configuration drafts')

        d=export_dialog(app,'CON.json');d.get_by_role('button',name='Review configuration backup',exact=True).click();expect(d.locator('.form-error')).to_contain_text('Reserved device')
        d.get_by_role('textbox',name='Backup file name',exact=True).fill('race.json');d.get_by_role('button',name='Review configuration backup',exact=True).click();(exports/'race.json').write_bytes(b'Existing user data');d.get_by_role('button',name='Save configuration backup',exact=True).click();expect(d.locator('.form-error')).to_be_visible();assert (exports/'race.json').read_bytes()==b'Existing user data'
        d.get_by_role('textbox',name='Backup file name',exact=True).fill('cancelled.json');expect(d.get_by_role('button',name='Save configuration backup',exact=True)).to_be_disabled();cancel(d);assert not (exports/'cancelled.json').exists();d=export_dialog(app,'new.json');expect(d.locator('.configuration-backup-counts')).to_be_hidden();cancel(d)
        passed('Reserved names and destination races preserve existing files, edits invalidate review, and closing clears prepared exports without publishing partial snapshots')

        candidate=json.loads(text);candidate['config']['servers'][0]['name']='Restored Alpha';gamma=copy.deepcopy(candidate['config']['servers'][1]);gamma.update(id='Gamma',name='Gamma',host='127.0.0.30',port=22,auth='Password',private_key=None);candidate['config']['servers'].append(gamma)
        candidate['config']['trusted_hosts'][0]['fingerprint']=b.data['host_key_rsa_fingerprint'];candidate['config']['trusted_hosts'].append(dict(host=gamma['host'],port=22,fingerprint=a.data['host_key_rsa_fingerprint']))
        source=exports/'restore.json';source.write_text(json.dumps(candidate),encoding='utf-8');original=app.config.read_bytes();before=(auth(a),auth(b))
        d=restore_dialog(app,source);review(d);source.write_text(json.dumps({**candidate,'version':2}),encoding='utf-8');d.get_by_role('button',name='Restore reviewed configuration',exact=True).click();expect(d.locator('.form-error')).to_contain_text('backup changed');assert app.config.read_bytes()==original;cancel(d)
        source.write_text(json.dumps(candidate),encoding='utf-8');d=restore_dialog(app,source);review(d);app.config.write_bytes(b'external editor change');d.get_by_role('button',name='Restore reviewed configuration',exact=True).click();expect(d.locator('.form-error')).to_contain_text('configuration file changed');assert app.config.read_bytes()==b'external editor change';cancel(d);app.config.write_bytes(original)
        d=restore_dialog(app,source);cancel(d);assert app.config.read_bytes()==original and (auth(a),auth(b))==before
        passed('Changed source bytes and external edits to the active file reject a reviewed restore without overwriting files or disconnecting; cancellation leaves the original state intact')

        d=restore_dialog(app,source);review(d);expect(d.locator('.configuration-restore-trust')).to_have_text('Imported trust entries ignored: 3');d.get_by_role('button',name='Restore reviewed configuration',exact=True).click();expect(p.get_by_role('dialog')).to_have_count(0)
        current=saved(app);wanted=copy.deepcopy(candidate['config']);wanted['trusted_hosts']=expected['trusted_hosts'];assert current==wanted
        previous=list(app.config.parent.glob('arcnix-config-before-restore-*.json'));assert len(previous)==1 and previous[0].read_bytes()==original
        assert history.read_bytes()==history_before and (launches(a),launches(b))==launch_before and (auth(a),auth(b))==before
        p.locator('.rail-servers').click();expect(p.locator('.server-inventory')).to_be_visible();expect(p.locator('.inventory-row')).to_have_count(3);expect(p.locator('.module-nav')).to_have_count(0)
        r=p.locator('.inventory-row').filter(has=p.get_by_role('checkbox',name='Gamma',exact=True));expect(r.get_by_role('checkbox')).to_be_disabled()
        p.locator('.rail-configurations').click();expect(p.get_by_role('textbox',name='Candidate configuration (UTF-8)',exact=True)).to_have_value('DRAFT_NOT_BACKED_UP\n')
        p.locator('.rail-scripts').click();p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click();p.locator('.script-row').filter(has_text='Snapshot fixture').get_by_role('button',name='Run on servers…',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True);d.locator('.parameter-presets').get_by_role('button',name='Saved site',exact=True).click();d.get_by_role('button',name='Load selected preset',exact=True).click();expect(d.get_by_role('textbox',name='directory',exact=True)).to_have_value('/srv/preset');cancel(d)
        app.settings('General');p.get_by_role('button',name='Review previous file…',exact=True).click();previous_dialog=p.get_by_role('dialog',name='Restore configuration',exact=True);expect(previous_dialog.get_by_role('textbox',name='Configuration backup file',exact=True)).to_have_value(__import__('re').compile(__import__('re').escape(previous[0].name)+'$'));cancel(previous_dialog)
        passed('Successful restore preserves the exact previous file and local trust, keeps task history and unsaved configuration drafts, disconnects only at commit and leaves imported pins unable to authorize Gamma; restored presets remain usable')

        p.locator('.rail-servers').click();p.locator('.host-row').filter(has=p.get_by_text('Restored Alpha',exact=True)).locator('.host-select').click();p.locator('.overview-connect').click();d=p.get_by_role('dialog',name='Connect to server',exact=True);d.get_by_role('textbox',name='Password or key passphrase',exact=True).fill('arcnix-test');d.get_by_role('button',name='Connect',exact=True).click();expect(p.locator('.connection-state')).to_have_text('SSH connected')
        p.locator('.local-slot').get_by_role('button',name='active.bin',exact=True).click();p.locator('.local-slot').get_by_role('button',name='Upload',exact=True).click();eventually(lambda:any(0<float(v)<100 for v in p.get_by_role('progressbar',include_hidden=True).evaluate_all("els=>els.map(e=>e.getAttribute('aria-valuenow')).filter(Boolean)")))
        before_config=app.config.read_bytes();d=restore_dialog(app,source);expect(d).to_contain_text('Finish active operations');expect(d.get_by_role('button',name='Review configuration replacement',exact=True)).to_be_disabled()
        eventually(lambda:(a.root/'active.bin').is_file(),timeout=60);expect(d.get_by_role('button',name='Review configuration replacement',exact=True)).to_be_enabled();cancel(d)
        assert hashlib.sha256((a.root/'active.bin').read_bytes()).digest()==hashlib.sha256(payload).digest();assert app.config.read_bytes()==before_config
        passed('A real SFTP upload blocks configuration replacement, continues to completion with the expected SHA-256, and re-enables review without a client-side restore queue')

        large=copy.deepcopy(candidate)
        for profile in large['config']['servers']:profile['initial_remote_path']='/'+'中'*10000
        large_path=exports/'large.json';large_path.write_text(json.dumps(large,ensure_ascii=False),encoding='utf-8');d=restore_dialog(app,large_path);review(d);d.get_by_role('button',name='Show configuration preview',exact=True).click();expect(d.locator('.configuration-backup-json')).to_contain_text('Preview truncated at 64 KiB');assert len(d.locator('.configuration-backup-json').text_content().encode())<66000;cancel(d)
        app.settings('General');p.get_by_role('button',name='Dark',exact=True).click();p.get_by_role('button',name='简体中文',exact=True).click();p.set_viewport_size(dict(width=900,height=700))
        help(p,p.get_by_role('button',name='帮助：配置备份',exact=True),'可读 JSON');p.get_by_role('button',name='恢复配置…',exact=True).click();d=p.get_by_role('dialog',name='恢复配置',exact=True);d.get_by_role('textbox',name='配置备份文件',exact=True).fill(str(source));d.get_by_role('button',name='读取配置备份',exact=True).click();d.get_by_role('button',name='审核配置替换',exact=True).click()
        expect(d.locator('.configuration-backup-counts')).to_contain_text('服务器 3 · 分组 1 · 脚本 1');assert p.evaluate('document.documentElement.scrollWidth<=innerWidth');assert not d.locator('*:visible').evaluate_all("els=>els.filter(el=>!el.children.length&&el.textContent.trim()&&parseFloat(getComputedStyle(el).fontSize)<12).map(el=>el.textContent)")
        button=d.get_by_role('button',name='恢复已审核配置',exact=True);box=button.bounding_box();assert box['y']+box['height']<=700;app.screenshot('restore-chinese-dark-900.png');d.get_by_role('button',name='取消',exact=True).click()
        passed('Large UTF-8 configuration previews stop at a labelled 64 KiB boundary; Chinese dark 900 px review keeps readable counts, hover/click help, no overflow and a visible commit footer')

        app.stop();app.launch();p=app.page;p.locator('.rail-servers').click();expect(p.locator('.inventory-row')).to_have_count(3)
        assert saved(app)['servers'][0]['name']=='Restored Alpha' and history.read_bytes()==history_before and len(list(app.config.parent.glob('arcnix-config-before-restore-*.json')))==1
        passed('Restart loads the restored schema and profile labels while retaining the separate original task history and recoverable previous file')

    recovery=Desktop(out/'corrupt-recovery',config);bad=b'{ corrupted original configuration\x00';recovery.config.write_bytes(bad)
    with recovery as app:
        p=app.page;app.settings('General');expect(p.get_by_role('button',name='Back up configuration…',exact=True)).to_be_disabled();d=restore_dialog(app,source);expect(d).to_contain_text('Current configuration is unreadable');expect(d.locator('.configuration-backup-current-counts')).to_have_count(0);review(d);d.get_by_role('button',name='Restore reviewed configuration',exact=True).click();expect(p.get_by_role('dialog')).to_have_count(0)
        old=list(app.config.parent.glob('arcnix-config-before-restore-*.json'));assert len(old)==1 and old[0].read_bytes()==bad;assert saved(app)['trusted_hosts']==[]
        expect(p.get_by_role('button',name='Back up configuration…',exact=True)).to_be_enabled();p.get_by_role('button',name='Dark',exact=True).click();eventually(lambda:saved(app)['settings']['theme']=='Dark')
        p.locator('.rail-scripts').click();p.locator('.context-links').get_by_role('button',name='My scripts',exact=True).click();expect(p.get_by_role('button',name='Import scripts…',exact=True)).to_be_enabled();assert not app.config.with_suffix('.tasks.json').exists()
        app.stop();app.launch();p=app.page;app.settings('General');expect(p.locator('.app')).to_have_class('app theme-dark');expect(p.get_by_role('button',name='Back up configuration…',exact=True)).to_be_enabled()
        passed('A corrupt startup file can be repaired from backup without restarting first: exact bad bytes survive, imported trust stays ungranted, writable controls reactivate, edits save and the repaired file reloads normally')

(out/'summary.json').write_text(json.dumps(checks,ensure_ascii=False,indent=2),encoding='utf-8');print(f'{len(checks)} groups passed; artifacts: {out}',flush=True)
