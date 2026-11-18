"""Owned SSH/WebView checks for Docker management and remote desktop preferences."""
import json
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def seed():
    return dict(version='28.5.1', containers=[
        dict(id='a' * 64, name='web', image='nginx:alpine', state='running', status='Up 2 hours', ports='127.0.0.1:8080->80/tcp, [::1]:8443->443/tcp', mounts='web-data', networks='bridge',
             details=dict(restart='unless-stopped', retries=0, memory=268435456, nano_cpus=1_000_000_000, network='bridge',
                          ports={'80/tcp': [dict(HostIp='127.0.0.1', HostPort='8080')], '443/tcp': [dict(HostIp='::1', HostPort='8443')], '9000/tcp': None},
                          bindings={'80/tcp': [dict(HostIp='127.0.0.1', HostPort='8080')]},
                          mounts=[dict(Type='volume', Source='/var/lib/docker/volumes/web-data/_data', Destination='/usr/share/nginx/html', RW=True)]),
             logs='2026-11-17T11:00:00Z fixture web ready\n', stderr_logs='2026-11-17T11:00:01Z fixture stderr included\n'),
        dict(id='b' * 64, name='worker', image='busybox:latest', state='exited', status='Exited (0)', ports='', mounts='', networks='host',
             details=dict(restart='no', retries=0, memory=0, nano_cpus=0, network='host', ports={}, bindings={}, mounts=[]), logs=''),
    ], volumes=[dict(name='web-data', driver='local', scope='local', mountpoint='/var/lib/docker/volumes/web-data/_data'), dict(name='unused-volume', driver='local', scope='local', mountpoint='/var/lib/docker/volumes/unused-volume/_data')])


