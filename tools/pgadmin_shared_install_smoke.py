"""Check two server desktops against one isolated local pgAdmin installation."""
from contextlib import contextmanager
import json
import os
import time
import zipfile

from desktop_fixture import Desktop, Peer, ROOT, expect


@contextmanager
def hold_install_lock(base):
    base.mkdir(parents=True)
    with (base / 'install.lock').open('w+b') as lock:
        if os.name == 'nt':
            import msvcrt
            msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def main():
    out = ROOT / 'artifacts' / 'pgadmin-shared' / str(time.time_ns())
    out.mkdir(parents=True)
    base = out / 'apps' / 'pgadmin'
    package = out / 'fixture.zip'
    with zipfile.ZipFile(package, 'w') as zip_file:
        zip_file.writestr('component.json', json.dumps(dict(version='9.19', python='bin/python3', pgadmin='pgadmin4')))
        for entry in ['bin/python3', 'pgadmin4/pgAdmin4.py', 'pgadmin4/setup.py',
                      'pgadmin4/pgadmin/static/js/generated/app.bundle.js']:
            zip_file.writestr(entry, '')
    with Peer(out) as peer:
        profiles = [peer.profile(name) for name in ['Server A', 'Server B']]
        config = peer.config(profiles, out, updates=dict(automatic_checks=False))
        with Desktop(out, config, extra_env={'ARCNIX_PGADMIN_PACKAGE': str(package)}) as app:
            page = app.page
            panel = page.locator('.pgadmin-panel:visible')
            install = panel.get_by_role('button', name='Install pgAdmin', exact=True)
            cancel = panel.get_by_role('button', name='Cancel installation for all servers', exact=True)

            def select(name):
                page.locator('.host-select').filter(has_text=name).click()
                app.workspace('Desktop')
                if not panel.is_visible():
                    app.application('pgAdmin')
                expect(panel).to_be_visible()

            for index in range(2):
                app.connect(index)
                app.application('pgAdmin')
                expect(install).to_be_enabled()
            with hold_install_lock(base):
                select('Server A')
                install.click()
                expect(cancel).to_be_visible()
                select('Server B')
                expect(cancel).to_be_visible()
                expect(install).to_have_count(0)
                cancel.click()
                expect(install).to_be_enabled()
                select('Server A')
                expect(page.locator('.pgadmin-error:visible')).to_contain_text('Installation cancelled')
                install.click()
                expect(cancel).to_be_visible()
                page.locator('.desktop-window.active.app-pgadmin .window-close').click()
                select('Server B')
                expect(cancel).to_be_visible()
            expect(panel).to_contain_text('Connect to PostgreSQL', timeout=10000)
            package.unlink()
            select('Server A')
            expect(panel).to_contain_text('Connect to PostgreSQL')
            assert (base / '9.19' / 'component.json').is_file()
            assert not list(base.glob('.installing-*'))
    print('Passed: shared install, cancel, retry, closing the initiating window, and reuse on both servers.')


if __name__ == '__main__':
    main()
