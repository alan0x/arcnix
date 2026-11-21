"""Exercise SFTP range searching and bounded result navigation in the real workbench."""
import os
import re
import time
from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


BLOCK = 128 * 1024
PAGE = BLOCK - 4


def main():
    out = ROOT / 'artifacts/code-large-search' / str(time.time_ns())
    out.mkdir(parents=True)
    with Peer(out / 'peer', io_delay=0.002) as peer:
        root = peer.root / 'project'
        root.mkdir()
        (root / 'quick-open.txt').write_text('Open from title bar\nResult selected\n', 'utf8')
        log = root / 'server.log'
        endings = (b'\r\n', b'\r', b'\n')
        with log.open('wb') as f:
            # The range ends inside the second UTF-8 character of the match.
            f.write(b'\xef\xbb\xbf' + b'x' * (PAGE - 7))
            f.write('命中_边界 A\r\n'.encode())
            for line in range(2, 120002):
                f.write(f'{line:07d} system: periodic inspection complete'.encode() + endings[line % 3])
            target_offset = f.tell()
            f.write('🚦 命中_末尾 B\r\n'.encode())
            f.write(b'after the match\r\n')
        # A match in a single minified line which exceeds the editing threshold.
        long_file = root / 'min.js'
        with long_file.open('wb') as f:
            for _ in range(12):
                f.write(b'a' * BLOCK)
            f.write(b'const needle=123456;')
        profile = peer.profile('Large log')
        profile['initial_remote_path'] = '/project'
        with Desktop(out / 'app', peer.config([profile], out)) as app:
            app.connect()
            app.application('Code Editor')
            page = app.code()
            errors = []
            page.on('pageerror', lambda e: errors.append(e.stack or str(e)))
            expect(page.get_by_role('treeitem', name='server.log', exact=True)).to_be_visible()
            page.evaluate("""() => {
                window.rangeMetrics = {count:0, max:0, total:0, ends:[], full:0, summaries:0};
                const held = [];
                let hold = true;
                const original=window.arcnixReply;
                window.arcnixReply=replies=>{
                    for (const {value} of replies) {
                        if (value?.digest && typeof value.text === 'string') {
                            rangeMetrics.count++;
                            rangeMetrics.max=Math.max(rangeMetrics.max,value.text.length);
                            rangeMetrics.total+=value.text.length;
                            rangeMetrics.ends.push(value.next);
                        }
                        if(value?.content?.length > 1024*1024) rangeMetrics.full++;
                    }
                    original(replies.filter(reply => {
                        const data = reply.value;
                        if (!data || typeof data.breaks !== 'number' || 'text' in data) return true;
                        if (Object.keys(data).sort().join() !== 'breaks,next') throw Error('Unexpected index payload');
                        rangeMetrics.summaries++;
                        if (hold) held.push(reply);
                        return !hold;
                    }));
                };
                window.holdIndex = () => { hold = true; };
                window.releaseIndex = () => { hold = false; original(held.splice(0)); };
            }""")
            toolbar = app.page.locator('.app-code .desktop-window-titlebar')
            expect(toolbar.get_by_role('button', name='Quick open')).to_have_text('project')
            expect(page.get_by_role('menubar')).to_be_visible()
            slot = toolbar.get_by_role('button', name='Quick open')
            picker = page.locator('.quick-input-widget input:visible').first
            def check_title_input():
                expect(picker).to_be_focused()
                slot_size = slot.bounding_box()
                title_size = toolbar.bounding_box()
                assert abs(slot_size['x'] + slot_size['width'] / 2 - title_size['x'] - title_size['width'] / 2) < 2
                box = picker.bounding_box()
                inside = picker.evaluate("""input => {
                    const r = input.getBoundingClientRect();
                    return [document.documentElement.className, document.elementFromPoint(r.x + 15, r.y + r.height / 2) === input,
                        getComputedStyle(document.documentElement).getPropertyValue('--command-left'),
                        document.querySelector('.part.titlebar').getBoundingClientRect().bottom];
                }""")
                eventually(lambda: page.locator('.part.statusbar').evaluate('el => Math.abs(el.getBoundingClientRect().bottom - innerHeight) < 2'))
                assert inside[1] and 'title-picker' in inside[0], (inside, box, slot_size)
                assert abs(box['x'] - float(inside[2][:-2])) < 12, (inside, box)
                assert abs(box['y'] + box['height'] / 2 - slot_size['height'] / 2) < 4, (inside, box, slot_size)
                assert box['y'] + box['height'] <= inside[3] + 2, (inside, box)
            slot.click()
            check_title_input()
            empty_panel = page.locator('.quick-input-widget')
            expect(empty_panel.locator('.monaco-list-row')).to_have_count(0)
            eventually(lambda: empty_panel.bounding_box()['height'] <= slot.bounding_box()['height'] + 1)
            picker.press('Escape')
            expect(picker).to_have_count(0)
            maximize = app.page.locator('.app-code .window-maximize')
            maximize.click()
            expect(maximize).to_have_attribute('aria-pressed', 'true')
            slot.focus()
            slot.press('Control+p')
            check_title_input()
            picker.press('Escape')
            expect(picker).to_have_count(0)
            maximize.click()
            expect(maximize).to_have_attribute('aria-pressed', 'false')
            page.keyboard.press('Control+p')
            check_title_input()
            picker.fill('quick-open.txt')
            expect(page.locator('.quick-input-list .monaco-list-row').filter(has_text='quick-open.txt').first).to_be_visible()
            picker.press('Enter')
            expect(page.locator('.tab.active')).to_contain_text('quick-open.txt')
            expect(page.locator('.view-line').filter(has_text='Result selected').first).to_be_visible()
            expect(picker).to_have_count(0)
            page.evaluate("arcnixCommand('workbench.action.closeActiveEditor')")
            expect(page.locator('.tab').filter(has_text='quick-open.txt')).to_have_count(0)
            toolbar.get_by_role('button', name='Toggle sidebar').click()
            expect(page.locator('.explorer-viewlet')).not_to_be_visible()
            toolbar.get_by_role('button', name='Toggle sidebar').click()
            tree_file = page.get_by_role('treeitem', name='server.log', exact=True)
            tree_file.dblclick()
            viewer = page.locator('.arcnix-paged-file')
            scroller = viewer.locator('.paged-lines')
            expect(viewer.locator('.paged-row[data-line="1"]').first).to_be_visible()
            assert page.evaluate('rangeMetrics.ends') == [PAGE], page.evaluate('rangeMetrics')
            # A seek to EOF skips every intervening page. Going home reads the first page again.
            scroller.focus()
            scroller.press('Control+End')
            expect(viewer).to_contain_text('after the match')
            assert page.evaluate('rangeMetrics.count') <= 3
            expect(viewer.locator('header')).to_contain_text('Indexing line numbers')
            eventually(lambda: page.evaluate('rangeMetrics.summaries') == 1)
            scroller.press('Control+Home')
            expect(viewer.locator('.paged-row[data-line="1"]').first).to_be_visible()
            assert page.evaluate('rangeMetrics.count') <= 5
            # Seek deep into the file with no line index and verify the actual remote records.
            before_seek = page.evaluate('rangeMetrics.count')
            scroller.evaluate('el => el.scrollTop = (el.scrollHeight - el.clientHeight) / 2')
            middle_records = viewer.locator('.paged-row').filter(has_text=re.compile(r'00[5-7]\d{4} system:'))
            expect(middle_records.first).to_be_visible()
            assert page.evaluate('rangeMetrics.summaries') == 1
            scroll_top = scroller.evaluate('el => el.scrollTop')
            page.evaluate('releaseIndex()')
            expect(viewer.locator('header')).to_have_text(re.compile(r'^Line \d+ · Read-only$'), timeout=40000)
            rows = middle_records.evaluate_all('rows => rows.map(row => [Number(row.dataset.line), parseInt(row.querySelector("code").textContent)])')
            assert rows and all(line == record for line, record in rows), rows
            assert abs(scroller.evaluate('el => el.scrollTop') - scroll_top) < 1
            middle_line = rows[0][0]
            scans = page.evaluate('rangeMetrics.summaries')
            assert 1 < scans < log.stat().st_size // PAGE, scans
            assert page.evaluate('rangeMetrics.count') - before_seek <= 2
            # A random jump inside the indexed prefix renders exact labels immediately.
            scroller.evaluate('el => el.scrollTop = el.scrollHeight * 0.18')
            eventually(lambda: page.evaluate('rangeMetrics.count') > before_seek + 1)
            warm = viewer.locator('.paged-row[data-line] code').filter(has_text=re.compile(r'^\d{7} system:'))
            expect(warm.first).to_be_visible()
            assert warm.first.evaluate('el => +el.parentNode.dataset.line === parseInt(el.textContent)')
            assert int(warm.first.inner_text()[:7]) < middle_line
            assert page.evaluate('rangeMetrics.summaries') == scans
            page.get_by_role('tab', name=re.compile('^Search')).click()
            box = page.locator('.search-view .search-widget textarea').first
            results = page.locator('.search-view .results .monaco-list-row')
            box.fill('命中_')
            box.press('Enter')
            tail = results.filter(has_text='末尾')
            expect(tail).to_be_visible(timeout=90000)
            expect(page.locator('.search-view')).to_contain_text('2 results in 1 file')
            with log.open('ab') as f:
                f.write('命中_新增\n'.encode())
            before = page.evaluate('rangeMetrics.count')
            tail.click()
            viewer = page.locator('.arcnix-paged-file')
            hit = viewer.locator('.paged-row[data-line="120002"] mark')
            expect(hit).to_have_text('命中_',timeout=15000)
            expect(viewer).to_contain_text('after the match')
            expect(results.filter(has_text='新增')).to_have_count(0)
            assert 1 <= page.evaluate('rangeMetrics.count') - before <= 3
            # The search already visited every page before its tail match.
            # Open an unvisited part of that prefix without an index-only read.
            scroller.evaluate('el => el.scrollTop = el.scrollHeight * 0.75')
            expect(hit).to_have_count(0)
            labelled = viewer.locator('.paged-row[data-line] code').filter(has_text=re.compile(r'^\d{7} system:'))
            expect(labelled.first).to_be_visible()
            assert labelled.first.evaluate('el => +el.parentNode.dataset.line === parseInt(el.textContent)')
            assert page.evaluate('rangeMetrics.summaries') == scans
            tail.click()
            expect(hit).to_have_text('命中_')
            scroller.focus()
            scroller.hover()
            page.mouse.wheel(0, -1000)
            expect(viewer.locator('.paged-row[data-line="120002"] mark')).to_have_count(0)
            # Traverse the upper edge of the initial hit window, then return over it.
            before_scroll = page.evaluate('rangeMetrics.count')
            for _ in range(12):
                scroller.evaluate('el => el.scrollTop -= 9000')
                page.wait_for_timeout(35)
            eventually(lambda: page.evaluate('rangeMetrics.count') > before_scroll)
            expect(viewer.locator('.paged-row[data-line]').first).to_be_visible()
            assert int(viewer.locator('.paged-row[data-line]').first.get_attribute('data-line')) < 116000
            for _ in range(12):
                scroller.evaluate('el => el.scrollTop += 9000')
                page.wait_for_timeout(35)
            scroller.press('Control+End')
            expect(viewer).to_contain_text('after the match')
            assert viewer.locator('.paged-row').count() < 100
            results.filter(has_text='边界').click()
            expect(viewer.locator('.paged-row[data-line="1"] mark')).to_have_text('命中_')
            page.screenshot(path=str(out/'large-log.png'))
            metrics = page.evaluate('rangeMetrics')
            assert metrics['max'] <= BLOCK and metrics['full'] == 0, metrics
            assert PAGE in metrics['ends'], metrics
            # Even same-size changes within the target block invalidate its old anchor.
            stat = log.stat()
            with log.open('r+b') as f:
                f.seek(target_offset)
                f.write('🟠'.encode())
            os.utime(log,ns=(stat.st_atime_ns,stat.st_mtime_ns))
            tail.click()
            expect(viewer).to_contain_text('changed')
            box.fill('needle=123456')
            box.press('Enter')
            result = results.filter(has_text='needle=123456')
            expect(result).to_be_visible(timeout=90000)
            result.click()
            expect(viewer.locator('.paged-row[data-line="1"] mark')).to_have_text('needle=123456')
            box.fill('')
            page.get_by_role('checkbox', name=re.compile('Use Regular Expression')).click()
            box.fill(r'末尾 B\nafter the match')
            box.press('Enter')
            multiline = results.filter(has_text='末尾 B')
            expect(multiline).to_be_visible(timeout=90000)
            multiline.click()
            expect(viewer.locator('.paged-row[data-line="120002"] mark')).to_have_text('末尾 B')
            expect(viewer.locator('.paged-row[data-line="120003"] mark')).to_have_text('after the match')
            # Replacing a log while its index is in flight must not label the
            # new contents with the previous snapshot's counts.
            rotated = root / 'rotated.log'
            with rotated.open('wb') as f:
                for line in range(1, 50001):
                    f.write(f'{line:07d} first-generation log message\n'.encode())
            page.get_by_role('tab', name=re.compile('^Explorer')).click()
            page.evaluate("arcnixCommand('workbench.files.action.refreshFilesExplorer')")
            rotating_file = page.get_by_role('treeitem', name='rotated.log', exact=True)
            rotating_file.dblclick()
            expect(viewer.locator('.paged-row[data-line="1"] code')).to_contain_text('first-generation')
            pending = page.evaluate('rangeMetrics.summaries')
            page.evaluate('holdIndex()')
            scroller.evaluate('el => el.scrollTop = el.scrollHeight / 2')
            eventually(lambda: page.evaluate('rangeMetrics.summaries') == pending + 1)
            with rotated.open('wb') as f:
                for line in range(1, 56001):
                    f.write(f'{line:07d} replacement\n'.encode())
            page.evaluate('releaseIndex()')
            expect(viewer.locator('header')).to_contain_text('changed', timeout=20000)
            expect(viewer.locator('.paged-row[data-line]')).to_have_count(0)
            page.evaluate("arcnixCommand('workbench.action.closeActiveEditor')")
            rotating_file.dblclick()
            expect(viewer.locator('.paged-row[data-line="1"] code')).to_contain_text('replacement')
            scroller.evaluate('el => el.scrollTop = el.scrollHeight / 2')
            expect(viewer.locator('.paged-row[data-line] code').filter(has_text=re.compile(r'^002\d{4} replacement')).first).to_be_visible(timeout=20000)
            checked = viewer.locator('.paged-row[data-line] code').filter(has_text=re.compile(r'^\d{7} replacement')).evaluate_all(
                'nodes => nodes.map(el => Number(el.parentNode.dataset.line) === parseInt(el.textContent))')
            assert checked and all(checked)
            assert not errors, errors
            print(f'{log.stat().st_size} bytes, random seek line {middle_line}, {scans} index summaries, target line 120002, largest reply {metrics["max"]} characters; {out}')


if __name__ == '__main__':
    main()
