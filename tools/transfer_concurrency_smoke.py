"""Real desktop file progress and bounded parallel SFTP, using owned fixtures only."""
from pathlib import Path
import hashlib
import json
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


FILE_COUNT = 8
FILE_SIZE = 2 * 1024 * 1024
CHILD_ROW = '.transfer-file-row'
CHILD_STATUS = '.transfer-file-status'
JOB_WRAPPER = '.queue-job'


def files(path):
    """Hashes and empty directory markers make recursive transfer checks exact."""
    return {
        item.relative_to(path).as_posix(): None if item.is_dir() else hashlib.sha256(item.read_bytes()).hexdigest()
        for item in path.rglob('*')
    }


def seed(path):
    (path / 'empty-中文').mkdir(parents=True)
    (path / 'nested-中文').mkdir()
    for index in range(FILE_COUNT):
        directory = path if index < FILE_COUNT // 2 else path / 'nested-中文'
        (directory / f'file-{index:02d}.bin').write_bytes(bytes([index]) * FILE_SIZE)


def main():
    output = ROOT / 'artifacts/transfer-concurrency-ui' / f'run-{time.time_ns()}'
    output.mkdir(parents=True)
    local = output / 'local'
    local.mkdir()
    seed(local / 'upload-tree')
    seed(local / 'cancel-tree')
    (local / 'downloads').mkdir()
    checks = []
    samples = {}

    with Peer(output / 'peer', io_delay=.01) as peer:
        config = peer.config([peer.profile('Transfer concurrency')], local)
        with Desktop(output / 'desktop', config) as app:
            page = app.page

            def start_watcher():
                # The observer records every rendered update rather than relying
                # on two coincidentally visible status labels in a screenshot.
                page.evaluate('''selectors => {
                    window.__transferSamples = [];
                    window.__transferObserver?.disconnect();
                    let last = '';
                    const sample = () => {
                        const rows = [...document.querySelectorAll(selectors.row)];
                        const files = rows.map(row => ({
                            path: row.querySelector(selectors.path)?.textContent || '',
                            status: row.querySelector(selectors.status)?.textContent || '',
                            percent: Number(row.querySelector('[role=progressbar]')?.getAttribute('aria-valuenow') || 0)
                        }));
                        const signature = JSON.stringify(files);
                        if (signature !== last) {
                            last = signature;
                            window.__transferSamples.push({at: performance.now(), files});
                        }
                    };
                    window.__transferObserver = new MutationObserver(sample);
                    window.__transferObserver.observe(document.body, {
                        subtree:true, childList:true, characterData:true,
                        attributes:true, attributeFilter:['aria-valuenow','class']
                    });
                    sample();
                }''', dict(row=CHILD_ROW, status=CHILD_STATUS, path='.transfer-file-path'))

            def job(name):
                return page.locator(JOB_WRAPPER).filter(has=page.locator('.job-name', has_text=name)).last

            def expand(item, wait=True):
                button = item.locator('.transfer-details-toggle')
                if button.get_attribute('aria-expanded') != 'true':
                    button.click()
                if wait:
                    expect(item.locator(CHILD_ROW)).to_have_count(FILE_COUNT)

            def run_folder(name, remote=False, active_screenshot=None):
                start_watcher()
                app.file(name, remote=remote).click()
                # Pre-expand the queued job before worker conflicts can open a
                # modal and make the underlying queue inert.
                page.get_by_role('button', name='Pause queue', exact=True).click()
                previous = page.locator(JOB_WRAPPER).count()
                page.locator('.remote-slot' if remote else '.local-slot').get_by_role(
                    'button', name='Download' if remote else 'Upload', exact=True,
                ).click()
                expect(page.locator(JOB_WRAPPER)).to_have_count(previous + 1)
                item = job(name)
                expect(item).to_be_visible()
                expand(item, wait=False)
                page.get_by_role('button', name='Resume queue', exact=True).click()
                expect(item.locator(CHILD_ROW)).to_have_count(FILE_COUNT)
                if active_screenshot:
                    eventually(lambda: sum(0 < float(value or '0') < 100 for value in item.locator(CHILD_ROW).get_by_role('progressbar').evaluate_all("bars => bars.map(bar => bar.getAttribute('aria-valuenow'))")) >= 2)
                    app.screenshot(active_screenshot)
                return item

            def status(item):
                return item.locator('.job-status')

            def compact_layout(item, width, check_row_height=True):
                page.set_viewport_size(dict(width=width, height=980 if width == 1440 else 800))
                geometry = item.evaluate('''item => {
                    const box = element => {
                        const bounds = element.getBoundingClientRect();
                        return {x:bounds.x,y:bounds.y,width:bounds.width,height:bounds.height};
                    };
                    const header = item.querySelector('.queue-row');
                    const parentBar = header.querySelector('.job-progress-column > .job-progress');
                    const text = header.querySelector('.job-transfer');
                    const rect = parentBar.getBoundingClientRect();
                    const content = text.getBoundingClientRect();
                    return {
                        parent:box(parentBar), text:box(text),
                        nestedInText:text.contains(parentBar),
                        overlapsText:rect.left < content.right && rect.right > content.left && rect.top < content.bottom && rect.bottom > content.top,
                        files:[...item.querySelectorAll('.transfer-file-row')].map(row => ({
                            row:box(row),bar:box(row.querySelector('.transfer-file-progress'))
                        })),
                        horizontalOverflow:document.documentElement.scrollWidth > innerWidth
                    };
                }''')
                assert not geometry['nestedInText'] and not geometry['overlapsText'], geometry
                assert 0 < geometry['parent']['width'] <= 140, geometry
                assert not geometry['horizontalOverflow'], geometry
                for file in geometry['files']:
                    if check_row_height:
                        assert file['row']['height'] <= 32, file
                    assert 0 < file['bar']['width'] <= 140, file
                app.screenshot(f'compact-{width}.png')
                return geometry

            def finish(item, name, limit):
                expect(status(item)).to_have_text('Completed', timeout=60000)
                expect(item.locator(CHILD_STATUS)).to_have_text(['Completed'] * FILE_COUNT)
                expect(item.locator(CHILD_ROW).get_by_role('progressbar')).to_have_count(FILE_COUNT)
                values = item.locator(CHILD_ROW).get_by_role('progressbar').evaluate_all(
                    "bars => bars.map(bar => bar.getAttribute('aria-valuenow'))",
                )
                assert values == ['100'] * FILE_COUNT, values
                observed = page.evaluate('window.__transferSamples')
                assert observed, 'No child progress updates were rendered'
                counts = [sum(file['status'] == 'Transferring' for file in sample['files']) for sample in observed]
                maximum = max(counts)
                assert maximum == limit, dict(name=name, maximum=maximum, limit=limit)
                advancing = max(sum(file['status'] == 'Transferring' and 0 < file['percent'] < 100 for file in sample['files']) for sample in observed)
                assert advancing >= 2, dict(name=name, concurrently_advancing_files=advancing)
                samples[name] = dict(maximum_running=maximum, simultaneously_advancing=advancing, samples=observed)
                app.screenshot(name + '.png')

            app.settings('Transfers')
            field = page.get_by_role('textbox', name='Maximum simultaneous files', exact=True)
            expect(field).to_have_value('5')
            field.fill('0')
            page.get_by_role('button', name='Apply transfer limit', exact=True).click()
            expect(page.locator('.settings-error')).to_contain_text('between 1 and 16')
            assert json.loads(app.config.read_text())['config']['settings'].get('transfer_concurrency', 5) == 5
            field.fill('5')
            page.get_by_role('button', name='Apply transfer limit', exact=True).click()
            expect(page.locator('.settings-error')).not_to_be_visible()
            checks.append('Legacy settings default to five simultaneous files; invalid zero is rejected without overwriting the saved limit')

            app.servers()
            app.connect()
            uploading = run_folder('upload-tree', active_screenshot='upload-running.png')
            finish(uploading, 'upload-default-five', 5)
            assert files(peer.root / 'upload-tree') == files(local / 'upload-tree')
            checks.append('One folder exposes every file and individual progress, transfers at least two files concurrently, never exceeds the default five, and preserves Unicode empty directories and exact contents')

            app.settings('Transfers')
            field.fill('2')
            page.get_by_role('button', name='Apply transfer limit', exact=True).click()
            expect(field).to_have_value('2')
            eventually(lambda: json.loads(app.config.read_text())['config']['settings']['transfer_concurrency'] == 2)
            app.screenshot('settings-limit-two.png')
            app.stop()
            app.launch()
            page = app.page
            app.settings('Transfers')
            expect(page.get_by_role('textbox', name='Maximum simultaneous files', exact=True)).to_have_value('2')
            app.servers()
            app.connect()
            app.file('downloads', remote=False).dblclick()
            downloading = run_folder('upload-tree', remote=True, active_screenshot='download-running.png')
            finish(downloading, 'download-limit-two', 2)
            assert files(local / 'downloads' / 'upload-tree') == files(local / 'upload-tree')
            checks.append('Changing the limit to two persists across a desktop restart; folder downloads advance in parallel while respecting the new bound and matching every file hash')
            samples['layout'] = {
                '1440':compact_layout(downloading, 1440),
                '1000':compact_layout(downloading, 1000),
            }
            page.set_viewport_size(dict(width=1440, height=980))
            checks.append('Expanded file rows stay at most 32 pixels high; parent and child bars are at most 140 pixels wide and separate from text, without horizontal overflow at 1440 or 1000 pixels')

            conflict = run_folder('upload-tree', remote=True)
            dialog = page.get_by_role('dialog')
            expect(dialog).to_be_visible()
            paths = set()
            for index in range(FILE_COUNT):
                expect(dialog).to_be_visible()
                incoming = dialog.locator('.conflict-path').first.inner_text()
                assert incoming not in paths, incoming
                paths.add(incoming)
                dialog.get_by_role('button', name='Skip file', exact=True).click()
                eventually(lambda: sum('Skipped' in value for value in conflict.locator(CHILD_STATUS).all_text_contents()) >= index + 1)
            expect(status(conflict)).to_contain_text('Skipped', timeout=30000)
            assert len(paths) == FILE_COUNT
            assert files(local / 'downloads' / 'upload-tree') == files(local / 'upload-tree')
            checks.append('Every simultaneous file conflict receives its own decision; skipping all files preserves all destination bytes')

            page.locator('.local-slot').get_by_role('button', name='Local parent directory', exact=True).click()
            canceling = run_folder('cancel-tree')
            eventually(lambda: any(0 < float(value or '0') < 100 for value in canceling.locator(CHILD_ROW).get_by_role('progressbar').evaluate_all("bars => bars.map(bar => bar.getAttribute('aria-valuenow'))")))
            canceling.get_by_role('button', name='Cancel transfer', exact=True).click()
            expect(status(canceling)).to_have_text('Canceled', timeout=20000)
            eventually(lambda: all(value not in ['Queued', 'Transferring', 'Waiting for a file conflict decision'] for value in canceling.locator(CHILD_STATUS).all_text_contents()))
            frozen = canceling.locator(CHILD_ROW).all_text_contents()
            page.wait_for_timeout(500)
            assert canceling.locator(CHILD_ROW).all_text_contents() == frozen
            app.screenshot('canceled-files.png')
            checks.append('Canceling a folder terminates every active or queued child and retains stable per-file progress')

            canceling.get_by_role('button', name='Retry transfer', exact=True).click()
            retry = job('cancel-tree')
            expand(retry)
            expect(status(retry)).to_have_text('Completed', timeout=60000)
            expect(retry.locator(CHILD_STATUS)).to_have_text(['Completed'] * FILE_COUNT)
            assert files(peer.root / 'cancel-tree') == files(local / 'cancel-tree')
            checks.append('Retry creates a fresh file progress attempt and completes the full canceled folder with matching contents')

            seed(local / 'disconnect-tree')
            page.locator('.local-slot').get_by_role('button', name='Refresh local directory', exact=True).click()
            expect(app.file('disconnect-tree', remote=False)).to_be_visible()
            disconnecting = run_folder('disconnect-tree', active_screenshot='before-disconnect.png')
            page.get_by_role('button', name='Disconnect', exact=True).click()
            expect(page.locator('.overview-connect')).to_be_visible()
            app.connect()
            disconnected = job('disconnect-tree')
            expect(status(disconnected)).to_have_text('Canceled')
            expand(disconnected)
            states = disconnected.locator(CHILD_STATUS).all_text_contents()
            assert all(state not in ['Queued', 'Transferring', 'Waiting for a file conflict decision'] for state in states), states
            frozen = disconnected.locator(CHILD_ROW).all_text_contents()
            page.wait_for_timeout(500)
            assert disconnected.locator(CHILD_ROW).all_text_contents() == frozen
            app.screenshot('disconnected-files-terminal.png')
            checks.append('Disconnecting an active folder makes every child terminal; reconnecting retains stable canceled progress without accepting stale worker updates')

    result = dict(passed=len(checks), checks=checks, output=str(output), concurrency={key: {name: value for name, value in evidence.items() if name != 'samples'} for key, evidence in samples.items() if key != 'layout'})
    (output / 'results.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
    (output / 'progress-samples.json').write_text(json.dumps(samples, indent=2), encoding='utf-8')
    print(json.dumps(result), flush=True)


if __name__ == '__main__':
    main()
