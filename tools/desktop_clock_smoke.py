"""Desktop clocks follow the connected server across time zones and Dock modes."""
from datetime import datetime, timezone
import json
import re
import time

from desktop_fixture import Desktop, Peer, ROOT, expect


def set_clock(peer, epoch, offset, dock):
    (peer.root / 'desktop-clock.json').write_text(json.dumps({
        'unix_seconds': epoch, 'offset': offset,
    }), encoding='utf-8')
    settings = peer.root / 'home/arcnix/.arcnix/desktop.json'
    settings.parent.mkdir(parents=True, exist_ok=True)
    settings.write_text(json.dumps({
        'version': 1, 'theme': 'System', 'appearance': {'dock': dock},
    }), encoding='utf-8')


def main():
    out = ROOT / 'artifacts/desktop-clock' / f'run-{time.time_ns()}'
    out.mkdir(parents=True)
    with Peer(out / 'east') as east, Peer(out / 'west') as west:
        epoch = int(datetime(2026, 11, 17, 22, 17, tzinfo=timezone.utc).timestamp())
        set_clock(east, epoch, '+0545', {'panel_mode': True})
        set_clock(west, epoch, '-0700', {'floating_status': True})
        config = east.config([east.profile('Eastern test server'), west.profile('Western test server')], out)
        config['config']['trusted_hosts'].extend(west.config([], out)['config']['trusted_hosts'])
        with Desktop(out / 'desktop', config) as app:
            page = app.page
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            cdp = page.context.new_cdp_session(page)
            cdp.send('Emulation.setTimezoneOverride', {'timezoneId': 'America/Los_Angeles'})
            assert page.evaluate('Intl.DateTimeFormat().resolvedOptions().timeZone') == 'America/Los_Angeles'

            app.connect(0)
            app.workspace('Desktop')
            clock = page.locator('.dock-panel-zone .desktop-panel-clock')
            expect(clock).to_have_attribute('title', 'Server time (UTC+05:45)')
            expect(clock.locator('.desktop-panel-date')).to_have_text('11/18/2026')
            display = clock.locator('.desktop-panel-time')
            expect(display).to_have_text(re.compile(r'^4:02:[0-5]\d\s+AM$'))
            expect(display).not_to_have_text(display.inner_text(), timeout=3000)
            log = (east.root / 'server-app-queries.jsonl').read_text(encoding='utf-8')
            assert log.count('# arcnix:desktop:clock') == 1

            page.get_by_role('button', name='Disconnect', exact=True).click()
            app.connect(1)
            app.workspace('Desktop')
            clock = page.locator('.desktop-dock-status .desktop-panel-clock')
            expect(clock).to_have_attribute('title', 'Server time (UTC-07:00)')
            expect(clock.locator('.desktop-panel-date')).to_have_text('11/17/2026')
            expect(clock.locator('.desktop-panel-time')).to_have_text(re.compile(r'^3:17:[0-5]\d\s+PM$'))
            assert not errors, errors
            print('Both Dock modes show the connected server clock, including date rollover and seconds.')


if __name__ == '__main__':
    main()
