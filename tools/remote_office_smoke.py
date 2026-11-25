"""SSH Office cache, conditional DOCX upload and recovery using an owned SFTP peer."""
from concurrent.futures import ThreadPoolExecutor
import json
import os
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect
from office_reader_smoke import fixtures
from docx import Document


def main():
    out = ROOT / 'artifacts/remote-office' / str(time.time_ns())
    local = out / 'local'
    local.mkdir(parents=True)
    with Peer(out / 'peer', io_delay=.03) as peer:
        fixtures(peer.root)
        source = peer.root / '报告.DOCX'
        desktop_files = peer.root / 'home/arcnix/Desktop'
        desktop_files.mkdir(parents=True, exist_ok=True)
        second = desktop_files / 'notes.docx'
        second.write_bytes(source.read_bytes())
        config = peer.config([peer.profile('First'), peer.profile('Second')], local)

        def open_file(app, name):
            app.application('Files')
            app.file(name).dblclick()
            return app.page.locator('.desktop-window.active.app-office')

        def content(app, selector):
            def find():
                app.page.wait_for_timeout(50)
                return next((page for ctx in app.browser.contexts for page in ctx.pages
                             if page.url.endswith(('/document', '/edit')) and page.locator(selector).count()), None)
            return eventually(find, timeout=30)

        def edit(app, host, extra):
            content(app, '.reader-paper').get_by_role('button', name='Edit Word document').click()
            editor = content(app, '#body[contenteditable="true"]')
            save = editor.get_by_role('button', name='Save to server')
            expect(save).to_be_disabled()
            line = editor.locator('.writer-surface-run').filter(has_text='Offline document preview').first
            line.click()
            editor.keyboard.press('End')
            editor.keyboard.insert_text(extra)
            expect(editor.locator('#body')).to_contain_text(extra)
            expect(save).to_be_enabled()
            return save

        cache = out / 'desktop/servers'
        with Desktop(out / 'desktop', config) as app:
            app.connect()
            for name, kind, text in [('预算.xlsx', 'calc', 'First sheet'), ('汇报.pptx', 'slides', 'First slide')]:
                host = open_file(app, name)
                reader = content(app, '.reader-paper[data-kind="' + kind + '"]')
                expect(reader.get_by_text(text, exact=True).first).to_be_visible()
                host.locator('.window-close').click()
                expect(app.page.locator('.desktop-window.app-office')).to_have_count(0)

            host = open_file(app, source.name)
            edit(app, host, ' first edit').click()
            expect(host.locator('.office-message')).to_have_text('Saved to server.')
            assert Document(source).paragraphs[1].text.endswith(' first edit')
            def disconnect():
                app.page.get_by_role('button', name='Disconnect', exact=True).click()
                expect(app.page.locator('.connection-state')).to_have_text('SSH disconnected')

            def activity():
                app.page.locator('.rail-activity').click()
                return app.page.locator('.office-sync-row').filter(has_text=source.name)

            disconnect()
            app.connect()
            host = open_file(app, source.name)
            expect(content(app, '.reader-paper').locator('.reader-paper')).to_contain_text(' first edit')

            gate = peer.root.parent / 'pause-office-upload'
            for drop, marker in [(False, ' after reconnect'), (True, ' after network loss')]:
                before = source.read_bytes()
                save = edit(app, host, marker)
                gate.with_suffix('.started').unlink(missing_ok=True)
                gate.touch()
                if drop:
                    gate.with_suffix('.drop').touch()
                try:
                    save.click()
                    eventually(gate.with_suffix('.started').exists)
                    app.application('Files')
                    uploading = app.file(source.name)
                    expect(uploading).to_be_disabled()
                    expect(uploading.locator('.file-upload-badge')).to_be_visible()
                    expect(activity()).to_contain_text('Uploading 0%')
                    app.page.locator('.rail-servers').click()
                    if not drop:
                        disconnect()
                finally:
                    gate.unlink(missing_ok=True)
                row = activity()
                expect(row).to_contain_text('Pending upload', timeout=20000)
                expect(app.page.locator('.rail-activity .sync-count')).to_have_text('1')
                assert source.read_bytes() == before
                rec_file = next(p for p in cache.rglob('sync.json') if json.loads(p.read_text())['pending'])
                sha = bytes(json.loads(rec_file.read_text())['pending']).hex()
                cached = next(rec_file.parent.glob(sha + '*'))
                assert Document(cached).paragraphs[1].text.endswith(marker)
                gate.with_suffix('.drop').unlink(missing_ok=True)
                app.page.locator('.rail-servers').click()
                expect(app.page.locator('.connection-state')).to_have_text('SSH disconnected')
                app.connect()
                expect(activity()).to_contain_text('Saved to server.', timeout=20000)
                assert source.read_bytes() == cached.read_bytes()
                assert json.loads(rec_file.read_text())['pending'] is None
                expect(app.page.locator('.rail-activity .sync-count')).to_have_count(0)
                app.page.locator('.rail-servers').click()
                host = open_file(app, source.name)
                expect(content(app, '.reader-paper').locator('.reader-paper')).to_contain_text(marker)

            baseline, stamp = source.read_bytes(), source.stat()
            save = edit(app, host, ' pending edit')
            # Change only a ZIP timestamp byte, leaving size, visible text and SFTP mtime identical.
            external = bytearray(baseline)
            external[10] ^= 1
            earlier_parts = set(peer.root.glob('.arcnix-*.part'))
            def concurrent_edit():
                deadline = time.monotonic() + 10
                while not set(peer.root.glob('.arcnix-*.part')) - earlier_parts:
                    if time.monotonic() > deadline:
                        raise AssertionError('Upload did not reach the remote staging file')
                    time.sleep(.002)
                source.write_bytes(external)
                os.utime(source, ns=(stamp.st_atime_ns, stamp.st_mtime_ns))

            with ThreadPoolExecutor(max_workers=1) as pool:
                changed = pool.submit(concurrent_edit)
                save.click()
                changed.result()
            expect(host.locator('.office-message')).to_contain_text('server file changed')
            expect(host.get_by_role('button', name='Retry upload')).to_be_visible()
            assert source.read_bytes() == external
            records = list(cache.rglob('sync.json'))
            assert len(records) == 3
            pending = next(p for p in records if json.loads(p.read_text())['pending'])
            def pending_copy():
                rec = json.loads(pending.read_text())
                return next(pending.parent.glob(bytes(rec['pending']).hex() + '*'))
            saved = pending_copy()
            assert Document(saved).paragraphs[1].text.endswith(' pending edit')

            # Two pending documents on restart. One can sync while the other still conflicts.
            host.locator('.window-close').click()
            expect(app.page.locator('.desktop-window.app-office')).to_have_count(0)
            app.show_desktop()
            shortcut = app.page.locator('.desktop-shortcuts').get_by_role('button', name=second.name, exact=True)
            shortcut.dblclick()
            host = app.page.locator('.desktop-window.active.app-office')
            save = edit(app, host, ' from desktop')
            gate.with_suffix('.started').unlink(missing_ok=True)
            gate.touch()
            try:
                save.click()
                eventually(gate.with_suffix('.started').exists)
                expect(shortcut).to_be_disabled()
                expect(shortcut.locator('.file-upload-badge')).to_be_visible()
                expect(host.locator('.office-tabs .window-tab-item.active .file-upload-badge')).to_be_visible()
            finally:
                gate.unlink(missing_ok=True)
            expect(shortcut).to_be_enabled()
            expect(shortcut.locator('.file-upload-badge')).to_have_count(0)
            expect(host.locator('.office-message')).to_have_text('Saved to server.')
            save = edit(app, host, ' recovered after restart')
            original, times = second.read_bytes(), second.stat()
            second.unlink()
            save.click()
            expect(host.get_by_role('button', name='Retry upload')).to_be_visible()
            second.write_bytes(original)
            os.utime(second, ns=(times.st_atime_ns, times.st_mtime_ns))
            expect(app.page.locator('.rail-activity .sync-count')).to_have_text('2')

        # The recovery list still works when the remote path has disappeared.
        source.unlink()
        with Desktop(out / 'desktop', config) as app:
            expect(app.page.locator('.rail-activity .sync-count')).to_have_text('2')
            app.page.locator('.rail-activity').click()
            expect(app.page.locator('.office-sync-row').filter(has_text='Waiting for connection')).to_have_count(2)
            app.servers()
            app.connect()
            eventually(lambda: Document(second).paragraphs[1].text.endswith(' recovered after restart'))
            expect(app.page.locator('.rail-activity .sync-count')).to_have_text('1')
            app.application('Office')
            host = app.page.locator('.desktop-window.active.app-office')
            host.get_by_role('button', name='Pending uploads').click()
            host.get_by_role('button', name='/' + source.name, exact=True).click()
            expect(host.locator('.office-message')).to_have_text('Local changes are waiting to sync.')
            reader = content(app, '.reader-paper[data-kind="writer"]')
            expect(reader.locator('.reader-paper')).to_contain_text(' pending edit')
            assert not source.exists()
            source.write_bytes(external)
            os.utime(source, ns=(stamp.st_atime_ns, stamp.st_mtime_ns))
            host.get_by_role('button', name='Retry upload').click()
            expect(host.locator('.office-message')).to_contain_text('server file changed')
            assert source.read_bytes() == external
            source.write_bytes(baseline)
            os.utime(source, ns=(stamp.st_atime_ns, stamp.st_mtime_ns))
            host.get_by_role('button', name='Retry upload').click()
            expect(host.locator('.office-message')).to_have_text('Saved to server.')
            assert Document(source).paragraphs[1].text.endswith(' pending edit')
            assert not json.loads(pending.read_text())['pending']

            save = edit(app, host, ' preserved locally')
            removed = source.read_bytes()
            source.unlink()
            save.click()
            expect(host.get_by_role('button', name='Retry upload')).to_be_visible()
            assert not source.exists()
            preserved = pending_copy()
            source.write_bytes(removed)
            host.get_by_role('button', name='Open server version').click()
            expect(host.get_by_text('Preserved local copy', exact=True)).to_be_visible()
            reader = content(app, '.reader-paper')
            expect(reader.locator('.reader-paper')).not_to_contain_text('preserved locally')
            assert Document(preserved).paragraphs[1].text.endswith(' preserved locally')
            expect(host.locator('.office-cache-path')).to_contain_text(preserved.name)
            expect(host.get_by_role('button', name='Retry upload')).to_have_count(0)

            # Keep First pending while Second saves an independent copy at the same remote path.
            before = source.read_bytes()
            save = edit(app, host, ' first profile only')
            source.unlink()
            save.click()
            expect(host.get_by_role('button', name='Retry upload')).to_be_visible()
            source.write_bytes(before)
            host.locator('.window-close').click()
            expect(app.page.locator('.desktop-window.app-office')).to_have_count(0)

            app.connect(index=1)
            host = open_file(app, source.name)
            edit(app, host, ' second profile only').click()
            expect(host.locator('.office-message')).to_have_text('Saved to server.')
            app.page.locator('.rail-activity').click()
            rows = app.page.locator('.office-sync-row').filter(has_text=source.name)
            expect(rows.filter(has_text='First')).to_contain_text('Pending upload')
            expect(rows.filter(has_text='Second')).to_contain_text('Saved to server.')
            expect(app.page.locator('.rail-activity .sync-count')).to_have_text('1')
            assert Document(source).paragraphs[1].text.endswith(' second profile only')
            assert Document(pending_copy()).paragraphs[1].text.endswith(' first profile only')
            eventually(lambda: len(list(cache.glob('*/files/*/sync.json'))) == 5)
            assert len(list(cache.iterdir())) == 2
    print('Remote Office save, disconnect, transport loss, automatic reconnect sync, conflict recovery and multi-profile activity passed.')


if __name__ == '__main__':
    main()
