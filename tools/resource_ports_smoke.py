"""Owned SSH/WebView checks for Resources ports and process termination."""
import json
import os
from pathlib import Path
import re
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def main():
    out = ROOT / 'artifacts/resource-ports-ui' / f'run-{time.time_ns()}'
    out.mkdir(parents=True)
    checks = []

    def passed(value):
        checks.append(value)
        print(value, flush=True)

    with Peer(out / 'peer') as peer:
        config = peer.config([peer.profile('Resources ports')], out)
        config['config']['settings']['resource_interval_seconds'] = 30

        def options(**values):
            (peer.root / 'resource-ports-fixture.json').write_text(json.dumps(values), encoding='utf-8')

        def queries(action=None):
            path = peer.root / 'resource-ports-queries.jsonl'
            values = [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
            return [value for value in values if action is None or value['action'] == action]

        binary = Path(os.environ['ARCNIX_TEST_BINARY']) if os.environ.get('ARCNIX_TEST_BINARY') else None
        with Desktop(out / 'desktop', config, binary=binary) as app:
            page = app.page
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            def ports():
                app.application('Monitor')
                page.locator('.monitor-sidebar').get_by_role('button', name='Processes', exact=True).click()
                details = page.get_by_role('button', name='Port details', exact=True)
                expect(details.or_(page.locator('.monitor-port-details'))).to_be_visible()
                if details.is_visible():
                    details.click()

            app.connect()
            ports()
            panel = page.locator('.resource-ports')
            rows = panel.locator('.resource-port-row')
            search = panel.locator('.resource-ports-search')
            refresh_button = panel.locator('.resource-ports-refresh')

            def refresh():
                count = len(queries('read'))
                password = panel.locator('.resource-ports-password')
                if password.count():
                    password.fill('fixture-admin-secret')
                refresh_button.click()
                eventually(lambda: len(queries('read')) > count)
                expect(refresh_button).to_be_enabled(timeout=20000)

            def web():
                return rows.filter(has_text='127.0.0.1:8080').first

            def dns():
                return rows.filter(has_text='0.0.0.0:5353').first

            def confirm(row):
                row.locator('.resource-port-kill').click()
                dialog = page.get_by_role('dialog', name='Force end process', exact=True)
                expect(dialog).to_be_visible()
                return dialog

            def cancel(dialog):
                dialog.get_by_role('button', name='Cancel', exact=True).click()
                expect(dialog).not_to_be_visible()

            expect(rows).to_have_count(4, timeout=20000)
            expect(web()).to_contain_text('fixture-web')
            expect(web()).to_contain_text('200')
            expect(web().locator('.resource-port-details')).to_contain_text('/usr/bin/fixture-web')
            expect(web().locator('.resource-port-details')).to_contain_text('fixture-web.service')
            expect(panel).to_contain_text('[::]:8080')
            expect(panel).to_contain_text('UDP')
            expect(panel.locator('.resource-ports-warning')).to_contain_text('unavailable')
            assert queries('read')[0]['administrator'] is False
            panel.scroll_into_view_if_needed()
            check = panel.locator('.resource-ports-admin input[type="checkbox"]').bounding_box()
            assert check and check['width'] <= 22 and check['height'] <= 22, check
            app.screenshot('ports-light.png')
            passed('Resources automatically lists TCP/UDP IPv4/IPv6 bindings with PID, executable, service and explicit process visibility limits')

            for value, count in [('fixture-web', 2), ('5353', 2), ('/usr/bin/fixture-web', 2), ('not-a-listener', 0)]:
                search.fill(value)
                expect(rows).to_have_count(count)
            search.fill('')
            expect(rows).to_have_count(4)
            many = [dict(protocol='tcp', family=4, address='127.0.0.1', port=10000 + index, uid=1000,
                         inode=1000 + index, holders_limited=False,
                         holders=[dict(pid=500 + index, start_ticks=30000 + index, uid=1000, fd=3,
                                       command=f'fixture-worker-{index}', executable='/usr/bin/fixture-worker', unit=None)])
                    for index in range(80)]
            options(snapshot=dict(rows=many, matched=80))
            refresh()
            expect(rows).to_have_count(80)
            search.fill('10079')
            expect(rows).to_have_count(1)
            expect(rows).to_contain_text('579')
            search.fill('')
            options()
            refresh()
            expect(rows).to_have_count(4)
            passed('Search covers program, executable and port; more than twenty bindings stay available without diagnostic-row truncation')

            count = len(queries('terminate'))
            dialog = confirm(web())
            expect(dialog.locator('.resource-port-confirm-target')).to_contain_text('200')
            expect(dialog.locator('.resource-port-confirm-target')).to_contain_text('fixture-web')
            expect(dialog).to_contain_text('SIGKILL')
            cancel(dialog)
            assert len(queries('terminate')) == count
            assert not (peer.root / 'resource-ports-terminated.json').exists()
            passed('Force end opens a concrete process confirmation and cancelling sends no termination request')

            dialog = confirm(web())
            # Change the remote boot after the UI captured its target. The
            # fixture independently validates that captured identity at commit.
            options(snapshot=dict(boot_id='22222222-2222-4222-8222-222222222222'))
            dialog.locator('.resource-port-confirm-submit').click()
            eventually(lambda: len(queries('terminate')) == count + 1)
            expect(page.locator('body')).to_contain_text('Process identity or listening socket changed')
            if dialog.is_visible():
                cancel(dialog)
            assert not (peer.root / 'resource-ports-terminated.json').exists()
            expect(rows).to_have_count(4)
            options()
            refresh()
            dialog = confirm(web())
            dialog.locator('.resource-port-confirm-submit').click()
            expect(dialog).not_to_be_visible(timeout=20000)
            expect(rows).to_have_count(2, timeout=20000)
            assert json.loads((peer.root / 'resource-ports-terminated.json').read_text()) == [200]
            assert queries('terminate')[-1]['target']['holder']['pid'] == 200
            assert queries('terminate')[-1]['target']['holder']['start_ticks'] == 12345
            assert queries('terminate')[-1]['target']['boot_id'] == '11111111-1111-4111-8111-111111111111'
            passed('Changed process identity is rejected; a confirmed valid termination targets the original identity and refreshes every socket owned by that PID')

            dialog = confirm(dns())
            dialog.locator('.resource-port-confirm-submit').click()
            expect(page.locator('body')).to_contain_text('Permission denied')
            if dialog.is_visible():
                cancel(dialog)
            assert json.loads((peer.root / 'resource-ports-terminated.json').read_text()) == [200]
            dialog = confirm(dns())
            dialog.locator('.resource-port-confirm-admin').click()
            dialog.locator('.resource-port-confirm-password').fill('fixture-admin-secret')
            dialog.locator('.resource-port-confirm-submit').click()
            expect(dialog).not_to_be_visible(timeout=20000)
            expect(rows).to_have_count(1, timeout=20000)
            assert queries('terminate')[-1]['administrator'] is True
            assert json.loads((peer.root / 'resource-ports-terminated.json').read_text()) == [200, 201]
            assert 'fixture-admin-secret' not in app.config.read_text()
            assert not app.config.with_suffix('.tasks.json').exists()
            passed('Another user process reports permission failure; explicit administrator confirmation works and does not persist the sudo password')

            (peer.root / 'resource-ports-terminated.json').unlink()
            panel.locator('.resource-ports-admin').click()
            panel.locator('.resource-ports-password').fill('fixture-admin-secret')
            refresh()
            expect(rows).to_have_count(4)
            assert queries('read')[-1]['administrator'] is True
            options(snapshot=dict(limits=['time']))
            refresh()
            expect(panel.locator('.resource-ports-warning')).to_contain_text(re.compile('limit', re.I))
            options(raw='invalid JSON')
            refresh()
            expect(panel.locator('.resource-ports-error')).not_to_have_text('')
            options(snapshot=dict(namespace=None))
            refresh()
            for button in panel.locator('.resource-port-kill').all():
                expect(button).to_be_disabled()
            options()
            refresh()
            expect(rows).to_have_count(4)
            passed('Administrator inspection, scan-limit coverage and malformed-response errors are explicit; unknown network identity disables termination and a fresh read recovers')

            app.settings('General')
            page.locator('.setting-theme:visible').get_by_role('radio', name='Dark', exact=True).check()
            app.servers()
            ports()
            page.set_viewport_size(dict(width=1000, height=780))
            panel.scroll_into_view_if_needed()
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
            expect(panel.locator('.resource-ports-refresh')).to_be_visible()
            dialog = confirm(web())
            box = dialog.bounding_box()
            assert box and box['x'] >= 0 and box['x'] + box['width'] <= 1000
            submit = dialog.locator('.resource-port-confirm-submit').bounding_box()
            assert submit and submit['y'] + submit['height'] <= 780
            check = dialog.locator('.resource-port-confirm-admin input[type="checkbox"]').bounding_box()
            assert check and check['width'] <= 22 and check['height'] <= 22, check
            label_color = dialog.locator('.resource-port-confirm-admin').evaluate('element => getComputedStyle(element).color')
            assert label_color not in ('rgb(0, 0, 0)', 'rgb(28, 28, 28)'), label_color
            app.screenshot('ports-dark-1000-confirm.png')
            cancel(dialog)
            passed('Dark 1000px layouts keep port rows and the confirmation actions inside the viewport')

            app.settings('General')
            page.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check()
            app.servers()
            page.locator('.desktop-dock').get_by_role('button', name='监控', exact=True).click()
            page.locator('.monitor-sidebar').get_by_role('button', name='进程', exact=True).click()
            details = page.get_by_role('button', name='端口详情', exact=True)
            expect(details.or_(page.locator('.monitor-port-details'))).to_be_visible()
            if details.is_visible():
                details.click()
            heading = page.locator('.monitor-titlebar')
            expect(heading.locator('.monitor-heading')).to_have_text('进程')
            expect(heading.locator('.help-trigger')).to_be_visible()
            expect(refresh_button).to_have_text('刷新端口')
            web().locator('.resource-port-kill').click()
            dialog = page.get_by_role('dialog', name='强制结束进程', exact=True)
            expect(dialog).to_contain_text('未保存的数据可能丢失')
            app.screenshot('ports-chinese-dark-1000.png')
            dialog.get_by_role('button', name='取消', exact=True).click()
            app.settings('常规')
            page.locator('.setting-language:visible').get_by_role('radio', name='English', exact=True).check()
            app.servers()
            ports()
            passed('Chinese port labels and the destructive-action warning remain translated in the narrow layout')

            page.get_by_role('button', name='Disconnect', exact=True).click()
            expect(page.locator('.connection-state')).to_have_text('SSH disconnected')
            expect(rows).to_have_count(0)
            expect(panel.locator('.resource-ports-password')).to_have_count(0)
            app.connect()
            ports()
            expect(rows).to_have_count(4, timeout=20000)
            assert queries('read')[-1]['administrator'] is False
            assert not errors, errors
            passed('Disconnect clears snapshots and credentials; reconnect starts an ordinary-user query without stale process targets')

            panel.locator('.resource-ports-admin').click()
            panel.locator('.resource-ports-password').fill('fixture-admin-secret')
            options(disconnect_transport=True)
            count = len(queries('read'))
            started = time.monotonic()
            refresh_button.click()
            eventually(lambda: len(queries('read')) > count, timeout=3)
            expect(page.locator('.connection-state')).to_have_text('SSH disconnected', timeout=4000)
            assert time.monotonic() - started < 5
            expect(rows).to_have_count(0)
            expect(panel.locator('.resource-ports-password')).to_have_count(0)
            options()
            app.connect()
            ports()
            expect(rows).to_have_count(4, timeout=20000)
            assert queries('read')[-1]['administrator'] is False
            assert not errors, errors
            passed('A transport failure during the port read immediately marks SSH disconnected, clears every stale target and credential, and permits a clean reconnect')

    (out / 'results.json').write_text(json.dumps(checks, ensure_ascii=False, indent=2), encoding='utf-8')
    print(f'{len(checks)} groups passed; artifacts: {out}', flush=True)


if __name__ == '__main__':
    main()
