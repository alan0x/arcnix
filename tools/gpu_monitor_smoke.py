"""Check adapter selection, historical series and missing GPU data over an owned SSH session."""
import json
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def payload(page):
    return json.loads(page.locator('.resource-trend-data').text_content())


def main():
    out = ROOT / 'artifacts/gpu-monitor-ui' / str(time.time_ns())
    with Peer(out / 'peer') as peer:
        counters = peer.root / 'gpu-counters.txt'
        counters.write_text(
            'NV\t00000000:01:00.0, Discrete, 511, 21, N/A, 0, 1024, 6144, 43\n'
            'DRM\t0000:02:00.0\tIntegrated\ti915\t59\t\t\t\n', encoding='utf-8')
        config = peer.config([peer.profile('GPU host')], out)
        with Desktop(out / 'desktop', config) as app:
            p = app.page
            app.connect()
            app.application('Monitor')
            p.locator('.monitor-sidebar .monitor-gpu').click()
            expect(p.locator('.monitor-gpu-header')).to_contain_text('Discrete')
            expect(p.locator('.plot-gpu figure')).to_have_count(3)
            expect(p.locator('.resource-trend-decode .resource-trend-legend')).to_have_text('0%')
            assert 'encode' not in payload(p)['plots']
            p.get_by_role('button', name='Pause sampling', exact=True).click()
            refresh = p.get_by_role('button', name='Refresh resource values', exact=True)
            # Pausing interrupts the previous segment. Two fresh reads create a new line.
            for _ in range(2):
                count = len(payload(p)['points'])
                refresh.click()
                eventually(lambda: len(payload(p)['points']) > count)
            expect(p.locator('.resource-trend-dedicated .resource-trend-line')).to_have_count(1)
            expect(p.locator('.resource-trend-dedicated .resource-trend-legend')).not_to_contain_text('/s')
            p.get_by_role('button', name='GPU 1', exact=True).click()
            expect(p.locator('.monitor-gpu-header')).to_contain_text('Integrated')
            expect(p.locator('.plot-gpu figure')).to_have_count(1)
            assert all(row['utilization'] in (None,59) for row in payload(p)['points'])
            counters.write_text('', encoding='utf-8')
            p.get_by_role('button', name='Refresh resource values', exact=True).click()
            expect(p.locator('.monitor-gpu-header')).to_contain_text('No GPU reported by this host')
            expect(p.locator('.plot-gpu figure')).to_have_count(0)
            p.locator('.monitor-sidebar .monitor-cpu').click()
            expect(p.locator('.resource-trend-cpu')).to_be_visible()
    with Peer(out / 'mac-peer') as peer:
        (peer.root / 'host-uname.txt').write_bytes(b'Darwin\n')
        (peer.root / 'darwin-metrics.txt').write_bytes(
            b'ARCNIX_DARWIN_V1\n8589934592\nMach Virtual Memory Statistics: (page size of 16384 bytes)\n'
            b'Pages free: 4096.\nPages inactive: 2048.\n\nARCNIX_NETWORK\n\nARCNIX_DISKS\n'
            b'\nARCNIX_DARWIN_END\nARCNIX_DARWIN_CLOCK\n1020\n{ sec = 20, usec = 0 }\n'
            b'ARCNIX_DARWIN_TOP\nCPU usage: 93% idle\nCPU usage: 92% idle\nARCNIX_DARWIN_REMOTE_END\n')
        counters = peer.root / 'gpu-counters.txt'
        gpu = dict(id='4251', name='Apple M2', utilization=45, renderer=31, tiler=0, shared=536870912)
        counters.write_text('GPU\t' + json.dumps(gpu) + '\n', encoding='utf-8')
        with Desktop(out / 'mac-desktop', peer.config([peer.profile('Mac')], out)) as app:
            p = app.page
            app.connect()
            app.application('Monitor')
            p.locator('.monitor-sidebar .monitor-gpu').click()
            expect(p.locator('.monitor-gpu-header')).to_contain_text('Apple M2')
            expect(p.locator('.plot-gpu figure')).to_have_count(4)
            expect(p.locator('.resource-trend-tiler .resource-trend-legend')).to_have_text('0%')
            expect(p.locator('.resource-trend-renderer .resource-trend-legend')).to_have_text('31%')
            expect(p.locator('.resource-trend-shared .resource-trend-legend')).not_to_contain_text('/s')
            # Supply a subsequent host timestamp so the history spans two valid reads.
            metrics = peer.root / 'darwin-metrics.txt'
            metrics.write_bytes(metrics.read_bytes().replace(b'\n1020\n', b'\n1030\n'))
            p.get_by_role('button', name='Refresh resource values', exact=True).click()
            expect(p.locator('.resource-trend-utilization .resource-trend-line')).to_have_count(1)
            counters.write_text(json.dumps({'SPDisplaysDataType': [{'sppci_model': 'Fallback GPU'}]}))
            p.get_by_role('button', name='Refresh resource values', exact=True).click()
            expect(p.locator('.monitor-gpu-header')).to_contain_text('Fallback GPU')
            expect(p.locator('.plot-gpu figure')).to_have_count(0)
    print('GPU adapter switching, Linux and macOS charts, and inventory fallback passed.')


if __name__ == '__main__':
    main()
