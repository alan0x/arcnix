"""Open real PDF pages over SFTP and on the read-only desktop (requires reportlab)."""
from pathlib import Path
import tempfile
import time

from reportlab.pdfgen.canvas import Canvas
from reportlab.lib.pdfencrypt import StandardEncryption

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def fixture(path, password=None):
    pdf = Canvas(str(path), pagesize=(360, 480),
                 encrypt=StandardEncryption(password) if password else None)
    for number, (width, height, rgb) in enumerate([
            (360, 480, (0.12, 0.40, 0.54)), (480, 300, (0.10, 0.48, 0.35)),
            (360, 480, (0.68, 0.31, 0.21))], 1):
        pdf.setPageSize((width, height))
        pdf.setFillColorRGB(*rgb)
        pdf.rect(0, height - 100, width, 100, fill=1, stroke=0)
        pdf.setFillColorRGB(1, 1, 1)
        pdf.setFont('Helvetica', 23)
        pdf.drawString(24, height - 62, f'Field notes  /  {number}')
        pdf.setFillColorRGB(0.12, 0.14, 0.16)
        pdf.setFont('Helvetica', 12)
        pdf.drawString(24, height - 140, 'Survey results and measurements')
        pdf.showPage()
    pdf.save()


def main():
    out = ROOT / 'artifacts/pdf-reader' / str(time.time_ns())
    name = 'Field notes 现场.PDF'
    with Peer(out / 'peer') as peer:
        docs = peer.root / 'Documents'
        docs.mkdir()
        fixture(docs / name)
        fixture(docs / 'Protected.pdf', 'reader-test')
        (docs / 'Invalid.pdf').write_bytes(b'not a PDF')
        (docs / 'notes.txt').write_text('unrelated')
        with (docs / 'Huge.pdf').open('wb') as f:
            f.truncate(64 * 1024 * 1024 + 1)
        profile = {**peer.profile('Documents'), 'initial_remote_path': '/Documents'}
        with Desktop(out / 'remote', peer.config([profile], out)) as app:
            p = app.page
            app.connect()
            app.file(name).dblclick()
            win = p.locator('.desktop-window.app-document.active')
            reader = win.locator('.pdf-reader')
            page = reader.get_by_role('textbox', name='Page number')
            image = reader.locator('.pdf-sheet img')

            def rendered(number):
                expect(page).to_have_value(str(number))
                expect(image).to_have_attribute('alt', f'{name} · {number}', timeout=30000)
                eventually(lambda: image.evaluate('e => e.complete && e.naturalWidth > 0'))
                return image.evaluate("""e => {
                    const c = document.createElement('canvas');
                    c.width = c.height = 1;
                    const ctx = c.getContext('2d');
                    ctx.drawImage(e, 25, 25, 1, 1, 0, 0, 1, 1);
                    return [...ctx.getImageData(0, 0, 1, 1).data].slice(0, 3);
                }""")

            def pick(filename):
                reader.get_by_role('button', name='Open PDF').click()
                picker = p.get_by_role('dialog', name='Open PDF')
                expect(picker.get_by_role('button', name='notes.txt')).to_have_count(0)
                picker.get_by_role('button', name=filename, exact=True).click()
                picker.get_by_role('button', name='Open', exact=True).click()

            r, g, b = rendered(1)
            assert b > g > r
            expect(reader.locator('.pdf-total')).to_have_text('/ 3')
            win.screenshot(path=str(out / 'reader.png'))
            expect(reader.get_by_role('button', name='Previous page')).to_be_disabled()
            reader.get_by_role('button', name='Next page').click()
            r, g, b = rendered(2)
            assert g > b > r
            page.fill('3')
            page.press('Enter')
            r, g, b = rendered(3)
            assert r > g > b
            expect(reader.get_by_role('button', name='Next page')).to_be_disabled()
            page.fill('99')
            page.press('Enter')
            expect(page).to_have_value('3')
            reader.get_by_role('button', name='Fit width').click()
            expect(reader.locator('.pdf-width')).to_have_attribute('aria-pressed', 'true')
            stage = reader.locator('.pdf-stage')
            eventually(lambda: stage.evaluate('e => e.scrollHeight > e.clientHeight'))
            width = image.bounding_box()['width']
            reader.get_by_role('button', name='Zoom in').click()
            eventually(lambda: image.bounding_box()['width'] > width)
            reader.get_by_role('button', name='Fit page').click()
            expect(reader.locator('.pdf-fit')).to_have_attribute('aria-pressed', 'true')
            reader.get_by_role('button', name='View fullscreen in Desktop').click()
            eventually(lambda: abs(win.bounding_box()['width'] - p.locator('.desktop-canvas').bounding_box()['width']) < 1)
            assert p.evaluate('document.fullscreenElement === null')
            p.keyboard.press('Escape')
            expect(reader.get_by_role('button', name='View fullscreen in Desktop')).to_be_visible()
            pick('Invalid.pdf')
            expect(reader.locator('.viewer-placeholder')).to_contain_text('Could not read this PDF.')
            pick('Huge.pdf')
            expect(reader.locator('.viewer-placeholder')).to_contain_text('64 MiB')
            pick('Protected.pdf')
            secret = reader.get_by_label('PDF password', exact=True)
            secret.fill('wrong')
            reader.get_by_role('button', name='Unlock PDF').click()
            expect(reader.locator('.viewer-placeholder')).to_contain_text('Incorrect PDF password.')
            secret.fill('reader-test')
            secret.press('Enter')
            expect(image).to_have_attribute('alt', 'Protected.pdf · 1', timeout=30000)
            win.locator('.window-close').click()
            app.application('PDF Reader')
            expect(reader.locator('.viewer-placeholder')).to_contain_text('Open a PDF to read it.')

    with tempfile.TemporaryDirectory(dir=Path.home()) as d:
        fixture(Path(d) / name)
        cfg = dict(version=1, config=dict(servers=[], groups=[], trusted_hosts=[],
                                          settings=dict(language='English')))
        with Desktop(out / 'local', cfg) as app:
            p = app.page
            p.get_by_role('button', name='Try the local desktop', exact=True).click()
            p.get_by_role('button', name='Read-only mode', exact=True).click()
            expect(p.locator('.connection-state')).to_have_text('Connected locally')
            app.application('Files')
            app.file(Path(d).name).dblclick()
            app.file(name).dblclick()
            reader = p.locator('.pdf-reader')
            expect(reader.locator('.pdf-sheet img')).to_have_attribute('alt', name + ' · 1', timeout=30000)
            reader.get_by_role('button', name='Next page').click()
            expect(reader.locator('.pdf-sheet img')).to_have_attribute('alt', name + ' · 2', timeout=30000)


if __name__ == '__main__':
    main()
