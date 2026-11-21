"""Exercise the local VS Code workbench against a disposable SFTP server."""
import base64
import json
import re
import time
from desktop_fixture import Desktop, Peer, ROOT, expect, eventually

def main():
    out = ROOT / 'artifacts/code-workbench' / str(time.time_ns())
    out.mkdir(parents=True)
    with Peer(out / 'ssh') as peer:
        folder = peer.root / '文档'
        folder.mkdir()
        path = folder / 'notes.md'
        source = '# 运维笔记\n\n![状态](status.png)\n'
        path.write_bytes(b'\xef\xbb\xbf' + source.replace('\n', '\r\n').encode())
        (folder/'status.png').write_bytes(base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a3X0AAAAASUVORK5CYII='))
        with Desktop(out/'app',peer.config([peer.profile('Workbench')],out)) as app:
            app.connect()
            app.application('Code Editor')
            expect(app.page.locator('.desktop-dock').get_by_role('button',name='Editor',exact=True)).to_have_count(0)
            page = app.code()
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            host = app.page.locator('.app-code .desktop-window-titlebar')
            embedded = page.locator('.part.titlebar')
            label = embedded.locator('.menubar-menu-title:visible').first
            expect(page.locator('html')).to_have_class(re.compile('has-menu-slot'))
            def colors(node):
                return node.evaluate('el => [getComputedStyle(el).backgroundColor, getComputedStyle(el).color]')
            eventually(lambda: colors(embedded) == colors(host))
            for background, foreground in [('#222927', '#edfffa'), ('#f3f5fa', '#182233')]:
                host.evaluate('(el, pair) => { el.style.setProperty("--surface", pair[0]); el.style.color = pair[1]; }',
                              [background, foreground])
                eventually(lambda: colors(embedded) == colors(host))
                eventually(lambda: colors(label)[1] == colors(host)[1])
            host.evaluate('el => { el.style.removeProperty("--surface"); el.style.removeProperty("color"); }')
            eventually(lambda: colors(embedded) == colors(host))
            for action, name in [('New Folder...', 'new-folder'), ('New File...', 'new-file.txt')]:
                page.get_by_role('button',name=action,exact=True).click()
                field = page.locator('.explorer-viewlet input:visible')
                field.fill(name)
                field.press('Enter')
                eventually(lambda: next(peer.root.rglob(name), None) is not None)
            page.get_by_role('treeitem',name='文档',exact=True).click()
            file = page.get_by_role('treeitem',name='notes.md',exact=True)
            expect(file).to_be_visible()
            file.dblclick()
            area = page.locator('.monaco-editor textarea.inputarea').first
            expect(area).to_be_visible()
            eventually(lambda: page.locator('.view-line span[class]').evaluate_all('spans => new Set(spans.map(span => span.className)).size') > 1)
            area.focus()
            area.press('Control+k')
            area.press('v')
            preview = page.locator('.arcnix-preview')
            expect(preview.get_by_role('heading')).to_have_text('运维笔记')
            image = preview.get_by_role('img',name='状态')
            eventually(lambda: image.evaluate('img=>img.complete && img.naturalWidth===1'))
            area.focus()
            area.press('Control+Home')
            area.press('End')
            area.evaluate("""el=>{
                window.testComposition=[];
                for(const type of ['compositionstart','compositionupdate','compositionend'])
                    el.addEventListener(type,e=>testComposition.push([type,e.data]));
            }""")
            cdp = page.context.new_cdp_session(page)
            for stage in ['zhong','zhongwen','中文']:
                cdp.send('Input.imeSetComposition',dict(text=stage,selectionStart=len(stage),selectionEnd=len(stage)))
            cdp.send('Input.insertText',dict(text='中文'))
            area.press('Control+s')
            expected=b'\xef\xbb\xbf'+source.replace('笔记','笔记中文').replace('\n','\r\n').encode()
            eventually(lambda: path.read_bytes()==expected)
            expect(preview.get_by_role('heading')).to_have_text('运维笔记中文')
            events=page.evaluate('testComposition')
            assert ['compositionupdate','zhongwen'] in events and events[-1]==['compositionend','中文'],events
            assert page.locator('.editor-group-container').count()>=2
            page.screenshot(path=str(out/'workbench.png'))
            # The external edit has the same timestamp and length; hash checking still protects it.
            import os
            meta=path.stat()
            changed=expected.replace('中文'.encode(),'远端'.encode())
            path.write_bytes(changed)
            os.utime(path,ns=(meta.st_atime_ns,meta.st_mtime_ns))
            area.press('End');page.keyboard.insert_text('！');area.press('Control+s')
            expect(page.locator('.notifications-toasts')).to_contain_text('changed')
            assert path.read_bytes()==changed
            assert not errors, errors
            print(json.dumps(dict(output=str(out),composition=events,split=True,relative_image=True,conflict=True),ensure_ascii=False))

if __name__=='__main__':
    main()