def main():
    output = ROOT / 'artifacts/docker-desktop' / f'run-{time.time_ns()}'
    output.mkdir(parents=True)
    local = output / 'local'
    local.mkdir()
    checks = []
    with Peer(output / 'peer') as peer, Peer(output / 'other-peer') as other:
        docker_file = peer.root / 'docker-fixture.json'
        docker_file.write_text(json.dumps(seed()))
        profiles = [peer.profile('Docker host'), dict(peer.profile('Other user'), username='arcnix-nosftp'), peer.profile('Same login'), other.profile('Other machine')]
        config = peer.config(profiles, local)
        config['config']['trusted_hosts'] += other.config([], local)['config']['trusted_hosts']
        with Desktop(output / 'desktop', config) as app:
            page = app.page
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))

            def docker():
                app.application('Docker')
                expect(page.locator('.docker-page')).to_be_visible()
                expect(page.locator('.docker-page').get_by_role('button', name='Refresh Docker', exact=True)).to_be_enabled()

            def settings(section='Desktop'):
                app.application('Settings')
                panel = page.locator('.desktop-settings-page')
                panel.get_by_role('button', name=section, exact=True).click()
                if section == 'Desktop':
                    expect(panel.get_by_role('button', name='Dark', exact=True)).to_be_enabled()
                return panel

            def close_settings():
                page.locator('.desktop-window.active .window-close').click()
                expect(page.locator('.desktop-settings-page')).to_have_count(0)

            def close(dialog):
                dialog.get_by_role('button', name='Close dialog', exact=True).click()
                expect(dialog).not_to_be_visible()

            def select(name):
                page.locator('.docker-container-list').get_by_role('button', name=name, exact=True).click()
                expect(page.locator('.docker-container-actions').get_by_role('button', name='Configure', exact=True)).to_be_enabled()

            app.connect()
            docker()
            expect(page.locator('.desktop-shortcuts .app-docker')).to_have_count(1)
            expect(page.locator('.desktop-dock .app-docker')).to_have_count(1)
            expect(page.locator('.docker-summary')).to_contain_text('2 containers · 2 volumes')
            expect(page.locator('.docker-port-details')).to_contain_text('127.0.0.1:8080 → 80/tcp')
            expect(page.locator('.docker-port-details')).to_contain_text('[::1]:8443 → 443/tcp')
            expect(page.locator('.docker-port-details')).to_contain_text('9000/tcp · Exposed only')
            expect(page.locator('.docker-mount-details')).to_contain_text('/usr/share/nginx/html')
            app.screenshot('docker-light.png')
            checks.append('Docker is discovered automatically; one icon per launcher opens container state, IPv4/IPv6 mappings, exposed ports and volume mounts')

            page.locator('.docker-tabs').get_by_role('button', name='Volumes', exact=True).click()
            page.locator('.docker-volume-list').get_by_role('button', name='web-data', exact=True).click()
            expect(page.locator('.docker-volume-details')).to_contain_text('/var/lib/docker/volumes/web-data/_data')
            expect(page.locator('.docker-volume-list').get_by_role('button', name='web-data', exact=True)).to_contain_text('web')
            expect(page.locator('.docker-volume-list').get_by_role('button', name='unused-volume', exact=True)).to_be_visible()
            page.locator('.docker-tabs').get_by_role('button', name='Ports', exact=True).click()
            expect(page.locator('.docker-port-row')).to_have_count(2)
            expect(page.locator('.docker-ports').get_by_role('button', name='worker', exact=True)).to_contain_text('Host network')
            page.locator('.docker-ports').get_by_role('button', name='Server listeners', exact=True).click()
            expect(page.locator('.desktop-window.active .desktop-window-title')).to_have_text('Monitor')
            expect(page.locator('.monitor-content .monitor-heading')).to_have_text('Processes')
            expect(page.locator('.monitor-port-details .resource-ports')).to_be_visible()
            docker()
            page.locator('.docker-ports').get_by_role('button', name='web', exact=True).click()
            expect(page.locator('.docker-detail-name')).to_have_text('web')
            page.locator('.docker-filter').fill('worker')
            expect(page.locator('.docker-container-row')).to_have_count(1)
            page.locator('.docker-filter').fill('')
            page.locator('.docker-container-actions').get_by_role('button', name='Logs', exact=True).click()
            expect(page.locator('.docker-log-output')).to_contain_text('fixture web ready')
            expect(page.locator('.docker-log-output')).to_contain_text('fixture stderr included')
            page.locator('.docker-log-panel').get_by_role('button', name='Details', exact=True).click()
            checks.append('Volumes show their paths and consumers; ports link back to containers; filters and bounded stdout/stderr log snapshots work')

            def action(name, title, state):
                page.locator('.docker-container-actions').get_by_role('button', name=name, exact=True).click()
                dialog = page.get_by_role('dialog', name=title, exact=True)
                expect(dialog).to_contain_text('a' * 64)
                dialog.get_by_role('button', name='Confirm', exact=True).click()
                expect(dialog).not_to_be_visible()
                eventually(lambda: json.loads(docker_file.read_text())['containers'][0]['state'] == state)
                expect(page.locator('.docker-container-actions').get_by_role('button', name='Configure', exact=True)).to_be_enabled()

            action('Stop', 'Stop container', 'exited')
            action('Start', 'Start container', 'running')
            action('Restart', 'Restart container', 'running')
            page.locator('.docker-container-actions').get_by_role('button', name='Configure', exact=True).click()
            dialog = page.get_by_role('dialog', name='Configure container', exact=True)
            dialog.get_by_role('textbox', name='Restart policy', exact=True).fill('always; reboot')
            dialog.get_by_role('button', name='Save to server', exact=True).click()
            expect(dialog).to_contain_text('Invalid restart policy')
            dialog.get_by_role('textbox', name='Restart policy', exact=True).fill('on-failure:3')
            dialog.get_by_role('textbox', name='CPU limit', exact=True).fill('1.5')
            dialog.get_by_role('textbox', name='Memory limit (MiB)', exact=True).fill('512')
            dialog.get_by_role('button', name='Save to server', exact=True).click()
            expect(dialog).not_to_be_visible()
            eventually(lambda: json.loads(docker_file.read_text())['containers'][0]['details']['memory'] == 512 * 1024 * 1024)
            expect(page.locator('.docker-configuration-summary')).to_contain_text('on-failure')
            checks.append('Start, stop and restart use reviewed full container IDs; configuration rejects shell input and applies restart policy plus CPU/memory limits')

            app.application('Terminal')
            reader = page.locator('.terminal-surface .xterm-accessibility-tree')
            keyboard = page.locator('.terminal-surface .xterm-helper-textarea')
            expect(reader).to_contain_text('Arcnix terminal fixture')
            keyboard.focus()
            page.keyboard.type('echo THEME_SESSION_PRESERVED')
            page.keyboard.press('Enter')
            expect(reader).to_contain_text('THEME_SESSION_PRESERVED')
            dialog = settings()
            dialog.get_by_role('button', name='Dark', exact=True).click()
            expect(dialog.get_by_role('button', name='Dark', exact=True)).to_have_attribute('aria-pressed', 'true')
            expect(dialog).to_contain_text('Saved on server')
            preferences = peer.root / 'home/arcnix/.arcnix/desktop.json'
            assert json.loads(preferences.read_text())['theme'] == 'Dark'
            close_settings()
            expect(page.locator('.server-desktop')).to_have_class('server-desktop theme-dark desktop-dark')
            app.application('Terminal')
            expect(reader).to_contain_text('THEME_SESSION_PRESERVED')
            docker()
            for width in (1100, 900):
                page.set_viewport_size(dict(width=width, height=740))
                assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
                page.locator('.workbench-tabs').get_by_role('button', name='Fullscreen desktop', exact=True).click()
                app.screenshot(f'docker-dark-fullscreen-{width}.png')
                page.get_by_role('button', name='Exit desktop fullscreen', exact=True).click()
            page.set_viewport_size(dict(width=1440, height=980))
            app.workspace('Home')
            expect(page.locator('.app')).to_have_class('app theme-light')
            app.workspace('Desktop')
            expect(page.locator('.app')).to_have_class('app theme-light')
            checks.append('Remote-user theme saves to ~/.arcnix/desktop.json, changes the desktop and controls, preserves terminal history, and leaves Home using the client theme')

            app.stop()
            app.launch()
            page = app.page
            page.on('pageerror', lambda error: errors.append(str(error)))
            app.connect()
            docker()
            expect(page.locator('.server-desktop')).to_have_class('server-desktop theme-dark desktop-dark')
            denied = peer.root / 'desktop-write-denied'
            denied.touch()
            dialog = settings()
            dialog.get_by_role('button', name='Light', exact=True).click()
            expect(dialog).to_contain_text('Permission denied')
            expect(page.locator('.server-desktop')).to_have_class('server-desktop theme-dark desktop-dark')
            assert json.loads(preferences.read_text())['theme'] == 'Dark'
            denied.unlink()
            dialog.get_by_role('button', name='Light', exact=True).click()
            expect(dialog.get_by_role('button', name='Light', exact=True)).to_have_attribute('aria-pressed', 'true')
            expect(dialog).to_contain_text('Saved on server')
            close_settings()
            expect(page.locator('.server-desktop')).to_have_class('server-desktop theme-light desktop-light')
            app.screenshot('docker-light-restored.png')
            original = preferences.read_text()
            preferences.write_text('{invalid JSON')
            dialog = settings('General')
            dialog.get_by_role('button', name='Reload settings', exact=True).click()
            expect(dialog.locator('.desktop-preferences > .error')).not_to_be_empty()
            dialog.get_by_role('button', name='Desktop', exact=True).click()
            expect(dialog.get_by_role('button', name='Dark', exact=True)).to_be_disabled()
            assert preferences.read_text() == '{invalid JSON'
            preferences.write_text(original)
            dialog.get_by_role('button', name='General', exact=True).click()
            dialog.get_by_role('button', name='Reload settings', exact=True).click()
            dialog.get_by_role('button', name='Desktop', exact=True).click()
            expect(dialog.get_by_role('button', name='Dark', exact=True)).to_be_enabled()
            close_settings()
            docker()
            checks.append('A new client process restores the server theme; failed writes retain the previous file/theme and can be retried')

            denied = peer.root / 'docker-denied'
            denied.touch()
            page.get_by_role('button', name='Refresh Docker', exact=True).click()
            expect(page.locator('.docker-error')).to_contain_text('permission denied')
            expect(page.locator('.docker-container-actions').get_by_role('button', name='Restart', exact=True)).to_be_disabled()
            denied.unlink()
            page.get_by_role('button', name='Refresh Docker', exact=True).click()
            expect(page.locator('.docker-container-actions').get_by_role('button', name='Configure', exact=True)).to_be_enabled()
            truncated = peer.root / 'docker-truncated'
            truncated.touch()
            page.get_by_role('button', name='Refresh Docker', exact=True).click()
            expect(page.locator('.docker-error')).to_contain_text('Incomplete Docker inventory')
            expect(page.locator('.docker-container-row')).to_have_count(2)
            truncated.unlink()
            page.get_by_role('button', name='Refresh Docker', exact=True).click()
            expect(page.locator('.docker-container-actions').get_by_role('button', name='Configure', exact=True)).to_be_enabled()
            trace = peer.root / 'server-app-queries.jsonl'
            count = len(trace.read_text().splitlines())
            page.wait_for_timeout(1800)
            assert len(trace.read_text().splitlines()) == count
            checks.append('Daemon permission errors retain inventory and block stale actions; explicit refresh recovers and no background Docker polling runs')

            # Same host/user shares preferences; a second SSH user has an independent home.
            page.locator('.host-select').nth(1).click()
            app.workspace('Home')
            page.locator('.overview-connect').click()
            page.get_by_role('dialog').get_by_role('button', name='Connect', exact=True).click()
            expect(page.locator('.connection-state')).to_have_text('SSH connected')
            docker()
            dialog = settings()
            dialog.get_by_role('button', name='Dark', exact=True).click()
            expect(dialog.get_by_role('button', name='Dark', exact=True)).to_have_attribute('aria-pressed', 'true')
            expect(dialog).to_contain_text('Saved on server')
            close_settings()
            second = peer.root / 'home/arcnix-nosftp/.arcnix/desktop.json'
            assert json.loads(second.read_text())['theme'] == 'Dark'
            assert json.loads(preferences.read_text())['theme'] == 'Light'
            page.locator('.host-select').nth(2).click()
            page.locator('.overview-connect').click()
            page.get_by_role('dialog').get_by_role('button', name='Connect', exact=True).click()
            expect(page.locator('.connection-state')).to_have_text('SSH connected')
            docker()
            expect(page.locator('.server-desktop')).to_have_class('server-desktop theme-light desktop-light')
            checks.append('Settings are shared by matching remote host/user across profiles and isolated by SSH user; Docker/settings work with SFTP disabled')

            hidden_seed = docker_file.with_suffix('.disabled')
            docker_file.rename(hidden_seed)
            dialog = settings('General')
            dialog.get_by_role('button', name='Detect applications', exact=True).click()
            expect(dialog).to_contain_text('Docker not detected')
            close_settings()
            expect(page.locator('.desktop-dock .app-docker')).to_have_count(0)
            hidden_seed.rename(docker_file)
            dialog = settings('General')
            dialog.get_by_role('button', name='Detect applications', exact=True).click()
            expect(dialog).to_contain_text('Docker detected')
            close_settings()
            docker()
            # A late log response from the previous connection must not populate another server.
            (peer.root / 'docker-slow').touch()
            page.locator('.docker-container-actions').get_by_role('button', name='Logs', exact=True).click()
            page.locator('.host-select').nth(3).click()
            page.locator('.overview-connect').click()
            page.get_by_role('dialog').get_by_role('button', name='Connect', exact=True).click()
            expect(page.locator('.connection-state')).to_have_text('SSH connected')
            expect(page.locator('.desktop-shortcuts .app-docker')).to_have_count(0)
            expect(page.locator('.desktop-shortcuts').get_by_role(
                'button', name='Web browser', exact=True,
            )).to_be_visible()
            expect(page.locator('.docker-page')).to_have_count(0)
            expect(page.locator('.server-desktop')).to_have_class('server-desktop theme-light desktop-light')
            assert not (other.root / 'home/arcnix/.arcnix/desktop.json').exists()
            checks.append('Malformed settings are preserved, partial inventory is rejected, and delayed Docker responses cannot leak into another server')
            # Return to the original login for localization checks.
            page.locator('.host-select').first.click()
            page.locator('.overview-connect').click()
            page.get_by_role('dialog').get_by_role('button', name='Connect', exact=True).click()
            expect(page.locator('.connection-state')).to_have_text('SSH connected')
            docker()
            app.settings('General')
            page.get_by_role('button', name='简体中文', exact=True).click()
            app.servers()
            expect(page.locator('.docker-tabs')).to_contain_text('容器')
            expect(page.locator('.desktop-dock .app-settings')).to_have_accessible_name('设置')
            app.screenshot('docker-chinese-light.png')
            commands = [json.loads(line)['command'] for line in trace.read_text().splitlines()]
            assert all('--follow' not in cmd and 'docker run ' not in cmd for cmd in commands)
            assert not errors, errors
            checks.append('Explicit rediscovery adds/removes Docker without duplicate launchers; Chinese localization works and queries use finite commands without container recreation')
    (output / 'results.json').write_text(json.dumps(checks, indent=2))
    print(json.dumps(dict(output=str(output), checks=checks), indent=2))


if __name__ == '__main__':
    main()
