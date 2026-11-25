"""Read local Office files in WebView2 (python-docx, openpyxl and python-pptx)."""
import json
from hashlib import sha256
from io import BytesIO
from pathlib import Path
import tempfile
import time
import re

from desktop_fixture import Desktop, ROOT, eventually, expect, set_desktop_mode
from window_tabs_smoke import round_trip
from docx import Document
from docx.enum.text import WD_LINE_SPACING
from docx.shared import Pt
from openpyxl import Workbook
from openpyxl.styles import Border, Side
from PIL import Image
from pptx import Presentation
from pptx.util import Inches


def fixtures(folder):
    doc = Document()
    title = doc.add_heading('本机阅读测试', 1)
    title.paragraph_format.line_spacing = Pt(0.85)
    title.paragraph_format.line_spacing_rule = WD_LINE_SPACING.AT_LEAST
    doc.add_paragraph('Offline document preview')
    doc.add_table(rows=1, cols=1).cell(0, 0).text = '表格内容'
    doc.save(folder / '报告.DOCX')

    book = Workbook()
    book.active.title = '收入'
    book.active['A1'] = 'First sheet'
    book.active.merge_cells('A1:A3')
    book.active['A1'].border = Border(left=Side(style='thin', color='FF000000'))
    for row in range(1, 4):
        book.active.cell(row, 2, row)
    book.create_sheet('支出')['A1'] = 'Second sheet'
    book.save(folder / '预算.xlsx')

    slides = Presentation()
    for title in ('First slide', 'Second slide'):
        slide = slides.slides.add_slide(slides.slide_layouts[6])
        slide.shapes.add_textbox(Inches(1), Inches(1), Inches(5), Inches(1)).text = title
    slides.save(folder / '汇报.pptx')
    (folder / 'broken.docx').write_bytes(b'broken document')
    with (folder / 'large.xlsx').open('wb') as f:
        f.truncate(32 * 1024 * 1024 + 1)


def office_page(app, suffix, selector):
    def ready():
        app.page.wait_for_timeout(75)
        for ctx in app.browser.contexts:
            for page in ctx.pages:
                if page.url.endswith(suffix) and page.locator(selector).count():
                    return page
    return eventually(ready, timeout=30)


def check_docx_layout(page, paper):
    expect(paper).to_have_css('padding-left', '120px')
    expect(paper).to_have_css('padding-top', '96px')
    # Measure text extents, since glyphs can overflow a collapsed paragraph box.
    title = page.get_by_text('本机阅读测试', exact=True).last
    body = page.get_by_text('Offline document preview', exact=True).last
    def text_box(locator):
        return locator.evaluate('''el => {
            const range = new Range();
            range.selectNodeContents(el);
            return range.getBoundingClientRect().toJSON();
        }''')
    assert text_box(title)['bottom'] <= text_box(body)['top']


def check_print_preview(app, editor, mode):
    # The WebView2 preview is a browser-owned target, separate from the document.
    cdp = app.browser.new_browser_cdp_session()
    replies = {}
    try:
        editor.get_by_role('button', name='Print…').evaluate("b => setTimeout(() => b.click())")

        def preview():
            app.page.wait_for_timeout(40)
            targets = cdp.send('Target.getTargets')['targetInfos']
            return next((t for t in targets if t['url'] == 'edge://print/'), None)

        target = eventually(preview)
        session = cdp.send('Target.attachToTarget', {'targetId': target['targetId']})['sessionId']

        def received(event):
            if event['sessionId'] == session:
                message = json.loads(event['message'])
                if 'id' in message:
                    replies[message['id']] = message

        cdp.on('Target.receivedMessageFromTarget', received)
        sequence = 0

        def command(method, **params):
            nonlocal sequence
            sequence += 1
            cdp.send('Target.sendMessageToTarget', dict(sessionId=session, message=json.dumps(
                dict(id=sequence, method=method, params=params))))
            reply = eventually(lambda: replies.get(sequence) or app.page.wait_for_timeout(20))
            return reply['result']

        def cancel_node():
            nodes = command('Accessibility.getFullAXTree')['nodes']
            return next((n['backendDOMNodeId'] for n in nodes if
                         n.get('role', {}).get('value') == 'button'
                         and n.get('name', {}).get('value') == 'Cancel'), None)

        node = eventually(cancel_node)
        result = command('Runtime.evaluate', expression="matchMedia('(prefers-color-scheme: dark)').matches")
        assert result['result']['value'] == (mode == 'Dark')
        button = command('DOM.resolveNode', backendNodeId=node)['object']['objectId']
        command('Runtime.callFunctionOn', objectId=button,
                functionDeclaration='function () { setTimeout(() => this.click()); }')
        eventually(lambda: preview() is None)
    finally:
        cdp.detach()


