"""Verify the Device menu against an SSH fixture that only records power requests."""
import json
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def main():
    out = ROOT / 'artifacts/device-power' / str(time.time_ns())
    with Peer(out / 'peer') as peer:
        profile = peer.profile('Power fixture')
        address = peer.data['address']

        def requests():
            lines = (peer.root / 'server-app-queries.jsonl').read_text(encoding='utf-8').splitlines()
            markers = [json.loads(line)['command'].splitlines()[0] for line in lines]
            return [m for m in markers if m in ('# arcnix:desktop:shutdown', '# arcnix:desktop:restart')]

        with Desktop(out / 'desktop', peer.config([profile], out)) as app:
            app.connect()
            app.application('Settings')
            page = app.page
            settings = page.locator('.desktop-settings-page')
            expect(settings.locator('.desktop-settings-card .section-title')).to_have_text([
                'Device', 'System information', 'Resource refresh interval', 'Server desktop'])
            power = settings.get_by_role('button', name='Power options', exact=True)

            power.hover()
            geometry = power.evaluate("""button => {
                const b = button.getBoundingClientRect();
                const i = button.querySelector('.glyph-power').getBoundingClientRect();
                return [b.width - b.height, (i.left+i.right-b.left-b.right)/2,
                        (i.top+i.bottom-b.top-b.bottom)/2];
            }""")
            assert all(abs(delta) < 1 for delta in geometry), geometry
            page.mouse.down()
            expect(power).to_be_focused()
            expect(power).to_have_css('outline-style', 'none')
            page.mouse.up()
            first = page.get_by_role('menuitem', name='Restart system…')
            expect(first).to_be_focused()
            expect(first).to_have_css('outline-style', 'none')
            page.keyboard.press('ArrowDown')
            second = page.get_by_role('menuitem', name='Shut down…')
            expect(second).to_be_focused()
            expect(second).to_have_css('outline-style', 'solid')
            page.keyboard.press('Escape')
            expect(power).to_be_focused()
            expect(power).to_have_css('outline-style', 'solid')
            page.keyboard.press('Shift+Tab')
            page.keyboard.press('Tab')
            expect(power).to_be_focused()
            expect(power).to_have_css('outline-style', 'solid')

            def choose(label):
                power.click()
                page.get_by_role('menuitem', name=label, exact=True).click()

            choose('Restart system…')
            restart = page.get_by_role('dialog', name='Restart system?', exact=True)
            expect(restart).to_contain_text(address)
            expect(restart).to_contain_text('disconnect SSH')
            restart.get_by_role('button', name='Cancel', exact=True).click()
            choose('Shut down…')
            shutdown = page.get_by_role('dialog', name='Shut down system?', exact=True)
            confirm = shutdown.get_by_role('button', name='Shut down', exact=True)
            field = shutdown.get_by_role('textbox')
            expect(confirm).to_be_disabled()
            expect(shutdown).to_contain_text(address)
            field.fill(address + '0')
            expect(confirm).to_be_disabled()
            field.fill(address)
            expect(confirm).to_be_enabled()
            shutdown.get_by_role('button', name='Cancel', exact=True).click()
            assert not requests()
            choose('Shut down…')
            expect(field).to_have_value('')
            expect(confirm).to_be_disabled()
            field.fill(address)
            confirm.click()
            eventually(lambda: requests() == ['# arcnix:desktop:shutdown'])
            choose('Restart system…')
            restart.get_by_role('button', name='Restart', exact=True).click()
            eventually(lambda: requests() == ['# arcnix:desktop:shutdown', '# arcnix:desktop:restart'])
    print('Passed General order, power menu, target confirmation, cancellation and SSH dispatch.')


if __name__ == '__main__':
    main()
