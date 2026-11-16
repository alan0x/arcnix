"""Native cross-pane drag/drop in an isolated WebView and loopback SFTP peer."""
from pathlib import Path
import json
import os
import socket
import subprocess
import sys
import time
import urllib.request
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'artifacts/python'))
from playwright.sync_api import sync_playwright, expect


def tree(path):
    """Include empty directories and file bytes in the transfer evidence."""
    return {
        child.relative_to(path).as_posix(): None if child.is_dir() else child.read_bytes()
        for child in path.rglob('*')
    }


def main():
    out = ROOT / 'artifacts/file-drag-drop-ui'
    out.mkdir(parents=True, exist_ok=True)
    ready = out / f'fixture-{time.time_ns()}.json'
    log = (out / 'desktop.log').open('w', encoding='utf-8')
    fixture = subprocess.Popen(
        [sys.executable, str(ROOT / 'tools/sftp_fixture.py'), '--ready', str(ready)],
        cwd=ROOT, stdout=log, stderr=log,
    )
    app = None
    checks = []
    try:
        deadline = time.monotonic() + 20
        while not ready.exists():
            assert fixture.poll() is None and time.monotonic() < deadline
            time.sleep(.1)
        peer = json.loads(ready.read_text())
        host, port = peer['address'].rsplit(':', 1)
        assert host == '127.0.0.1'
        local = out / f'local-{time.time_ns()}'
        local.mkdir()
        (local / 'upload-folder' / 'empty').mkdir(parents=True)
        (local / 'upload-folder' / 'nested-中文.txt').write_bytes(b'nested folder payload')
        (local / 'multi.txt').write_bytes(b'multiselected file payload')
        (local / 'unselected.txt').write_bytes(b'unselected row payload')
        (local / 'download-target').mkdir()
        remote = Path(peer['root']) / 'drop-target'
        remote.mkdir()
        original_tree = tree(local / 'upload-folder')
        profile = dict(
            id='drag-drop', name='Drag and drop', host=host, port=int(port),
            username='arcnix', auth='PrivateKey', private_key=peer['client_key_ed25519'],
            initial_remote_path='/drop-target',
        )
        config = out / 'config.json'
        config.write_text(json.dumps(dict(version=1, config=dict(
            servers=[profile, dict(profile, id='no-sftp', name='No SFTP', username='arcnix-nosftp')],
            groups=[], trusted_hosts=[dict(host=host, port=int(port), fingerprint=peer['host_key_rsa_fingerprint'])],
            local_directory=str(local), settings=dict(language='English', theme='Light', show_hidden_files=True, resource_interval_seconds=3),
        ))))
        metadata = json.loads(subprocess.check_output(
            ['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=ROOT,
        ))
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            debug = listener.getsockname()[1]
        endpoint = f'http://127.0.0.1:{debug}'
        env = os.environ.copy()
        env['WEBVIEW2_USER_DATA_FOLDER'] = str(out / f'webview-{time.time_ns()}')
        env['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'] = f'--remote-debugging-port={debug} --remote-debugging-address=127.0.0.1'
        app = subprocess.Popen(
            [str(Path(metadata['target_directory']) / 'debug/arcnix.exe'), '--config', str(config)],
            cwd=ROOT, env=env, stdout=log, stderr=log,
        )
        deadline = time.monotonic() + 20
        while True:
            try:
                urllib.request.urlopen(endpoint + '/json/version', timeout=1).close()
                break
            except OSError:
                assert app.poll() is None and time.monotonic() < deadline
                time.sleep(.1)
        with sync_playwright() as pw:
            browser = pw.chromium.connect_over_cdp(endpoint)
            page = browser.contexts[0].pages[0]
            page.set_default_timeout(10000)
            page.set_viewport_size(dict(width=1440, height=980))
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            left, right = page.locator('.local-slot'), page.locator('.remote-slot')
            jobs = page.locator('.queue-row')

            def file(pane, name):
                return pane.get_by_role('button', name=name, exact=True)

            def disclosure(button, expanded):
                expect(button).to_have_text('')
                expect(button).to_have_attribute('aria-expanded', str(expanded).lower())
                icon = button.locator('.disclosure-icon')
                expect(icon).to_have_count(1)
                mask = icon.evaluate('node => getComputedStyle(node).maskImage')
                assert 'data:image/svg+xml' in mask, mask
                svg = unquote(mask)
                assert '<svg' in svg and 'http://www.w3.org/2000/svg' in svg and '<path' in svg, svg
                assert 'stroke-linecap="round"' in svg and 'stroke-width="2"' in svg, svg
                button_box, icon_box = button.bounding_box(), icon.bounding_box()
                assert button_box and icon_box
                for dimension in ['width', 'height']:
                    assert abs(icon_box[dimension] - 16) <= .5, icon_box
                for axis, dimension in [('x', 'width'), ('y', 'height')]:
                    center = button_box[axis] + button_box[dimension] / 2
                    icon_center = icon_box[axis] + icon_box[dimension] / 2
                    assert abs(center - icon_center) <= .75, (button_box, icon_box)
                return svg

            def blank(pane):
                empty = pane.locator('.file-empty')
                if empty.is_visible():
                    return empty
                return pane.locator('.file-list-container')

            def drag(source, target, hint=None, screenshot=None, source_selected=None):
                box = target.bounding_box()
                assert box and box['width'] > 0 and box['height'] > 0
                if hint is None:
                    source.drag_to(target, target_position=dict(x=box['width'] - 24, y=box['height'] - 12))
                    return
                origin = source.bounding_box()
                x, y = origin['x'] + 20, origin['y'] + origin['height'] / 2
                page.mouse.move(x, y)
                page.mouse.down()
                try:
                    page.mouse.move(x + 16, y, steps=6)
                    page.mouse.move(box['x'] + box['width'] - 24, box['y'] + box['height'] - 12, steps=15)
                    active = page.locator('[data-file-drop-target] > .file-drop-hint')
                    expect(active).to_have_text(hint)
                    expect(active).to_be_visible()
                    if source_selected is not None:
                        expect(page.locator('[data-file-drag-source] .row-selected')).to_have_count(source_selected)
                    page.screenshot(path=str(out / screenshot))
                finally:
                    page.mouse.up()
                expect(page.locator('[data-file-drop-target],[data-file-drag-source]')).to_have_count(0)

            def complete(count):
                expect(jobs).to_have_count(count)
                for index in range(count):
                    expect(jobs.nth(index).locator('.job-status')).to_have_text('Completed', timeout=20000)

            def connect():
                page.locator('.overview-connect').click()
                page.get_by_role('dialog', name='Connect to server', exact=True).get_by_role('button', name='Connect', exact=True).click()
                expect(page.locator('.connection-state')).to_have_text('SSH connected')

            def gesture(pane, kind, drag_id, row=None):
                # Exercise the Rust session guard without native drag consuming
                # pointer events needed to change the listing in the meantime.
                path = row.locator('xpath=ancestor::*[contains(concat(" ", normalize-space(@class), " "), " file-row ")]').get_attribute('aria-label') if row else ''
                pane.locator('.file-gesture-input').evaluate('''(bridge, gesture) => {
                    bridge.value = JSON.stringify(gesture);
                    bridge.dispatchEvent(new Event('input', {bubbles:true}));
                }''', dict(kind=kind, drag_id=drag_id, path=path))

            try:
                page.locator('.host-select').first.click()
                expect(page.locator('.file-pane')).to_have_count(0)
                expect(jobs).to_have_count(0)
                assert not list(remote.iterdir())
                checks.append('A disconnected workspace exposes no file panes or drag destination')
                connect()
                expect(right.locator('.file-empty')).to_have_text('This folder is empty')

                file(left, 'upload-folder').click()
                expander = left.get_by_role('button', name='Expand upload-folder', exact=True)
                closed_path = disclosure(expander, False)
                expander.click()
                expect(file(left, 'nested-中文.txt')).to_be_visible()
                open_path = disclosure(left.get_by_role('button', name='Collapse upload-folder', exact=True), True)
                assert closed_path and open_path and closed_path != open_path
                page.screenshot(path=str(out / 'arrow-expanded.png'))
                file(left, 'upload-folder').focus()
                page.keyboard.press('ArrowLeft')
                expect(file(left, 'nested-中文.txt')).to_have_count(0)
                disclosure(left.get_by_role('button', name='Expand upload-folder', exact=True), False)
                page.keyboard.press('ArrowRight')
                expect(file(left, 'nested-中文.txt')).to_be_visible()
                disclosure(left.get_by_role('button', name='Collapse upload-folder', exact=True), True)
                checks.append('File disclosures use centered 16-pixel SVG masks with reactive expanded semantics; keyboard left/right collapses and expands without font glyph checks')
                file(left, 'nested-中文.txt').click(modifiers=['Control'])
                file(left, 'multi.txt').click(modifiers=['Control'])
                expect(left.locator('.row-selected')).to_have_count(3)
                drag(file(left, 'upload-folder'), blank(right), 'Release to upload to the current remote folder', 'upload-drop-highlight.png')
                complete(2)
                expect(file(right, 'upload-folder')).to_be_visible()
                assert tree(remote / 'upload-folder') == original_tree
                assert (remote / 'multi.txt').read_bytes() == (local / 'multi.txt').read_bytes()
                assert tree(local / 'upload-folder') == original_tree
                checks.append('A file and folder upload together into an empty pane, including Unicode and empty directories; selecting a parent and child queues the tree once and preserves sources')

                folder_job = page.locator('.queue-job').filter(has=page.locator('.job-name', has_text='upload-folder')).last
                queue_toggle = folder_job.locator('.transfer-details-toggle')
                disclosure(queue_toggle, False)
                queue_toggle.click()
                expect(folder_job.locator('.transfer-file-row')).to_have_count(1)
                disclosure(queue_toggle, True)
                page.screenshot(path=str(out / 'queue-arrow-expanded.png'))
                queue_toggle.click()
                expect(folder_job.locator('.transfer-file-row')).to_have_count(0)
                disclosure(queue_toggle, False)
                checks.append('Transfer queue disclosures share the same centered SVG geometry and reactive expanded semantics while opening and closing file progress')

                file(left, 'upload-folder').click()
                file(left, 'multi.txt').click(modifiers=['Control'])
                expect(left.locator('.row-selected')).to_have_count(2)
                drag(file(left, 'unselected.txt'), blank(right), 'Release to upload to the current remote folder', 'unselected-drop-highlight.png', source_selected=1)
                complete(3)
                expect(file(right, 'unselected.txt')).to_be_visible()
                assert (remote / 'unselected.txt').read_bytes() == (local / 'unselected.txt').read_bytes()
                checks.append('Dragging an unselected row transfers only that row instead of the previous multiselection')

                file(left, 'download-target').dblclick()
                expect(left.locator('.file-empty')).to_have_text('This folder is empty')
                left.get_by_role('button', name='Icon grid', exact=True).click()
                right.get_by_role('button', name='Icon grid', exact=True).click()
                file(right, 'upload-folder').click()
                file(right, 'multi.txt').click(modifiers=['Control'])
                drag(file(right, 'upload-folder'), blank(left), 'Release to download to the current local folder', 'download-drop-highlight.png')
                complete(5)
                downloaded = local / 'download-target'
                assert tree(downloaded / 'upload-folder') == original_tree
                assert (downloaded / 'multi.txt').read_bytes() == (local / 'multi.txt').read_bytes()
                page.screenshot(path=str(out / 'download-grid.png'))
                checks.append('Icon-grid multiselection downloads files and recursive folders into the opposite current directory with identical contents')

                drag(file(right, 'unselected.txt'), file(left, 'upload-folder'))
                complete(6)
                assert (downloaded / 'unselected.txt').read_bytes() == (local / 'unselected.txt').read_bytes()
                assert not (downloaded / 'upload-folder' / 'unselected.txt').exists()
                checks.append('Dropping onto an opposite folder row transfers to the displayed directory')

                drag(file(right, 'unselected.txt'), blank(left))
                expect(jobs).to_have_count(7)
                dialog = page.get_by_role('dialog')
                expect(dialog).to_be_visible()
                expect(jobs.last.locator('.job-status')).to_have_text('Waiting for a file conflict decision')
                dialog.get_by_role('button', name='Skip file', exact=True).click()
                expect(jobs.last.locator('.job-status')).to_contain_text('Skipped')
                assert (downloaded / 'unselected.txt').read_bytes() == (local / 'unselected.txt').read_bytes()
                checks.append('Dropped transfers use the existing conflict decision and Skip leaves destination bytes intact')

                drag(file(right, 'multi.txt'), file(right, 'upload-folder'))
                page.wait_for_timeout(300)
                expect(jobs).to_have_count(7)
                source = file(right, 'multi.txt').bounding_box()
                target = blank(left).bounding_box()
                page.mouse.move(source['x'] + 20, source['y'] + source['height'] / 2)
                page.mouse.down()
                page.mouse.move(source['x'] - 20, source['y'] + source['height'] / 2, steps=8)
                page.mouse.move(target['x'] + target['width'] / 2, target['y'] + target['height'] / 2, steps=15)
                page.keyboard.press('Escape')
                page.mouse.up()
                page.wait_for_timeout(300)
                expect(jobs).to_have_count(7)
                checks.append('Same-pane drops and Escape-canceled native drags add no queue entries')

                gesture(right, 'drag_start', 8_000_001, file(right, 'multi.txt'))
                left.get_by_role('button', name='Refresh local directory', exact=True).click()
                page.wait_for_timeout(200)
                expect(left.locator('.file-transfer-ready')).to_have_count(1)
                gesture(left, 'transfer_drop', 8_000_001)
                page.wait_for_timeout(300)
                expect(jobs).to_have_count(7)
                expect(page.get_by_role('dialog')).to_have_count(0)
                checks.append('Refreshing a destination invalidates a previously frozen drag, even after the same directory is ready again')

                (downloaded / 'multi.txt').unlink()
                file(right, 'multi.txt').click()
                gesture(right, 'drag_start', 8_000_002, file(right, 'multi.txt'))
                file(right, 'unselected.txt').click()
                gesture(left, 'transfer_drop', 8_000_002)
                expect(jobs).to_have_count(8)
                expect(jobs.last.locator('.job-status')).to_have_text('Completed', timeout=20000)
                assert (downloaded / 'multi.txt').read_bytes() == (local / 'multi.txt').read_bytes()
                gesture(left, 'transfer_drop', 8_000_002)
                page.wait_for_timeout(300)
                expect(jobs).to_have_count(8)
                checks.append('A drag keeps its original source selection when selection changes, and replaying its drop token cannot queue another transfer')

                page.locator('.host-select').nth(1).click()
                connect()
                expect(right.locator('.pane-summary').first).to_contain_text('Unavailable')
                count = jobs.count()
                drag(file(left, 'multi.txt'), blank(right))
                page.wait_for_timeout(300)
                expect(jobs).to_have_count(count)
                checks.append('SSH without SFTP accepts no dropped transfer')
                assert not errors, errors
                result = dict(passed=len(checks), checks=checks)
                (out / 'results.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
                print(json.dumps(result), flush=True)
            except Exception:
                try:
                    page.screenshot(path=str(out / 'failure.png'))
                    (out / 'failure.txt').write_text(page.locator('body').inner_text(), encoding='utf-8')
                except Exception:
                    pass
                raise
            finally:
                browser.close()
    finally:
        if app and app.poll() is None:
            app.terminate()
            app.wait(timeout=10)
        fixture.terminate()
        fixture.wait(timeout=10)
        log.close()


if __name__ == '__main__':
    main()
