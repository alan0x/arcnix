"""Open real PDF pages over SFTP and on the read-only desktop (requires reportlab)."""
from pathlib import Path
import tempfile
import time

from reportlab.pdfgen.canvas import Canvas
from reportlab.lib.pdfencrypt import StandardEncryption

from window_tabs_smoke import round_trip
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


def expect_sidebar_width(sidebar, width):
    expect(sidebar).to_be_visible()
    eventually(lambda: abs(sidebar.bounding_box()['width'] - width) < 1)


def main():
    out = ROOT / 'artifacts/pdf-reader' / str(time.time_ns())
    name = 'Field notes 现场.PDF'
    with Peer(out / 'peer') as peer:
        desktop_prefs = peer.root / 'home/arcnix/.arcnix/desktop.json'
        desktop_prefs.parent.mkdir(parents=True, exist_ok=True)
        desktop_prefs.write_text('{"version":1,"theme":"Dark"}', encoding='utf-8')
        docs = peer.root / 'Documents'
        docs.mkdir()
        fixture(docs / name)
        fixture(docs / 'Second.pdf')
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
                win.locator('.desktop-window-actions').get_by_role('button', name='Open PDF').click()
                picker = p.get_by_role('dialog', name='Open PDF')
                expect(picker.get_by_role('button', name='notes.txt')).to_have_count(0)
                picker.get_by_role('button', name=filename, exact=True).click()
                picker.get_by_role('button', name='Open', exact=True).click()

            r, g, b = rendered(1)
            assert b > g > r
            expect(reader.locator('.pdf-total')).to_have_text('/ 3')
            expect(win.locator('.desktop-window-titlebar .pdf-open + .window-minimize')).to_have_count(1)
            expect(win.locator('.desktop-window-titlebar .window-tab-select')).to_have_text(name)
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
            zoom = reader.get_by_role('textbox', name='Zoom percentage')
            bar = reader.locator('.viewer-toolbar').bounding_box()
            groups = reader.locator('.viewer-controls')
            first, last = groups.first.bounding_box(), groups.last.bounding_box()
            assert abs(first['x'] + last['x'] + last['width'] - 2 * bar['x'] - bar['width']) < 3
            box = reader.locator('.pdf-zoom-box').bounding_box()
            def center_y(bounds):
                return bounds['y'] + bounds['height'] / 2
            assert abs(center_y(zoom.bounding_box()) - center_y(box)) < 1
            one_digit = page.bounding_box()
            assert one_digit['width'] < 40 and abs(one_digit['height'] - box['height']) < 1
            page.fill('12345')
            assert page.evaluate('e => e.scrollWidth <= e.clientWidth + 1')
            page.press('Enter')
            expect(page).to_have_value('3')
            assert abs(page.bounding_box()['width'] - one_digit['width']) < 1
            reader.get_by_role('button', name='Zoom presets').click()
            win.screenshot(path=str(out / 'zoom-menu.png'))
            reader.get_by_role('menuitemradio', name='150%', exact=True).click()
            expect(zoom).to_have_value('150%')
            preset_width = image.bounding_box()['width']
            zoom.fill('105.3%')
            zoom.press('Enter')
            expect(reader.locator('.pdf-scale')).to_have_text('105.3%')
            eventually(lambda: abs(image.bounding_box()['width'] / preset_width - 1.053 / 1.5) < 0.01)
            pick('Invalid.pdf')
            expect(reader.locator('.viewer-placeholder')).to_contain_text('Could not read this PDF.')
            def check_pdf_split(active, other):
                expect(active.locator('.viewer-placeholder')).to_contain_text('Could not read this PDF.')
                expect(other.locator('.pdf-page-number')).to_have_value('3')
                expect(other.locator('.pdf-zoom-input')).to_have_value('105.3%')

            round_trip(p, 'document', check_pdf_split)
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
            tabs = win.locator('.pdf-tabs .window-tab-select')
            expect(tabs).to_have_count(4)
            pick(name)
            expect(tabs).to_have_count(4)
            rendered(3)
            expect(zoom).to_have_value('105.3%')
            win.screenshot(path=str(out / 'tabs-and-zoom.png'))
            win.locator('.window-minimize').click()
            p.locator('.desktop-dock').get_by_role('button', name='PDF Reader', exact=True).click(button='right')
            menu = p.locator('.desktop-context-menu')
            expect(menu.get_by_role('button', name='Dock settings')).to_have_count(0)
            expect(menu.locator('.dock-document')).to_have_count(4)
            menu.get_by_role('button', name='Protected.pdf', exact=True).click()
            expect(image).to_have_attribute('alt', 'Protected.pdf · 1')
            win.get_by_role('button', name='Close file tab Huge.pdf').click()
            expect(tabs).to_have_count(3)
            expect(image).to_have_attribute('alt', 'Protected.pdf · 1')
            win.get_by_role('button', name='Close file tab Protected.pdf').click()
            win.get_by_role('button', name='Close file tab Invalid.pdf').click()
            rendered(3)
            dock = p.locator('.desktop-dock').get_by_role('button', name='PDF Reader', exact=True)
            dock.click(button='right')
            menu.get_by_role('button', name='New window').click()
            expect(p.locator('.desktop-window.app-document')).to_have_count(2)
            pick('Second.pdf')
            expect(image).to_have_attribute('alt', 'Second.pdf · 1', timeout=30000)
            zoom.fill('80%')
            zoom.press('Enter')
            expect(reader.locator('.pdf-scale')).to_have_text('80%')
            pick(name)
            rendered(3)
            expect(zoom).to_have_value('105.3%')
            expect(tabs).to_have_count(1)
            dock.click(button='right')
            menu.get_by_role('button', name='Second.pdf', exact=True).click()
            expect(zoom).to_have_value('80%')
            win.locator('.window-close').click()
            expect(p.locator('.desktop-window.app-document')).to_have_count(1)
            rendered(3)
            win.get_by_role('button', name='Close file tab ' + name).click()
            expect(reader.locator('.viewer-placeholder')).to_contain_text('Open a PDF to read it.')
            win.locator('.window-close').click()
            expect(p.locator('.desktop-window.app-document')).to_have_count(0)
            app.application('PDF Reader')
            expect(reader.locator('.viewer-placeholder')).to_contain_text('Open a PDF to read it.')

    with tempfile.TemporaryDirectory(dir=Path.home()) as d:
        fixture(Path(d) / name)
        cfg = dict(version=1, config=dict(servers=[], groups=[], trusted_hosts=[],
                                          settings=dict(language='English', show_hidden_files=True, resource_interval_seconds=3)))
        with Desktop(out / 'local', cfg) as app:
            p = app.page
            p.get_by_role('button', name='Try the local desktop', exact=True).click()
            p.get_by_role('button', name='Read-only mode', exact=True).click()
            expect(p.locator('.connection-state')).to_have_text('Connected locally')
            app.application('Files')
            files = p.locator('.app-files.active')
            def resize_sidebar(root, delta):
                sidebar = root.locator('.files-sidebar')
                root.locator('.files-sidebar-splitter').hover()
                before = round(sidebar.bounding_box()['width'])
                handle = root.locator('.files-sidebar-splitter').bounding_box()
                x, y = handle['x'] + handle['width'] / 2, handle['y'] + handle['height'] / 2
                p.mouse.move(x, y)
                p.mouse.down()
                p.mouse.move(x + delta, y, steps=8)
                p.mouse.up()
                eventually(lambda: abs(sidebar.bounding_box()['width'] - before - delta) < 2)
                return round(sidebar.bounding_box()['width'])

            files_width = resize_sidebar(files, 52)
            app.file(Path(d).name).dblclick()
            app.file(name).dblclick()
            reader = p.locator('.pdf-reader')
            expect(reader.locator('.pdf-sheet img')).to_have_attribute('alt', name + ' · 1', timeout=30000)
            reader.get_by_role('button', name='Next page').click()
            expect(reader.locator('.pdf-sheet img')).to_have_attribute('alt', name + ' · 2', timeout=30000)

            p.locator('.app-document.active .pdf-open').click()
            picker = p.get_by_role('dialog', name='Open PDF')
            picker_width = resize_sidebar(picker, 31)
            picker.get_by_role('button', name='Cancel', exact=True).click()
            app.application('Office')
            p.locator('.app-office.active .office-open').click()
            picker = p.get_by_role('dialog', name='Open Office document')
            expect_sidebar_width(picker.locator('.files-sidebar'), picker_width)
            picker.get_by_role('button', name='Cancel', exact=True).click()

        with Desktop(out / 'local', None) as app:
            p = app.page
            p.get_by_role('button', name='Try the local desktop', exact=True).click()
            p.get_by_role('button', name='Read-only mode', exact=True).click()
            app.application('Files')
            expect_sidebar_width(p.locator('.app-files.active .files-sidebar'), files_width)
            app.application('PDF Reader')
            p.locator('.app-document.active .pdf-open').click()
            picker = p.get_by_role('dialog', name='Open PDF')
            expect_sidebar_width(picker.locator('.files-sidebar'), picker_width)


if __name__ == '__main__':
    main()
