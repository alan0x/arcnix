"""Read local Office files in WebView2 (python-docx, openpyxl and python-pptx)."""
from hashlib import sha256
from pathlib import Path
import tempfile
import time

from desktop_fixture import Desktop, ROOT, eventually, expect
from docx import Document
from openpyxl import Workbook
from pptx import Presentation
from pptx.util import Inches


def fixtures(folder):
    doc = Document()
    doc.add_heading('本机阅读测试', 1)
    doc.add_paragraph('Offline document preview')
    doc.add_table(rows=1, cols=1).cell(0, 0).text = '表格内容'
    doc.save(folder / '报告.DOCX')

    book = Workbook()
    book.active.title = '收入'
    book.active['A1'] = 'First sheet'
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
            app.application('Office')
            host = p.locator('.desktop-window.app-office')
            expect(host.locator('.viewer-placeholder')).to_contain_text('DOCX')
            host.locator('.window-close').click()
            app.application('Files')
            app.file(folder.name).dblclick()
            app.file('报告.DOCX').dblclick()

            def child(kind):
                def ready():
                    p.wait_for_timeout(75)
                    for context in app.browser.contexts:
                        for page in context.pages:
                            if page.url.endswith('/document') and page.locator(
                                    f'.reader-paper[data-kind="{kind}"]').count():
                                return page
                return eventually(ready, timeout=30)

            def pick(name):
                host.get_by_role('button', name='Open Office document').click()
                picker = p.get_by_role('dialog', name='Open Office document')
                picker.get_by_role('button', name=name, exact=True).click()
                picker.get_by_role('button', name='Open', exact=True).click()

            word = child('writer')
            expect(host.get_by_role('button', name='Edit Word document')).to_have_count(0)
            expect(word.locator('.reader-paper')).to_contain_text('本机阅读测试')
            expect(word.locator('.reader-paper td')).to_contain_text('表格内容')
            zoom = word.locator('output')
            before = zoom.text_content()
            word.get_by_role('button', name='Zoom in').click()
            expect(zoom).not_to_have_text(before)
            word.get_by_role('button', name='Fit width').click()
            word.screenshot(path=str(out / 'word.png'))

            pick('预算.xlsx')
            sheet = child('calc')
            picker = sheet.get_by_role('combobox', name='Worksheet')
            expect(picker.locator('option')).to_have_text(['收入', '支出'])
            picker.select_option(label='支出')
            expect(sheet.get_by_text('Second sheet', exact=True)).to_be_visible()
            expect(sheet.get_by_text('First sheet', exact=True)).to_be_hidden()
            sheet.screenshot(path=str(out / 'excel.png'))

            pick('汇报.pptx')
            deck = child('slides')
            expect(deck.get_by_text('First slide', exact=True).first).to_be_visible()
            deck.get_by_role('button', name='Next', exact=True).click()
            expect(deck.get_by_text('Second slide', exact=True).first).to_be_visible()
            expect(deck.get_by_role('button', name='Next', exact=True)).to_be_disabled()
            deck.screenshot(path=str(out / 'slides.png'))

            pick('broken.docx')
            error = host.locator('.viewer-placeholder')
            expect(error).not_to_contain_text('Opening Office document')
            expect(error).not_to_be_empty()
            pick('large.xlsx')
            expect(error).to_contain_text('32 MiB')
            pick('报告.DOCX')
            expect(child('writer').locator('.reader-paper')).to_contain_text('Offline document preview')
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
            host = p.locator('.desktop-window.app-office')
            host.get_by_role('button', name='Edit Word document').click()

            def editor_ready():
                p.wait_for_timeout(75)
                return next((page for ctx in app.browser.contexts for page in ctx.pages
                             if page.url.endswith('/edit') and page.locator('#body[contenteditable="true"]').count()), None)

            editor = eventually(editor_ready, timeout=30)
            run = editor.get_by_text('Offline document preview', exact=True).last
            run.click()
            editor.keyboard.press('End')
            editor.keyboard.insert_text(' · 已修改')
            expect(editor.locator('#body')).to_contain_text('Offline document preview · 已修改')
            run = editor.locator('.writer-surface-run').filter(has_text='Offline document preview').first
            run.evaluate("""el => { const r = document.createRange(); r.selectNodeContents(el);
                const s = getSelection(); s.removeAllRanges(); s.addRange(r);
                document.dispatchEvent(new Event('selectionchange')); }""")
            editor.get_by_role('button', name='Bold', exact=True).click()
            editor.screenshot(path=str(out / 'word-editing.png'))
            editor.get_by_role('button', name='Save as DOCX…').click()
            picker = p.get_by_role('dialog', name='Save as…')
            name = picker.get_by_role('textbox', name='File name')
            name.fill('报告.DOCX')
            picker.get_by_role('button', name='Save', exact=True).click()
            expect(picker).to_contain_text('Choose a new DOCX file name.')
            name.fill('broken.docx')
            picker.get_by_role('button', name='Save', exact=True).click()
            expect(host.locator('.office-message')).to_have_text('Choose a new DOCX file name.')
            expect(editor.locator('#body')).to_contain_text(' · 已修改')
            editor.get_by_role('button', name='Save as DOCX…').click()
            name.fill('修订版.docx')
            picker.get_by_role('button', name='Save', exact=True).click()
            expect(host.locator('.viewer-filename')).to_have_text('修订版.docx', timeout=30000)
            copy = Document(folder / '修订版.docx')
            assert copy.paragraphs[0].text == '本机阅读测试'
            assert copy.paragraphs[1].text == 'Offline document preview · 已修改'
            assert any(r.bold for r in copy.paragraphs[1].runs)
            assert copy.tables[0].cell(0, 0).text == '表格内容'
        assert all(sha256(p.read_bytes()).digest() == digest for p, digest in originals.items())
    print(out)


if __name__ == '__main__':
    run()
