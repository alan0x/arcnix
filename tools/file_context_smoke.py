"""Real pointer/keyboard file-menu workflows in an isolated WebView and SSH peer."""
from pathlib import Path
import ctypes,json,os,socket,subprocess,sys,time,urllib.request
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'artifacts/python'))
from playwright.sync_api import sync_playwright,expect

def clipboard():
    user=ctypes.windll.user32;kernel=ctypes.windll.kernel32
    user.GetClipboardData.restype=ctypes.c_void_p;kernel.GlobalLock.argtypes=[ctypes.c_void_p];kernel.GlobalLock.restype=ctypes.c_void_p;kernel.GlobalUnlock.argtypes=[ctypes.c_void_p]
    if not user.OpenClipboard(None):return None
    try:
        data=user.GetClipboardData(13)
        if not data:return None
        pointer=kernel.GlobalLock(data)
        if not pointer:return None
        try:return ctypes.wstring_at(pointer)
        finally:kernel.GlobalUnlock(data)
    finally:user.CloseClipboard()
def copied(expected):
    deadline=time.monotonic()+3
    while time.monotonic()<deadline:
        if clipboard()==expected:return
        time.sleep(.03)
    raise AssertionError('The copied path did not match the isolated test selection')

def check_delete_batch(page, pane, folder, out):
    count = 103
    folder.mkdir()
    for index in range(count):
        (folder / f'document-{index:03}.txt').touch()
    pane.get_by_role('button', name='alpha.txt', exact=True).click()
    page.keyboard.press('F5')
    pane.get_by_role('button', name=folder.name, exact=True).dblclick()
    pane.get_by_role('button', name='document-000.txt', exact=True).click()
    pane.get_by_role('button', name='document-102.txt', exact=True).click(modifiers=['Shift'])
    expect(pane.locator('.file-row.row-selected')).to_have_count(count)
    page.keyboard.press('Shift+Delete')
    dialog = page.locator('.file-delete-dialog')
    expect(dialog.locator('.file-delete-name')).to_have_text('103 local items')
    paths = dialog.locator('.file-delete-path')
    expect(paths).to_have_count(3)
    dialog.screenshot(path=out / 'delete-batch.png')
    dialog.get_by_role('button', name='View all 103 items').click()
    expect(paths).to_have_count(25)
    previous = dialog.get_by_role('button', name='Previous page')
    following = dialog.get_by_role('button', name='Next page')
    expect(previous).to_be_disabled()
    for _ in range(4):
        following.click()
    expect(paths).to_have_count(3)
    expect(paths.last).to_contain_text('document-102.txt')
    expect(following).to_be_disabled()
    previous.click()
    expect(paths.first).to_contain_text('document-075.txt')
    dialog.get_by_role('button', name='Show fewer items').click()
    expect(paths.first).to_contain_text('document-000.txt')
    dialog.get_by_role('button', name='Cancel', exact=True).click()
    expect(dialog).to_have_count(0)
    assert len(list(folder.iterdir())) == count

