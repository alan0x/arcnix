"""Exercise live SFTP append, incremental indexing and scroll intent in Code."""
import re
import time
from desktop_fixture import Desktop, Peer, ROOT, eventually, expect

PAGE = 128 * 1024 - 4


def main():
    out = ROOT / 'artifacts/code-follow' / str(time.time_ns())
    out.mkdir(parents=True)
    with Peer(out / 'peer', io_delay=0.002) as peer:
        root = peer.root / 'logs'
        root.mkdir()
        log = root / 'live.log'
        initial_size = 16 * PAGE
        records = (initial_size - 128) // 64
        with log.open('wb') as file:
            for line in range(1, records + 1):
                file.write(f'{line:08d} heartbeat'.encode().ljust(63) + b'\n')
            # EOF is exactly a page boundary; its CR becomes part of a CRLF.
            file.write(b'boundary'.rjust(initial_size - file.tell() - 1, b'-') + b'\r')
        profile = peer.profile('Follow logs')
        profile['initial_remote_path'] = '/logs'

        def append(data):
            with log.open('ab') as file:
                file.write(data)

        with Desktop(out / 'app', peer.config([profile], out)) as app:
            app.connect()
            app.application('Code Editor')
            page = app.code()
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            source = page.get_by_role('treeitem', name='live.log', exact=True)
            expect(source).to_be_visible()
            page.evaluate("""() => {
                window.traffic = { bodies: [], counts: [], polls: 0, max: 0 };
                const reply = window.arcnixReply;
                let waiting, hold = false;
                window.holdRead = () => { hold = true; };
                window.hasHeld = () => !!waiting;
                window.releaseRead = () => { reply([waiting]); waiting = undefined; };
                window.arcnixReply = list => reply(list.filter(item => {
                    const v = item.value;
                    if (v?.stamp) traffic.polls++;
                    if (typeof v?.breaks === 'number') {
                        if (v.text === undefined) traffic.counts.push(v.next);
                        else {
                            traffic.bodies.push(v.next);
                            traffic.max = Math.max(traffic.max, v.text.length);
                            if (hold) { hold = false; waiting = item; return false; }
                        }
                    }
                    return true;
                }));
            }""")
            source.dblclick()
            viewer = page.locator('.arcnix-paged-file')
            body = viewer.locator('.paged-lines')
            row = lambda line: viewer.locator(f'.paged-row[data-line="{line}"] code')
            expect(row(1)).to_contain_text('heartbeat')
            body.focus()
            body.press('Control+End')
            expect(row(records + 1)).to_contain_text('boundary', timeout=30000)
            start = page.evaluate('[traffic.bodies.length, traffic.counts.length]')
            emoji = '🚦'.encode()
            append(b'\n' + '新增 '.encode() + emoji[:2])
            expect(row(records + 2)).to_have_text('新增 ', timeout=12000)
            append(emoji[2:] + b' complete\n')
            expect(row(records + 2)).to_have_text('新增 🚦 complete', timeout=12000)
            append(b'next append\n')
            expect(row(records + 3)).to_have_text('next append', timeout=12000)
            at_end = 'el => el.scrollHeight - el.scrollTop - el.clientHeight < 2'
            assert body.evaluate(at_end)
            traffic = page.evaluate('traffic')
            for offset in traffic['bodies'][start[0]:] + traffic['counts'][start[1]:]:
                assert offset >= initial_size - PAGE, offset

            # Move the scrollbar while a follow read is on its way back to the UI.
            page.evaluate('holdRead()')
            append(b'in flight\n')
            eventually(lambda: page.evaluate('hasHeld()'))
            body.evaluate('el => el.scrollTop = (el.scrollHeight - el.clientHeight) * .4')
            middle = viewer.locator('.paged-row code').filter(has_text=re.compile('heartbeat'))
            expect(middle.first).to_be_visible()
            first = middle.first.inner_text()
            top = body.evaluate('el => el.scrollTop')
            page.evaluate('releaseRead()')
            burst = 14000
            append(b'burst record: background append\n' * burst + b'burst end\n')
            size = log.stat().st_size
            eventually(lambda: page.evaluate('size => traffic.counts.includes(size)', size), timeout=25)
            assert body.evaluate('el => el.scrollTop') == top
            expect(middle.first).to_have_text(first)
            assert not body.evaluate(at_end)
            # Even when paused the index extended to EOF without reading the prefix again.
            late = page.evaluate('traffic.counts')
            assert all(end >= initial_size - PAGE for end in late[start[1]:])
            body.press('End')
            expect(row(records + 5 + burst)).to_have_text('burst end', timeout=15000)
            append(b'resumed\n')
            expect(row(records + 6 + burst)).to_have_text('resumed', timeout=12000)
            assert body.evaluate(at_end)
            idle = page.evaluate('[traffic.polls, traffic.bodies.length, traffic.counts.length]')
            eventually(lambda: page.evaluate('traffic.polls') >= idle[0] + 2)
            assert page.evaluate('[traffic.bodies.length, traffic.counts.length]') == idle[1:]
            # A larger replacement with different bytes at the old EOF is not an append.
            with log.open('r+b') as file:
                file.seek(-32, 2)
                file.write(b'rotated!' * 20)
            expect(viewer.locator('header')).to_contain_text('changed', timeout=12000)
            assert page.evaluate('traffic.max') <= 128 * 1024
            assert not errors, errors
            print(f'{size} bytes, followed line {records + 6 + burst}, paused at {first.strip()}, {out}')


if __name__ == '__main__':
    main()
