"""Owned SSH/WebView scenarios for desktop isolation, installers and session restore."""
import json,time,re
from server_apps_fixture import read_desktop_settings
from desktop_fixture import Desktop,Peer,ROOT,expect,eventually, set_desktop_mode
from docker_desktop_smoke import seed

def main():
    out=ROOT/'artifacts/desktop-extensions'/f'run-{time.time_ns()}';out.mkdir(parents=True)
    local=out/'local';local.mkdir();checks=[]
    with Peer(out/'ubuntu') as ubuntu,Peer(out/'fedora') as fedora:
        (ubuntu.root/'docker-fixture.json').write_text(json.dumps(seed()))
        (ubuntu.root/'tmux-enabled').touch();(ubuntu.root/'software-snap-enabled').touch()
        (ubuntu.root/'large-process-inventory').touch()
        (ubuntu.root/'work').mkdir();(ubuntu.root/'data').mkdir();(ubuntu.root/'data/disk-file.txt').write_text('Disk mount demo')
        (fedora.root/'software-manager').write_text('dnf')
        (fedora.root/'software-snap-enabled').touch()
        (ubuntu.root/'trace-task-queries').touch();(fedora.root/'trace-task-queries').touch()
        config=ubuntu.config([ubuntu.profile('Ubuntu test'),fedora.profile('Fedora test')],local)
        config['config']['trusted_hosts']+=fedora.config([],local)['config']['trusted_hosts']
        with Desktop(out/'desktop',config) as app:
            p=app.page;errors=[];p.on('pageerror',lambda e:errors.append(str(e)))
            def enter(label):
                app.application('Monitor' if label=='Services' else label)
                if label=='Services':
                    p.locator('.monitor-sidebar').get_by_role('button',name='Services',exact=True).click()
            def close(dialog):dialog.get_by_role('button',name='Close dialog',exact=True).click();expect(dialog).not_to_be_visible()
            def finish_task(review=True):
                d=p.get_by_role('dialog',name='Operation status',exact=True)
                if review:
                    p.get_by_role('dialog',name='Run script on servers',exact=True).get_by_role('button',name='Check targets',exact=True).click()
                    expect(d.locator('.task-status')).to_have_text(['Ready'],timeout=25000)
                    d.get_by_role('button',name='Run operation',exact=True).click()
                fits_desktop(d)
                expect(p.locator('.rail-servers')).to_have_class(re.compile(r'\bactive\b'))
                expect(d.locator('.task-status')).to_have_text(['Succeeded'],timeout=25000)
                d.get_by_role('button',name='Close',exact=True).click()
            app.connect();app.workspace('Desktop')
            expect(p.locator('.desktop-shortcuts').get_by_role('button',name='Player',exact=True)).to_have_count(0)
            expect(p.locator('.desktop-dock').get_by_role('button',name='Monitor',exact=True)).to_be_visible()
            expect(p.locator('.desktop-dock').get_by_role('button',name='Services',exact=True)).to_have_count(0)
            expect(p.locator('.desktop-dock').get_by_role('button',name='Logs',exact=True)).to_have_count(0)
            rail=p.locator('.navigation-rail').evaluate('el=>getComputedStyle(el).backgroundColor')
            def fits_desktop(popup):
                expect(popup).to_be_visible()
                def contained():
                    outer=p.locator('.desktop-canvas').bounding_box()
                    inner=popup.bounding_box()
                    return inner and all((
                        inner['x'] >= outer['x']-1, inner['y'] >= outer['y']-1,
                        inner['x']+inner['width'] <= outer['x']+outer['width']+1,
                        inner['y']+inner['height'] <= outer['y']+outer['height']+1,
                    ))
                eventually(contained)

            app.application('Settings');d=p.locator('.desktop-settings-page')
            expect(p.locator('.desktop-window.active .desktop-window-title')).to_have_text('Settings')
            fits_desktop(p.locator('.desktop-window.active'))
            expect(p.locator('.desktop-modal-backdrop')).to_have_count(0)
            d.locator('select.setting-resource-refresh-interval:visible').select_option(label='Every 10 s')
            prefs=ubuntu.root/'home/arcnix/.arcnix/desktop.json'
            eventually(lambda: read_desktop_settings(prefs).get('resource_interval_seconds') == 10)
            d.get_by_role('button',name='Keyboard shortcuts',exact=True).click()
            expect(d).to_contain_text('Ctrl+Shift+T')
            expect(d).to_contain_text('Ctrl+S')
            p.set_viewport_size(dict(width=900,height=650))
            assert d.locator('.desktop-preferences').evaluate('el=>el.scrollWidth <= el.clientWidth + 1')
            p.set_viewport_size(dict(width=1440,height=980))
            set_desktop_mode(d, 'Dark')
            expect(p.locator('.app')).to_have_class('app theme-light')
            expect(p.locator('.server-desktop')).to_have_class(re.compile(r'\btheme-dark\b'))
            assert p.locator('.navigation-rail').evaluate('el=>getComputedStyle(el).backgroundColor')==rail
            assert p.locator('.app').evaluate('el=>getComputedStyle(el).getPropertyValue("--surface").trim()') != p.locator('.server-desktop').evaluate('el=>getComputedStyle(el).getPropertyValue("--surface").trim()')
            p.locator('.desktop-window.active .window-close').click();enter('Files')
            expect(p.locator('.desktop-window-host')).to_have_count(0)
            assert p.locator('.file-pane').evaluate('el=>getComputedStyle(el).backgroundColor') not in ['rgb(255, 255, 255)','rgb(245, 247, 251)']
            app.screenshot('desktop-only-dark.png')
            checks.append('A desktop Settings window centralizes keyboard reference and remote preferences; desktop theme stays local to the desktop')

            enter('Monitor')
            sections=p.locator('.monitor-sidebar')
            expect(sections.locator('.monitor-section-label')).to_have_text(['Overview','CPU','Memory','Disks','Network','Processes','Services','Logs'])
            expect(p.locator('.monitor-sampling-interval')).to_have_text('Every 10 s')
            monitor_tools = p.locator('.monitor-window-tools')
            expect(monitor_tools.locator('.help-tip, .help-copy')).to_have_count(0)
            expect(monitor_tools.get_by_role('button',name='Refresh resource values',exact=True)).to_be_visible()
            expect(p.locator('.desktop-dock').get_by_role('button',name='Disks',exact=True)).to_have_count(0)
            expect(p.locator('.resource-trend-chart')).to_have_count(3)
            sections.get_by_role('button',name='CPU',exact=True).click()
            expect(p.locator('.resource-trend-chart')).to_have_count(1)
            expect(p.locator('.resource-trend-cpu')).to_be_visible()
            expect(p.locator('.monitor-content')).to_contain_text('logical processors')
            sections.get_by_role('button',name='Memory',exact=True).click()
            expect(p.locator('.resource-trend-memory')).to_be_visible()
            expect(p.locator('.monitor-metric-grid')).to_contain_text('Swap')
            sections.get_by_role('button',name='Network',exact=True).click()
            expect(p.locator('.resource-trend-network')).to_be_visible()
            expect(p.locator('.resource-table')).to_contain_text('eth0')
            network = sections.get_by_role('button',name='Network',exact=True)
            expect(network.locator('.monitor-section-summary')).to_contain_text('KB/s', timeout=15000)
            p.locator('.desktop-window.app-monitor .window-maximize').click()
            sidebar_widths = []
            for width, height in [(1440, 980), (1100, 780), (900, 660)]:
                p.set_viewport_size(dict(width=width, height=height))
                sidebar_widths.append(sections.evaluate('el => el.offsetWidth'))
                assert network.evaluate('''button => {
                    const icon = button.querySelector('.action-glyph').getBoundingClientRect();
                    const title = button.querySelector('.monitor-section-label').getBoundingClientRect();
                    const detail = button.querySelector('.monitor-section-summary');
                    const line = detail.getBoundingClientRect();
                    const range = document.createRange();
                    range.selectNodeContents(detail);
                    return Math.abs((icon.top + icon.bottom) - (title.top + line.bottom)) < 3
                        && Math.abs(range.getBoundingClientRect().left - title.left) < 2 && line.top >= title.bottom - 1
                        && detail.scrollWidth <= detail.clientWidth && detail.scrollHeight <= detail.clientHeight;
                }'''), width
            assert 206 <= sidebar_widths[2] < sidebar_widths[1] < sidebar_widths[0] <= 216, sidebar_widths
            p.set_viewport_size(dict(width=1440, height=980))
            sections.get_by_role('button',name='Processes',exact=True).click()
            processes = p.locator('.monitor-process-inventory')
            table = processes.locator('.monitor-process-table')
            header = table.locator('.monitor-process-header')
            rows = table.locator('.monitor-process-row:not(.monitor-process-header)')
            viewport = table.get_by_role('list', name='Processes')
            expect(rows.first).to_contain_text('fixture-batch worker')
            expect(rows.first).to_contain_text('13.5%')
            expect(rows.first).to_contain_text('4.2 MB')
            expect(rows.first.locator('.monitor-process-ports')).to_have_text('')
            expect(rows.filter(has_text='fixture-web').locator('.monitor-process-ports')).to_have_text('TCP 8080')
            query = processes.get_by_role('textbox',name='Filter by program, PID, UID or port')
            expect(query).to_have_attribute('placeholder', 'Filter processes')
            toolbar = processes.locator('.monitor-process-toolbar')
            controls = [query, toolbar.get_by_role('button',name='Port details'), toolbar.get_by_role('button',name='Refresh processes')]
            expect(toolbar.get_by_role('button', name=re.compile('Next page|Previous page'))).to_have_count(0)
            centers = [box['y'] + box['height'] / 2 for box in (control.bounding_box() for control in controls)]
            assert max(centers) - min(centers) < 2, centers
            expect(processes).not_to_contain_text(re.compile(r'\d+ processes · \d+ matching'))
            header_y = header.bounding_box()['y']
            assert 0 < rows.count() < 80
            viewport.evaluate('el => { el.scrollTop = el.scrollHeight / 2; }')
            expect(rows.filter(has_text='fixture-idle-2800')).to_have_count(1)
            assert rows.count() < 80
            viewport.evaluate('el => { el.scrollTop = el.scrollHeight; }')
            expect(rows.last).to_contain_text('fixture-idle-5299')
            assert rows.count() < 80
            assert abs(header.bounding_box()['y'] - header_y) < 1
            assert p.locator('.monitor-content').evaluate('el => el.scrollTop') == 0
            query.fill('absent-process')
            expect(processes).to_contain_text('No matching processes')
            query.fill('8080')
            expect(rows).to_have_count(1)
            expect(rows.first).to_contain_text('fixture-web')
            query.fill('batch')
            expect(rows).to_have_count(1)
            expect(rows.first).to_contain_text('202')
            unavailable = ubuntu.root / 'processes-unavailable'
            unavailable.touch()
            refresh_processes = processes.get_by_role('button',name='Refresh processes',exact=True)
            refresh_processes.click()
            expect(processes).to_contain_text('Showing the previous process snapshot')
            expect(processes).to_contain_text('ps is unavailable')
            unavailable.unlink()
            refresh_processes.click()
            expect(processes.locator('.monitor-process-feedback')).to_have_count(0)
            p.set_viewport_size(dict(width=900,height=660))
            assert p.locator('.monitor-content').evaluate('el=>el.scrollWidth <= el.clientWidth + 1')
            p.set_viewport_size(dict(width=1440,height=980))
            query.fill('')
            expect(rows.first).to_contain_text('fixture-batch worker')
            assert viewport.evaluate('el => el.scrollTop') == 0
            app.screenshot('monitor-processes.png')
            query.fill('batch')
            processes.get_by_role('button',name='Port details',exact=True).click()
            ports = p.locator('.resource-ports')
            expect(ports.locator('.resource-ports-table')).to_contain_text('fixture-web')
            expect(ports.locator('.resource-ports-table')).to_contain_text('Process information unavailable')
            expect(ports).to_have_css('border-top-width', '0px')
            p.get_by_role('button',name='Back to processes',exact=True).click()
            expect(query).to_have_value('batch')
            sections.get_by_role('button',name='Disks',exact=True).click()
            expect(p.locator('.disk-device-row')).to_have_count(4)
            expect(p.locator('.disk-mount-list')).to_contain_text('224')
            p.set_viewport_size(dict(width=900,height=660))
            metrics=p.locator('.monitor-content')
            assert metrics.evaluate('el=>el.scrollWidth <= el.clientWidth + 1')
            p.set_viewport_size(dict(width=1440,height=980))
            p.locator('.disk-mount-list').get_by_role('button',name='/data',exact=True).click()
            expect(app.file('disk-file.txt')).to_be_visible()
            enter('Monitor')
            expect(sections.get_by_role('button',name='Disks',exact=True)).to_have_attribute('aria-pressed','true')
            checks.append('Monitor lists all visible processes, matches listeners to owning processes, and keeps a port detail view for unassigned listeners')

            enter('Services');expect(p.locator('.service-row')).to_have_count(4)
            p.get_by_role('button',name='nginx.service',exact=True).click()
            p.locator('.operations-toolbar').get_by_role('button',name='Logs',exact=True).click()
            expect(sections.get_by_role('button',name='Logs',exact=True)).to_have_attribute('aria-pressed','true')
            expect(p.get_by_role('textbox',name='Log service unit',exact=True)).to_have_value('nginx.service')
            expect(p.locator('.system-log-output')).to_contain_text('Fixture service warning')
            expect(p.locator('.desktop-window.active .desktop-window-title')).to_have_text('Monitor')
            enter('Services')
            expect(p.get_by_role('button',name='nginx.service',exact=True)).to_have_class(re.compile(r'\bselected\b'))
            p.get_by_role('button',name='Start at boot',exact=True).click();expect(p.locator('.service-row')).to_have_count(1)
            p.get_by_role('button',name='All services',exact=True).click()
            p.get_by_role('button',name='worker.service',exact=True).click()
            p.get_by_role('button',name='Enable at boot…',exact=True).click()
            d=p.get_by_role('dialog',name='Run script on servers',exact=True)
            p.set_viewport_size(dict(width=900,height=650))
            fits_desktop(p.locator('.desktop-modal-backdrop .modal-scrim'))
            fits_desktop(d)
            fits_desktop(d.locator('.modal-footer'))
            body=d.locator('.modal-body')
            assert body.evaluate('el=>el.scrollHeight > el.clientHeight')
            body.evaluate('el=>el.scrollTop=el.scrollHeight')
            fits_desktop(d.get_by_role('button',name='Check targets',exact=True))
            app.screenshot('desktop-contained-service-dialog.png')
            p.set_viewport_size(dict(width=1440,height=980))
            fits_desktop(d)
            finish_task()
            app.servers();enter('Services');p.get_by_role('button',name='Refresh services',exact=True).click()
            expect(p.get_by_role('button',name='worker.service',exact=True)).to_contain_text('enabled')
            expect(p.get_by_role('button',name='worker.service',exact=True)).to_contain_text('inactive')
            p.get_by_role('button',name='New startup task',exact=True).click();d=p.get_by_role('dialog',name='New startup task',exact=True)
            d.get_by_role('textbox',name='Task name',exact=True).fill('demo-worker')
            d.get_by_role('textbox',name='Startup command',exact=True).fill('echo startup-ready')
            d.get_by_role('button',name='Review startup task',exact=True).click();finish_task()
            app.servers();enter('Services');p.get_by_role('button',name='Refresh services',exact=True).click()
            expect(p.get_by_role('button',name='arcnix-startup-demo-worker.service',exact=True)).to_contain_text('enabled')
            app.screenshot('services.png')
            checks.append('Services separates activity and boot enablement, includes unloaded units, and creates reviewed boot tasks without starting them implicitly')
            checks.append('Services link to filtered logs within Monitor and retain the selected unit on return. Task dialogs and scrims stay within the resized desktop')

            enter('Docker');p.locator('.docker-toolbar').get_by_role('button',name='Install software',exact=True).click()
            expect(p.locator('.app-center-content')).to_be_visible()
            p.locator('.app-center-app-card').filter(has=p.get_by_role('button',name='Nginx',exact=True)).get_by_role('button',name='Install…',exact=True).click()
            d=p.get_by_role('dialog',name='Install software',exact=True)
            expect(d.get_by_role('button',name='Docker Compose',exact=True)).to_have_attribute('aria-pressed','true')
            d.get_by_role('textbox',name='Project name',exact=True).fill('nginx-demo')
            d.get_by_role('textbox',name='Host port',exact=True).fill('18080')
            d.get_by_role('button',name='Preview Compose',exact=True).click();expect(d.locator('.compose-preview')).to_contain_text('restart: unless-stopped')
            assert d.locator('.compose-preview').evaluate('el=>getComputedStyle(el).color')==d.get_by_role('button',name='Preview Compose',exact=True).evaluate('el=>getComputedStyle(el).color')
            desktop_text = p.locator('.server-desktop').evaluate('el => getComputedStyle(el).color')
            expect(d.locator('.compose-preview')).to_have_css('color', desktop_text)
            expect(d.locator('.compose-install-path')).to_have_text('~/.arcnix/apps/nginx-demo/compose.yaml')
            app.screenshot('unified-compose-install.png')
            d.get_by_role('button',name='Install',exact=True).click();finish_task(review=False)
            manifest=ubuntu.root/'home/arcnix/.arcnix/apps/nginx-demo/arcnix.json'
            assert manifest.exists()
            app.servers();enter('Docker');p.locator('.docker-tabs').get_by_role('button',name='Projects',exact=True).click()
            expect(p.locator('.compose-project-list')).to_contain_text('nginx-demo')
            p.get_by_role('button',name='Compose file',exact=True).click();expect(p.locator('.compose-preview')).to_contain_text('18080:80')
            p.locator('.compose-actions').get_by_role('button',name='Open folder',exact=True).click()
            expect(app.file('compose.yaml')).to_be_visible()
            checks.append('Docker and App Center share one installer; Compose definitions persist under the SSH home and projects support review/apply, lifecycle and Files access')

            enter('Terminal');reader=p.locator('.terminal-surface .xterm-accessibility-tree');keyboard=p.locator('.terminal-surface .xterm-helper-textarea')
            expect(p.locator('.terminal-status')).to_have_text('Interactive SSH')
            keyboard.focus();p.keyboard.type('cd /work');p.keyboard.press('Enter');p.keyboard.type('echo SESSION_SURVIVES_CLIENT');p.keyboard.press('Enter');expect(reader).to_contain_text('SESSION_SURVIVES_CLIENT')
            p.locator('.terminal-overflow').click();fits_desktop(p.locator('.terminal-context-menu'));p.keyboard.press('Escape');expect(p.locator('.terminal-context-menu')).to_have_count(0)
            title=p.locator('.desktop-window.active .desktop-window-titlebar').bounding_box();surface=p.locator('.terminal-surface').bounding_box();assert surface['y']-title['y']<=60,(title,surface)
            p.locator('.desktop-window.active .window-maximize').click()
            prefs=ubuntu.root/'home/arcnix/.arcnix/desktop.json'
            eventually(lambda:read_desktop_settings(prefs).get('terminal',{}).get('maximized') is True)
            app.screenshot('compact-terminal.png')
            saved=read_desktop_settings(prefs);assert 'SESSION_SURVIVES_CLIENT' not in json.dumps(saved)
            app.stop();app.launch();p=app.page;p.on('pageerror',lambda e:errors.append(str(e)))
            app.connect();enter('Terminal');reader=p.locator('.terminal-surface .xterm-accessibility-tree');keyboard=p.locator('.terminal-surface .xterm-helper-textarea')
            expect(reader).to_contain_text('SESSION_SURVIVES_CLIENT')
            expect(p.locator('.desktop-window.active')).to_have_class(re.compile(r'\bmaximized\b'))
            keyboard.focus();p.keyboard.type('pwd');p.keyboard.press('Enter');expect(reader).to_contain_text('/work')
            assert read_desktop_settings(prefs)['terminal']['tabs'][0]['id']==saved['terminal']['tabs'][0]['id']
            enter('Monitor');expect(p.locator('.monitor-sampling-interval')).to_have_text('Every 10 s')
            checks.append('Terminal uses one window header, persists layout without output transcripts and reconnects to the same tmux session/cwd after client restart')

            p.locator('.host-select').nth(1).click();app.connect(1);enter('App Center')
            expect(p.locator('.app-center-app-card')).to_have_count(18)
            p.locator('.app-center-app-card').filter(has=p.get_by_role('button',name='Nginx',exact=True)).get_by_role('button',name='Install…',exact=True).click()
            d=p.get_by_role('dialog',name='Install software',exact=True);expect(d.locator('.install-provider')).to_contain_text('DNF')
            expect(d.locator('.install-package-name')).to_have_text('nginx')
            d.get_by_role('button',name='Install',exact=True).click();finish_task(review=False)
            p.locator('.app-center-navigation').get_by_role('button',name='Installed',exact=True).click()
            expect(p.locator('.app-center-inventory-list')).to_contain_text('nginx')
            p.locator('.app-center-navigation').get_by_role('button',name='Discover',exact=True).click()
            p.locator('.app-center-app-card').filter(has=p.get_by_role('button',name='htop',exact=True)).get_by_role('button',name='Install…',exact=True).click()
            d=p.get_by_role('dialog',name='Install software',exact=True);d.get_by_role('button',name='Snap',exact=True).click()
            expect(d.get_by_role('checkbox',name='Allow classic confinement',exact=True)).not_to_be_checked()
            d.get_by_role('button',name='Install',exact=True).click();finish_task(review=False)
            p.locator('.app-center-navigation').get_by_role('button',name='Installed',exact=True).click()
            expect(p.locator('.app-center-inventory-list')).to_contain_text('htop')
            snap_row=p.locator('.app-center-installed-row').filter(has_text='htop')
            snap_row.get_by_role('button',name='Upgrade…',exact=True).click()
            p.get_by_role('dialog',name='Upgrade software',exact=True).get_by_role('button',name='Upgrade',exact=True).click()
            finish_task(review=False)
            snap_row.get_by_role('button',name='Remove…',exact=True).click()
            p.get_by_role('dialog',name='Remove software',exact=True).get_by_role('button',name='Remove',exact=True).click()
            finish_task(review=False)
            expect(snap_row).to_have_count(0)
            checks.append('Application cards route Fedora native installation to DNF and Snap keeps its own inventory/channel and explicit confinement choice')
            assert not errors,errors
    (out/'results.json').write_text(json.dumps(checks,indent=2))
    print(json.dumps(dict(output=str(out),checks=checks),indent=2))

if __name__=='__main__':main()