def main(binary=None):
    out=ROOT/'artifacts/file-context-ui';out.mkdir(parents=True,exist_ok=True)
    ready=out/('fixture-'+str(time.time_ns())+'.json');log=(out/'desktop.log').open('w',encoding='utf-8')
    fixture=subprocess.Popen([sys.executable,str(ROOT/'tools/sftp_fixture.py'),'--ready',str(ready)],cwd=ROOT,stdout=log,stderr=log)
    app=None;checks=[]
    try:
        deadline=time.monotonic()+20
        while not ready.exists():assert fixture.poll() is None and time.monotonic()<deadline;time.sleep(.1)
        peer=json.loads(ready.read_text());host,port=peer['address'].rsplit(':',1);assert host=='127.0.0.1'
        local=out/('files-'+str(time.time_ns()));local.mkdir();(local/'alpha.txt').write_bytes(b'alpha contents');(local/'beta-中文.txt').write_bytes(b'beta contents');(local/'child').mkdir();(local/'child'/'nested.txt').write_bytes(b'nested')
        remote_root=Path(peer['root']);(remote_root/'folder'/'nested.txt').write_bytes(b'remote nested')
        config=out/'config.json';profile=dict(id='file-menu',name='File menus',host=host,port=int(port),username='arcnix',auth='PrivateKey',private_key=peer['client_key_ed25519'],initial_remote_path='/')
        config.write_text(json.dumps(dict(version=1,config=dict(servers=[profile,dict(profile,id="no-sftp",name="SSH without SFTP",username="arcnix-nosftp")],groups=[],trusted_hosts=[dict(host=host,port=int(port),fingerprint=peer['host_key_rsa_fingerprint'])],local_directory=str(local),settings=dict(language='English',theme='Light',show_hidden_files=True,resource_interval_seconds=3)))))
        with socket.socket() as s:s.bind(('127.0.0.1',0));debug=s.getsockname()[1]
        endpoint=f'http://127.0.0.1:{debug}';env=os.environ.copy();env['WEBVIEW2_USER_DATA_FOLDER']=str(out/('webview-'+str(time.time_ns())));env['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS']=f'--remote-debugging-port={debug} --remote-debugging-address=127.0.0.1'
        if binary is None:
            metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=ROOT))
            binary=Path(metadata['target_directory'])/'debug/arcnix.exe'
        app=subprocess.Popen([str(binary),'--config',str(config)],cwd=ROOT,env=env,stdout=log,stderr=log)
        deadline=time.monotonic()+20
        while True:
            try:urllib.request.urlopen(endpoint+'/json/version',timeout=1).close();break
            except OSError:assert app.poll() is None and time.monotonic()<deadline;time.sleep(.1)
        with sync_playwright() as pw:
            browser=pw.chromium.connect_over_cdp(endpoint);page=browser.contexts[0].pages[0];page.set_default_timeout(10000);page.set_viewport_size(dict(width=1440,height=980))
            try:
                page.locator('.host-select').first.click();page.locator('.overview-transfer').click();expect(page.locator('.connection-state')).to_have_text('SSH connected')
                left=page.locator('.local-slot');right=page.locator('.remote-slot');menu=page.locator('.file-context-menu')
                def file(pane,name):return pane.get_by_role('button',name=name,exact=True)
                def blank(pane):
                    box=pane.locator('.file-list-container').bounding_box()
                    box=box if box['height']>16 else pane.locator('.file-empty').bounding_box()
                    frame=pane.bounding_box()
                    bottom=min(box['y']+box['height'],frame['y']+frame['height']-28,page.viewport_size['height']-8)
                    page.mouse.click(box['x']+box['width']-20,bottom-8,button='right')
                    expect(menu).to_be_visible()
                def action(name):menu.get_by_role('button',name=name,exact=True).click()
                for pane,root,existing in [(left,local,'alpha.txt'),(right,remote_root,'welcome.txt')]:
                    # An entry created outside the app is absent from the cached listing.
                    (root/'New file').mkdir()
                    blank(pane); action('New file')
                    name=pane.locator('.file-inline-name')
                    expect(name).to_be_focused()
                    expect(name).to_have_value('New file (2)')
                    assert name.evaluate('e => e.selectionStart === 0 && e.selectionEnd === e.value.length')
                    assert (root/'New file (2)').read_bytes()==b''
                    expect(page.get_by_role('dialog')).to_have_count(0)
                    name.fill('bad/name.txt'); name.press('Enter')
                    expect(pane.locator('.file-name-error')).to_be_visible()
                    duplicate=root/existing
                    previous=duplicate.read_bytes()
                    name.fill(existing); name.press('Enter')
                    expect(pane.locator('.file-name-error')).to_contain_text('already exists')
                    assert duplicate.read_bytes()==previous
                    name.fill('discarded'); name.press('Escape')
                    expect(name).to_have_count(0)
                    assert (root/'New file (2)').is_file() and not (root/'discarded').exists()
                    blank(pane); action('New file')
                    expect(name).to_have_value('New file (3)')
                    name.fill('新文件 empty.txt'); name.press('Enter')
                    expect(file(pane,'新文件 empty.txt')).to_be_visible()
                    assert (root/'新文件 empty.txt').read_bytes()==b''
                checks.append('Local and SFTP creation reserves unique names, selects them inline, and preserves existing entries on collisions and Escape')
                file(left,'alpha.txt').click();file(left,'beta-中文.txt').click(modifiers=['Control']);file(left,'alpha.txt').click(button='right');expect(menu).to_be_visible();expect(left.locator('.row-selected')).to_have_count(2)
                expect(menu.get_by_role('button',name='Rename',exact=True)).to_be_disabled();action('Copy selected paths');expect(menu).to_have_count(0);copied(str(local/'alpha.txt')+'\n'+str(local/'beta-中文.txt'))
                file(left,'alpha.txt').click(button='right');action('Properties');dialog=page.get_by_role('dialog',name='Properties',exact=True);expect(dialog).to_contain_text('2 selected');expect(dialog).to_contain_text(str(local/'beta-中文.txt'));dialog.get_by_role('button',name='Close dialog',exact=True).click()
                file(left,'child').click(button='right');expect(left.locator('.row-selected')).to_have_count(1);expect(menu.get_by_role('button',name='Open folder',exact=True)).to_have_count(0);page.screenshot(path=str(out/'folder-menu.png'));page.keyboard.press('Escape');file(left,'child').dblclick();expect(file(left,'nested.txt')).to_be_visible();left.get_by_role('button',name='Local parent directory',exact=True).click();expect(file(left,'alpha.txt')).to_be_visible()
                checks.append('Right-click keeps existing multiple selection, replaces it for a different item, copies exact Unicode-capable paths and opens folders')
                blank(left);expect(left.locator('.row-selected')).to_have_count(0);expect(menu.get_by_role('button',name='Rename',exact=True)).to_have_count(0);action('Copy directory path');expect(menu).to_have_count(0);copied(str(local))
                blank(left);action('New folder')
                name=left.locator('.file-inline-name')
                expect(name).to_be_focused()
                assert (local/'New folder').is_dir()
                name.fill('created'); name.press('Tab')
                expect(file(left,'created')).to_be_visible(); assert (local/'created').is_dir()
                blank(left);action('Select all');expect(left.locator('.row-selected')).to_have_count(len(list(local.iterdir())))
                checks.append('Blank-space menus clear selection and provide working directory-copy, create-folder and select-all actions')
                file(left,'beta-中文.txt').click();file(left,'beta-中文.txt').click(button='right');action('Rename')
                name=left.locator('.file-inline-name')
                expect(name).to_be_focused()
                expect(name).to_have_value('beta-中文.txt')
                assert name.evaluate('e => e.value.slice(e.selectionStart, e.selectionEnd)')=='beta-中文'
                page.keyboard.insert_text('renamed')
                expect(name).to_have_value('renamed.txt')
                name.press('Enter')
                expect(file(left,'renamed.txt')).to_be_visible()
                assert (local/'renamed.txt').read_bytes()==b'beta contents'
                file(left,'renamed.txt').click();page.keyboard.press('Shift+F10');expect(menu).to_be_visible();expect(menu.get_by_role('button',name='Upload',exact=True)).to_be_focused();page.keyboard.press('ArrowDown');expect(menu.get_by_role('button',name='Rename',exact=True)).to_be_focused();page.keyboard.press('End');expect(menu.get_by_role('button',name='Delete permanently',exact=True)).to_be_focused();page.keyboard.press('Home');expect(menu.get_by_role('button',name='Upload',exact=True)).to_be_focused();page.keyboard.press('Escape');expect(menu).to_have_count(0);expect(file(left,'renamed.txt')).to_be_focused()
                page.keyboard.press('F2');expect(name).to_be_focused();name.fill('cancelled.txt');name.press('Escape');expect(name).to_have_count(0);assert (local/'renamed.txt').exists() and not (local/'cancelled.txt').exists()
                file(left,'alpha.txt').click();page.keyboard.press('Alt+Enter');expect(dialog).to_be_visible();expect(dialog).to_contain_text('14 B');expect(dialog).not_to_contain_text('\\\\?\\');dialog.get_by_role('button',name='Close dialog',exact=True).click();file(left,'alpha.txt').click();page.keyboard.press('Control+Shift+c');copied(str(local/'alpha.txt'));(local/'after-refresh.txt').write_bytes(b'refresh');page.keyboard.press('F5');expect(file(left,'after-refresh.txt')).to_be_visible()
                checks.append('Rename changes the actual file; Shift+F10, arrows, Home/End, Escape/focus restoration, F2, Alt+Enter, Ctrl+Shift+C and F5 behave consistently')
                file(left,'alpha.txt').click(button='right');action('Upload');expect(page.locator('.queue-row').last).to_contain_text('Completed');assert (remote_root/'alpha.txt').read_bytes()==b'alpha contents';expect(file(right,'alpha.txt')).to_be_visible()
                file(right,'welcome.txt').click(button='right');expect(menu.get_by_role('button',name='Download',exact=True)).to_have_count(0);page.keyboard.press('Escape');right.get_by_role('button',name='Download',exact=True).click();expect(page.locator('.queue-row')).to_have_count(2);expect(page.locator('.queue-row').last).to_contain_text('Completed');assert (local/'welcome.txt').read_bytes()==(remote_root/'welcome.txt').read_bytes()
                right.get_by_role('button',name='Expand folder',exact=True).click();expect(file(right,'nested.txt')).to_be_visible();file(right,'nested.txt').click(button='right');action('Rename')
                name=right.locator('.file-inline-name')
                expect(name).to_be_focused()
                name.fill('renamed-nested.txt');name.press('Enter')
                expect(name).to_have_count(0)
                right.get_by_role('button',name='Expand folder',exact=True).click();expect(file(right,'renamed-nested.txt')).to_be_visible();assert (remote_root/'folder'/'renamed-nested.txt').read_bytes()==b'remote nested';assert not (remote_root/'renamed-nested.txt').exists()
                file(right,'renamed-nested.txt').click(button='right');action('Properties');expect(dialog).to_contain_text('/folder/renamed-nested.txt');expect(dialog).to_contain_text('Permissions');dialog.get_by_role('button',name='Close dialog',exact=True).click()
                checks.append('Context upload and toolbar download use actual SFTP; expanded remote children rename in their own directory and show permissions/path properties')
                deletion=page.get_by_role('dialog',name='Delete permanently',exact=True)
                file(left,'after-refresh.txt').click(button='right');action('Delete permanently');expect(deletion.locator('.file-delete-name')).to_have_text('after-refresh.txt');expect(deletion.locator('.file-delete-location')).to_contain_text(str(local));expect(deletion).to_contain_text('This cannot be undone.');deletion.get_by_role('button',name='Cancel',exact=True).click();assert (local/'after-refresh.txt').exists()
                file(left,'after-refresh.txt').click();page.keyboard.press('Shift+Delete');expect(deletion).to_be_visible();page.screenshot(path=str(out/'delete-confirm.png'));deletion.get_by_role('button',name='Delete permanently',exact=True).click();expect(deletion).not_to_be_visible();expect(file(left,'after-refresh.txt')).to_have_count(0);assert not (local/'after-refresh.txt').exists()
                (local/'delete-tree'/'nested').mkdir(parents=True);(local/'delete-tree'/'nested'/'leaf.txt').write_bytes(b'tree');(local/'delete-one.txt').write_bytes(b'one');(local/'delete-two.txt').write_bytes(b'two');file(left,'alpha.txt').click();page.keyboard.press('F5');expect(file(left,'delete-tree')).to_be_visible()
                file(left,'delete-one.txt').click();file(left,'delete-two.txt').click(modifiers=['Control']);file(left,'delete-one.txt').click(button='right');action('Delete permanently');expect(deletion).to_contain_text('2 local items');deletion.get_by_role('button',name='Delete permanently',exact=True).click();expect(deletion).not_to_be_visible();assert not (local/'delete-one.txt').exists() and not (local/'delete-two.txt').exists()
                file(left,'delete-tree').click(button='right');action('Delete permanently');deletion.get_by_role('button',name='Delete permanently',exact=True).click();expect(file(left,'delete-tree')).to_have_count(0);assert not (local/'delete-tree').exists()
                checks.append('Permanent removal and Shift+Delete require confirmation, Cancel preserves bytes, multi-selection deletes exact targets and folders are removed recursively')
                file(right,'renamed-nested.txt').click(button='right');action('Delete permanently');expect(deletion.locator('.file-delete-name')).to_have_text('renamed-nested.txt');expect(deletion.locator('.file-delete-location')).to_have_text('Remote files · /folder');deletion.get_by_role('button',name='Cancel',exact=True).click();assert (remote_root/'folder'/'renamed-nested.txt').exists()
                file(right,'renamed-nested.txt').click(button='right');action('Delete permanently');deletion.get_by_role('button',name='Delete permanently',exact=True).click();expect(deletion).not_to_be_visible();assert not (remote_root/'folder'/'renamed-nested.txt').exists()
                (remote_root/'delete-tree'/'nested').mkdir(parents=True);(remote_root/'delete-tree'/'nested'/'leaf').write_bytes(b'remote tree');(remote_root/'fixture-delete-denied-ui').write_bytes(b'keep');(remote_root/'delete-two.txt').write_bytes(b'two');file(right,'alpha.txt').click();page.keyboard.press('F5');expect(file(right,'delete-tree')).to_be_visible()
                file(right,'delete-tree').click();file(right,'delete-two.txt').click(modifiers=['Control']);file(right,'delete-tree').click(button='right');action('Delete permanently');expect(deletion).to_contain_text('2 remote items');deletion.get_by_role('button',name='Delete permanently',exact=True).click();expect(deletion).not_to_be_visible();assert not (remote_root/'delete-tree').exists() and not (remote_root/'delete-two.txt').exists()
                file(right,'fixture-delete-denied-ui').click(button='right');action('Delete permanently');deletion.get_by_role('button',name='Delete permanently',exact=True).click();expect(deletion).not_to_be_visible();elevation=page.get_by_role('dialog',name='Administrator permission required',exact=True);expect(elevation).to_contain_text('fixture-delete-denied-ui');elevation.get_by_role('button',name='Cancel',exact=True).click();assert (remote_root/'fixture-delete-denied-ui').read_bytes()==b'keep';expect(file(right,'fixture-delete-denied-ui')).to_be_visible()
                checks.append('Remote deletion uses SFTP, preserves canceled selections, removes expanded children and multiple trees, and reports denied deletion without hiding the surviving file')
                check_delete_batch(page, left, local/'batch-review', out)
                left.get_by_role('button', name='Local parent directory', exact=True).click()
                checks.append('A 103-item selection has a bounded preview, complete paged review and safe cancellation')
                left.get_by_role('button',name='Icon grid',exact=True).click();file(left,'alpha.txt').click(button='right');expect(menu).to_be_visible();page.mouse.click(5,5);expect(menu).to_have_count(0)
                left.get_by_role('button',name='Local file actions',exact=True).click();more=page.locator('.file-actions-menu');expect(more).to_be_visible();expect(more.get_by_role('button',name='Rename',exact=True)).to_be_enabled();page.keyboard.press('Escape');expect(more).to_have_count(0)
                page.set_viewport_size(dict(width=1000,height=700));file(right,'alpha.txt').click(button='right');expect(menu).to_be_visible();box=menu.bounding_box();assert box['x']>=7 and box['y']>=7 and box['x']+box['width']<=993 and box['y']+box['height']<=693;page.screenshot(path=str(out/'menu-narrow.png'));menu.get_by_role('button',name='Properties',exact=True).press('Escape');expect(menu).to_have_count(0)
                checks.append('Icon grids and existing More buttons share the same actions; outside click dismisses and narrow-window menus stay inside the viewport')
                page.set_viewport_size(dict(width=1440,height=980))
                page.locator('.host-select').nth(1).click();page.locator('.overview-transfer').click();expect(page.locator('.connection-state')).to_have_text('SSH connected')
                expect(right.locator('.pane-summary').first).to_contain_text('Unavailable')
                blank(right);expect(menu.get_by_role('button',name='New file',exact=True)).to_be_disabled();expect(menu.get_by_role('button',name='New folder',exact=True)).to_be_disabled();expect(menu.get_by_role('button',name='Refresh',exact=True)).to_be_disabled();expect(menu.get_by_role('button',name='Visible columns',exact=True)).to_be_focused();page.keyboard.press('Escape');expect(menu).to_have_count(0)
                page.locator('.settings-link').click();page.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check();expect(page.locator('.context-title')).to_have_text('设置');page.locator('.rail-servers').click();left.get_by_role('button',name='alpha.txt',exact=True).click(button='right');expect(menu.get_by_role('button',name='属性',exact=True)).to_be_visible();expect(menu.get_by_role('button',name='刷新',exact=True)).to_have_count(0);page.screenshot(path=str(out/'menu-chinese.png'));page.keyboard.press('Escape')
                blank(left);expect(menu.get_by_role('button',name='刷新',exact=True)).to_be_visible();page.keyboard.press('Escape')
                left.get_by_role('button',name='平铺图标',exact=True).click()
                (local/'新建文件夹').write_bytes(b'keep')
                left.get_by_role('button',name='本地文件操作',exact=True).click()
                page.locator('.file-actions-menu').get_by_role('button',name='新建文件夹',exact=True).click()
                name=left.locator('.icons-view .file-inline-name')
                expect(name).to_be_focused()
                expect(name).to_have_value('新建文件夹 (2)')
                name.press('Escape')
                expect(name).to_have_count(0)
                assert (local/'新建文件夹').read_bytes()==b'keep' and (local/'新建文件夹 (2)').is_dir()
                checks.append('Unavailable SFTP actions are disabled without a focus trap, and context labels follow the application language')
                result=dict(passed=len(checks),checks=checks);(out/'results.json').write_text(json.dumps(result,indent=2));print(json.dumps(result),flush=True)
            except Exception:
                try:page.screenshot(path=str(out/'failure.png'));(out/'failure.txt').write_text(page.locator('body').inner_text(),encoding='utf-8')
                except Exception:pass
                raise
            finally:browser.close()
    finally:
        if app and app.poll() is None:app.terminate();app.wait(timeout=10)
        fixture.terminate();fixture.wait(timeout=10);log.close()
if __name__=='__main__':main()