def run():
    out = ROOT / 'artifacts' / 'office-reader' / str(time.time_ns())
    config = dict(version=1, config=dict(servers=[], groups=[], trusted_hosts=[],
                                        settings=dict(language='English')))
    with tempfile.TemporaryDirectory(dir=Path.home()) as tmp:
        folder = Path(tmp)
        fixtures(folder)
        originals = {p: sha256(p.read_bytes()).digest() for p in folder.iterdir()}
        with Desktop(out, config) as app:
            p = app.page
            p.get_by_role('button', name='Try the local desktop', exact=True).click()
            p.get_by_role('button', name='Read-only mode', exact=True).click()
            expect(p.locator('.connection-state')).to_have_text('Connected locally')
            app.application('pgAdmin')
            pgadmin = p.locator('.desktop-window.active.app-pgadmin')
            expect(pgadmin).to_contain_text('Open pgAdmin from a connected SSH server')
            pgadmin.locator('.window-close').click()
            app.application('Office')
            host = p.locator('.desktop-window.app-office.active')
            expect(host.locator('.viewer-placeholder')).to_contain_text('DOCX')
            host.locator('.window-close').click()
            app.application('Files')
            app.file(folder.name).dblclick()
            app.file('报告.DOCX').dblclick()

            def child(kind):
                return office_page(app, '/document', f'.reader-paper[data-kind="{kind}"]')

            def pick(name):
                host.locator('.desktop-window-actions').get_by_role('button', name='Open Office document', exact=True).click()
                picker = p.get_by_role('dialog', name='Open Office document')
                picker.get_by_role('button', name=name, exact=True).click()
                picker.get_by_role('button', name='Open', exact=True).click()

            host.locator('.window-minimize').click()
            app.page.locator('.desktop-dock').get_by_role('button', name='Office', exact=True).click(button='right')
            app.page.locator('.desktop-context-menu .dock-document').filter(has_text='报告.DOCX').click()
            word = child('writer')
            check_docx_layout(word, word.locator('.reader-paper'))
            expect(host.locator('.office-document-strip, .viewer-filename')).to_have_count(0)
            assert host.evaluate("""el => {
                const top = selector => el.querySelector(selector).getBoundingClientRect().top;
                return Math.abs(top('.browser-surface') - top('.desktop-window-content')) <= 1;
            }""")
            expect(word.get_by_role('button', name='Edit Word document')).to_have_count(0)
            expect(word.locator('.reader-paper')).to_contain_text('本机阅读测试')
            expect(word.locator('.reader-paper td')).to_contain_text('表格内容')
            p.locator('.desktop-dock').get_by_role('button', name='Office', exact=True).click()
            p.locator('.window-overview .overview-select').filter(has_text='报告.DOCX').click()
            expect(host).to_have_class(re.compile(r'\bactive\b'))
            expect(word.locator('.reader-paper')).to_contain_text('本机阅读测试')
            word.get_by_role('menuitem', name='File', exact=True).click()
            word.get_by_role('menuitem', name='Open…', exact=True).click()
            p.get_by_role('dialog', name='Open Office document').get_by_role('button', name='Cancel', exact=True).click()
            word.keyboard.press('Control+f')
            find = word.get_by_role('searchbox', name='Find text')
            find.fill('preview')
            expect(word.locator('.syntra-find output')).to_have_text('1 / 1')
            find.fill('[absent]')
            expect(word.locator('.syntra-find output')).to_have_text('No matches')
            find.press('Escape')
            word.evaluate('window.print = () => {window.printRequested = true}')
            word.get_by_role('button', name='Print…').click()
            assert word.evaluate('window.printRequested')
            word.emulate_media(media='print')
            expect(word.get_by_role('menubar')).to_be_hidden()
            expect(word.locator('.reader-paper')).to_be_visible()
            word.emulate_media(media='screen')
            zoom = word.locator('.syntra-toolbar output')
            before = zoom.text_content()
            word.get_by_role('button', name='Zoom in').click()
            expect(zoom).not_to_have_text(before)
            word.get_by_role('button', name='Fit width').click()
            word.screenshot(path=str(out / 'word.png'))

            pick('预算.xlsx')
            sheet = child('calc')
            # A rowspan abutting the row headers must have the same black edge on every row.
            grid = sheet.locator('.sheet:visible table')
            samples = grid.evaluate('''table => {
                const box = table.getBoundingClientRect();
                return [...table.tBodies[0].rows].map(row => {
                    const r = row.cells[0].getBoundingClientRect();
                    return [Math.round(r.right - box.left), Math.round(r.top + r.height / 2 - box.top)];
                });
            }''')
            with Image.open(BytesIO(grid.screenshot(scale='css'))) as raster:
                for x, y in samples:
                    assert min(max(raster.getpixel((x + dx, y))[:3]) for dx in range(-2, 3)) < 160
            picker = sheet.get_by_role('combobox', name='Worksheet')
            expect(picker.locator('option')).to_have_text(['收入', '支出'])
            picker.select_option(label='支出')
            expect(sheet.get_by_text('Second sheet', exact=True)).to_be_visible()
            expect(sheet.get_by_text('First sheet', exact=True)).to_be_hidden()
            sheet.screenshot(path=str(out / 'excel.png'))

            def check_office_split(active, other):
                expect(active.locator('.office-tabs .window-tab-item.active .editor-tab-label')).to_have_text('预算.xlsx')
                expect(other.locator('.office-tabs .window-tab-item.active .editor-tab-label')).to_have_text('报告.DOCX')
                expect(picker.locator('option:checked')).to_have_text('支出')

            round_trip(p, 'office', check_office_split)
            pick('汇报.pptx')
            deck = child('slides')
            expect(deck.get_by_text('First slide', exact=True).first).to_be_visible()
            deck.get_by_role('button', name='Next', exact=True).click()
            expect(deck.get_by_text('Second slide', exact=True).first).to_be_visible()
            expect(deck.get_by_role('button', name='Next', exact=True)).to_be_disabled()
            deck.screenshot(path=str(out / 'slides.png'))

            tabs = host.locator('.office-tabs .window-tab-select')
            expect(tabs).to_have_count(3)
            tabs.filter(has_text='预算.xlsx').click()
            expect(picker.locator('option:checked')).to_have_text('支出')
            dock = p.locator('.desktop-dock').get_by_role('button', name='Office', exact=True)
            menu = p.locator('.desktop-context-menu')
            dock.click(button='right')
            menu.get_by_role('button', name='New window').click()
            expect(p.locator('.desktop-window.app-office')).to_have_count(2)
            pick('broken.docx')
            error = host.locator('.viewer-placeholder')
            expect(error).not_to_contain_text('Opening Office document')
            expect(error).not_to_be_empty()
            pick('large.xlsx')
            expect(error).to_contain_text('32 MiB')
            pick('报告.DOCX')
            expect(tabs).to_have_count(3)
            expect(child('writer').locator('.reader-paper')).to_contain_text('Offline document preview')
            dock.click(button='right')
            menu.get_by_role('button', name='broken.docx', exact=True).click()
            expect(tabs).to_have_count(2)
            host.locator('.window-close').click()
            expect(p.locator('.desktop-window.app-office')).to_have_count(1)
            expect(tabs).to_have_count(3)
            host.locator('.window-close').click()
            app.application('Office')
            expect(host.locator('.viewer-placeholder')).to_contain_text('Open a DOCX')
        with Desktop(out / 'editing', config) as app:
            p = app.page
            p.get_by_role('button', name='Try the local desktop', exact=True).click()
            p.get_by_role('button', name='Read-write mode', exact=True).click()
            expect(p.locator('.connection-state')).to_have_text('Connected locally')
            app.application('Files')
            app.file(folder.name).dblclick()
            app.file('报告.DOCX').dblclick()
            host = p.locator('.desktop-window.app-office.active')
            editor = office_page(app, '/edit', '#body[contenteditable="true"]')
            check_docx_layout(editor, editor.locator('.writer-page'))
            active_label = host.locator('.office-tabs .window-tab-item.active .editor-tab-label')
            expect(active_label).to_have_text('报告.DOCX')
            editor.get_by_role('button', name='Return to reading').click()
            reader = office_page(app, '/document', '.reader-paper')
            reader.get_by_role('menuitem', name='Edit', exact=True).click()
            reader.get_by_role('menuitem', name='Edit Word document', exact=True).click()
            office_page(app, '/edit', '#body[contenteditable="true"]')
            # An untouched editable tab closes immediately. Reopening from Files also edits directly.
            host.locator('.window-close').click()
            expect(p.locator('.desktop-window.app-office')).to_have_count(0)
            app.application('Files')
            app.file('报告.DOCX').dblclick()
            editor = office_page(app, '/edit', '#body[contenteditable="true"]')
            editor.locator('#body').focus()
            editor.evaluate("""() => {
                const runs = document.querySelectorAll('#body .writer-surface-run');
                const range = new Range();
                range.setStart(runs[0].firstChild, 2);
                range.setEnd(runs[1].firstChild, 7);
                getSelection().removeAllRanges();
                getSelection().addRange(range);
            }""")
            selection = editor.evaluate("getSelection().toString()")
            assert '阅读测试' in selection and 'Offline' in selection
            editor.keyboard.press('Control+x')
            expect(editor.locator('#body')).not_to_contain_text('阅读测试')
            expect(editor.locator('#body')).not_to_contain_text('Offline')
            editor.keyboard.press('Control+z')
            expect(editor.locator('#body')).to_contain_text('本机阅读测试')
            expect(editor.locator('#body')).to_contain_text('Offline document preview')
            run = editor.get_by_text('Offline document preview', exact=True).last
            run.click()
            editor.keyboard.press('End')
            editor.keyboard.press('Control+Shift+ArrowLeft')
            assert editor.evaluate("getSelection().toString()") == 'preview'
            editor.keyboard.press('Control+x')
            expect(editor.locator('#body')).not_to_contain_text('preview')
            editor.keyboard.press('Control+v')
            expect(editor.locator('#body')).to_contain_text('Offline document preview')
            editor.keyboard.press('Control+z')
            expect(editor.locator('#body')).not_to_contain_text('preview')
            editor.keyboard.press('Control+z')
            expect(editor.locator('#body')).to_contain_text('Offline document preview')
            run.click()
            editor.keyboard.press('End')
            editor.keyboard.insert_text(' · 已修改')
            expect(editor.locator('#body')).to_contain_text('Offline document preview · 已修改')
            run = editor.locator('.writer-surface-run').filter(has_text='Offline document preview').first
            run.evaluate("""el => { const r = document.createRange(); r.selectNodeContents(el);
                const s = getSelection(); s.removeAllRanges(); s.addRange(r);
                document.dispatchEvent(new Event('selectionchange')); }""")
            editor.get_by_role('button', name='Bold', exact=True).click()
            editor.get_by_role('button', name='Undo', exact=True).click()
            expect(editor.get_by_role('button', name='Redo', exact=True)).to_be_enabled()
            editor.keyboard.press('Control+y')
            editor.get_by_role('menuitem', name='Format', exact=True).focus()
            editor.keyboard.press('ArrowDown')
            expect(editor.get_by_role('menuitem', name='Bold', exact=True)).to_be_focused()
            editor.keyboard.press('Escape')
            editor.get_by_role('button', name='Return to reading', exact=True).click()
            editor.get_by_role('button', name='Keep editing', exact=True).click()
            run = editor.locator('.writer-surface-run').filter(has_text='Offline document preview').first
            run.click()
            editor.get_by_role('combobox', name='Paragraph style').select_option('h2')
            expect(editor.locator('#body h2')).to_contain_text('已修改')
            editor.get_by_role('menuitem', name='Format', exact=True).click()
            editor.screenshot(path=str(out / 'word-editing.png'))
            editor.keyboard.press('Escape')
            editor.keyboard.press('Control+o')
            open_picker = p.get_by_role('dialog', name='Open Office document')
            open_picker.get_by_role('button', name='Cancel', exact=True).click()
            editor.get_by_role('menuitem', name='File', exact=True).click()
            expect(editor.get_by_role('menuitem', name='Save as…', exact=True)).to_be_visible()
            editor.get_by_role('menuitem', name='Open…', exact=True).click()
            open_picker.get_by_role('button', name='预算.xlsx', exact=True).click()
            open_picker.get_by_role('button', name='Open', exact=True).click()
            expect(host.locator('.office-tabs .window-tab-select')).to_have_count(2)
            expect(host.locator('.office-tabs .window-tab-item:not(.active) .editor-tab-label')).to_have_text('*报告.DOCX')
            expect(active_label).to_have_text('预算.xlsx')
            host.locator('.window-close').click()
            expect(active_label).to_have_text('*报告.DOCX')
            expect(editor.locator('#message')).to_be_empty()
            expect(editor.locator('#body h2')).to_contain_text('已修改')
            editor.get_by_role('button', name='Save as…', exact=True).click()
            picker = p.get_by_role('dialog', name='Save as…')
            name = picker.get_by_role('textbox', name='File name')
            # The native shell rejects a reloaded bridge. Its key handler must cancel browser refresh.
            p.evaluate("""window.addEventListener('keydown', e => { if (e.key === 'F5') window.f5Prevented = e.defaultPrevented; }, {once:true})""")
            name.press('F5')
            assert p.evaluate('window.f5Prevented === true')
            expect(picker).to_be_visible()
            name.fill('报告.DOCX')
            picker.get_by_role('button', name='Save', exact=True).click()
            expect(picker).to_contain_text('Choose a new DOCX file name.')
            name.fill('broken.docx')
            picker.get_by_role('button', name='Save', exact=True).click()
            expect(host.locator('.office-message')).to_have_text('Choose a new DOCX file name.')
            expect(editor.locator('#body')).to_contain_text(' · 已修改')
            editor.get_by_role('button', name='Save as…', exact=True).click()
            name.fill('修订版.docx')
            picker.get_by_role('button', name='Save', exact=True).click()
            expect(host.locator('.office-tabs .window-tab-item.active .editor-tab-label')).to_have_text('修订版.docx', timeout=30000)
            copy = Document(folder / '修订版.docx')
            assert copy.paragraphs[0].text == '本机阅读测试'
            assert copy.paragraphs[1].text == 'Offline document preview · 已修改'
            assert copy.paragraphs[1].style.name == 'Heading 2'
            assert any(r.bold for r in copy.paragraphs[1].runs)
            assert copy.tables[0].cell(0, 0).text == '表格内容'
            # Save repeatedly to the selected DOCX, check the destination and read the result from disk.
            copy_path = folder / '修订版.docx'
            for mode, marker in [('Light', ' · 保存'), ('Dark', ' · 再次保存')]:
                app.application('Settings')
                panel = p.locator('.desktop-settings-page')
                set_desktop_mode(panel, mode)
                if mode == 'Dark':
                    panel.get_by_role('button', name='#249688', exact=True).click()
                    expect(panel.get_by_role('textbox', name='Accent color', exact=True)).to_have_value('#249688')
                accent = panel.get_by_role('textbox', name='Accent color', exact=True).input_value()
                app.application('Office')
                editor = office_page(app, '/edit', '#body[contenteditable="true"]')
                bar = editor.locator('.syntra-command-bar')
                expect(bar).to_have_css('color-scheme', mode.lower())
                if mode == 'Dark':
                    expect(bar).to_have_css('accent-color', 'rgb(36, 150, 136)')
                check_print_preview(app, editor, mode)
                expect(editor.locator('.writer-page')).to_have_css('background-color', 'rgb(255, 255, 255)')
                paragraph = editor.locator('#body h2').first
                paragraph.click()
                editor.keyboard.press('End')
                editor.keyboard.insert_text(marker)
                editor.keyboard.press('Control+s')
                target = p.get_by_role('dialog', name='Save to the current file?')
                expect(target).to_contain_text(copy_path.name)
                expect(p.locator(f'.desktop-modal-backdrop.desktop-{mode.lower()}')).to_be_visible()
                save = target.get_by_role('button', name='Save', exact=True)
                color = ', '.join(str(int(accent[i:i+2], 16)) for i in (1, 3, 5))
                expect(save).to_have_css('background-color', f'rgb({color})')
                expect(target.get_by_role('button', name='Cancel', exact=True)).not_to_have_css(
                    'background-color', f'rgb({color})')
                target.screenshot(path=str(out / f'save-confirm-{mode.lower()}.png'))
                if mode == 'Light':
                    p.keyboard.press('Escape')
                    expect(target).to_have_count(0)
                    expect(paragraph).to_contain_text(marker)
                    expect(active_label).to_have_text('*修订版.docx')
                    editor.keyboard.press('Control+s')
                    target.get_by_role('button', name='Save as…', exact=True).click()
                    expect(picker).to_be_visible()
                    picker.get_by_role('button', name='Cancel', exact=True).click()
                    expect(paragraph).to_contain_text(marker)
                    editor.keyboard.press('Control+s')
                expect(save).to_be_focused()
                p.keyboard.press('Enter')
                eventually(lambda: Document(copy_path).paragraphs[1].text.endswith(marker))
                expect(host.locator('.office-message')).to_have_text('Document saved.')
                expect(active_label).to_have_text('修订版.docx')
            editor = office_page(app, '/edit', '#body[contenteditable="true"]')
            editor.locator('#body h2').first.click()
            editor.keyboard.press('End')
            editor.keyboard.insert_text(' · conflict')
            disk = Document(copy_path)
            disk.add_paragraph('External change')
            disk.save(copy_path)
            external = copy_path.read_bytes()
            editor.get_by_role('button', name='Save', exact=True).click()
            target = p.get_by_role('dialog', name='Save to the current file?')
            target.get_by_role('button', name='Save', exact=True).click()
            expect(host.locator('.office-message')).to_contain_text('file changed')
            expect(active_label).to_have_text('*修订版.docx')
            expect(editor.locator('#body')).to_contain_text('conflict')
            assert copy_path.read_bytes() == external
            editor.keyboard.press('Control+Shift+s')
            expect(picker).to_be_visible()

        assert all(sha256(p.read_bytes()).digest() == digest for p, digest in originals.items())
    print(out)


if __name__ == '__main__':
    run()
