"""Exercise reversible and permanent deletion on an owned SFTP peer and local files."""
import argparse
import json
import re
import subprocess
import time
from urllib.parse import quote, unquote

from desktop_fixture import Desktop, Peer, ROOT, expect
import paramiko


def external_record(ubuntu_gio):
    if not ubuntu_gio:
        original = '/from Ubuntu.txt'
        return 'native entry', f'[Trash Info]\nPath={quote(original)}\nDeletionDate=2026-01-02T03:04:05\n', 'external original'
    # Export an unchanged record and payload from an isolated Ubuntu GIO home Trash.
    script = """
import json, os, subprocess, tempfile
from pathlib import Path
with tempfile.TemporaryDirectory() as temp:
    xdg = Path(temp) / 'xdg'
    original = Path(temp) / 'from Ubuntu #%.txt'
    original.write_text('external original')
    subprocess.run(['gio', 'trash', str(original)], env=dict(os.environ, XDG_DATA_HOME=str(xdg)), check=True)
    info, = (xdg / 'Trash/info').glob('*.trashinfo')
    payload = xdg / 'Trash/files' / info.stem
    print(json.dumps([info.stem, info.read_text(), payload.read_text()]))
"""
    return json.loads(subprocess.check_output(
        ['wsl', '-d', 'Ubuntu', '--', 'python3', '-'], input=script, text=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ubuntu-gio', action='store_true', help='Generate the external record with Ubuntu GIO under WSL')
    args = parser.parse_args()
    out = ROOT / 'artifacts/trash' / str(time.time_ns())
    local = out / 'local'
    local.mkdir(parents=True)
    local_file = local / f'local-{out.name}.txt'
    local_file.write_bytes(b'local original')
    with Peer(out / 'peer') as peer:
        folder = peer.root / '测试 # %'
        folder.mkdir()
        (folder / 'child.txt').write_bytes(b'folder contents')
        denied = peer.root / 'fixture-trash-denied.txt'
        denied.write_bytes(b'keep original')
        volume = peer.root / 'volume'
        volume.mkdir()
        mounted_file = volume / 'sample.txt'
        mounted_file.write_bytes(b'on the mounted disk')
        mountinfo = peer.root / 'proc/self/mountinfo'
        mountinfo.parent.mkdir(parents=True)
        mountinfo.write_text('1 0 8:1 / / rw - ext4 /dev/root rw\n'
                             '2 1 8:2 / /volume rw - ext4 /dev/data rw\n')
        desktop = peer.root / 'home/arcnix/Desktop'
        desktop.mkdir(parents=True, exist_ok=True)
        desktop_file = desktop / 'desktop.txt'
        desktop_file.write_bytes(b'desktop original')
        with Desktop(out / 'app', peer.config([peer.profile('Trash test')], local)) as app:
            p = app.page
            app.connect()
            def item(panel, path):
                return panel.locator('.trash-item').filter(has=p.get_by_text(str(path), exact=True))

            def open_bin(remote):
                pane = p.locator('.remote-slot:visible' if remote else '.local-slot:visible')
                pane.get_by_role('button', name='Remote file actions' if remote else 'Local file actions', exact=True).click()
                p.locator('.file-actions-menu:visible').get_by_role('button', name='Open Trash', exact=True).click()
                return p.get_by_role('dialog', name='Remote Trash' if remote else 'Local Trash', exact=True)

            def purge(panel, row):
                row.get_by_role('button', name='Delete permanently', exact=True).click()
                review = panel.locator('.trash-confirm')
                expect(review).to_contain_text('cannot be undone')
                review.get_by_role('button', name='Cancel', exact=True).click()
                expect(row).to_be_visible()
                row.get_by_role('button', name='Delete permanently', exact=True).click()
                review.get_by_role('button', name='Delete permanently', exact=True).click()
                expect(row).to_have_count(0)

            entry = app.file(folder.name)
            entry.click(button='right')
            menu = p.locator('.file-context-menu')
            expect(menu.get_by_role('button', name='Delete', exact=True)).to_have_count(0)
            expect(menu.get_by_role('button', name='Delete permanently', exact=True)).to_be_enabled()
            app.action('Move to Trash')
            expect(entry).to_have_count(0)
            bin_dir = peer.root / '.local/share/Trash'
            info = next((bin_dir/'info').glob('*.trashinfo'))
            record = dict(line.split('=', 1) for line in info.read_text().splitlines()[1:])
            assert unquote(record['Path']) == '/' + folder.name and '%25' in record['Path']
            assert re.fullmatch(r'\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d', record['DeletionDate'])
            assert (bin_dir/'files'/info.stem/'child.txt').read_bytes() == b'folder contents'
            panel = open_bin(True)
            row = item(panel, '/' + folder.name)
            folder.mkdir()
            row.get_by_role('button', name='Restore', exact=True).click()
            expect(panel.locator('.trash-error')).to_contain_text('exists')
            assert info.is_file() and not list(folder.iterdir())
            folder.rmdir()
            row.get_by_role('button', name='Restore', exact=True).click()
            expect(row).to_have_count(0)
            assert (folder/'child.txt').read_bytes() == b'folder contents' and not info.exists()
            panel.get_by_role('button', name='Close', exact=True).click()

            refused = app.file(denied.name)
            refused.click()
            refused.press('Delete')
            expect(p.locator('.notice')).to_contain_text('Cannot move item')
            assert denied.read_bytes() == b'keep original'
            assert not list((bin_dir / 'info').iterdir())

            # Separate mount, non-sticky shared .Trash: use the private per-UID directory.
            (volume / '.Trash').mkdir()
            app.file('volume').dblclick()
            mounted = app.file('sample.txt')
            mounted.click()
            mounted.press('Delete')
            expect(mounted).to_have_count(0)
            volume_bin = volume / f'.Trash-{peer.root.stat().st_uid}'
            volume_info = next((volume_bin / 'info').glob('*.trashinfo'))
            assert 'Path=sample.txt' in volume_info.read_text().splitlines()
            assert not list((volume / '.Trash').iterdir())
            panel = open_bin(True)
            row = item(panel, '/volume/sample.txt')
            row.get_by_role('button', name='Restore', exact=True).click()
            expect(row).to_have_count(0)
            assert mounted_file.read_bytes() == b'on the mounted disk' and not volume_info.exists()
            panel.get_by_role('button', name='Close', exact=True).click()

            # With a sticky .Trash, keep both generations even when their original paths match.
            (volume / '.Trash').rmdir()
            profile = peer.profile('Trash test')
            key = paramiko.Ed25519Key.from_private_key_file(profile['private_key'])
            with paramiko.Transport((profile['host'], profile['port'])) as transport:
                transport.connect(username=profile['username'], pkey=key)
                with paramiko.SFTPClient.from_transport(transport) as sftp:
                    sftp.mkdir('/volume/.Trash', mode=0o1777)
            for contents in [b'first', b'second']:
                mounted_file.write_bytes(contents)
                # Re-read the source after changing its size and modification time.
                p.locator('.remote-slot:visible').get_by_role('button', name='Refresh remote directory', exact=True).click()
                mounted.click()
                mounted.press('Delete')
                expect(mounted).to_have_count(0)
            shared_bin = volume / '.Trash' / str(peer.root.stat().st_uid)
            assert len(list((shared_bin / 'info').glob('*.trashinfo'))) == 2
            assert {f.read_bytes() for f in (shared_bin / 'files').iterdir()} == {b'first', b'second'}
            panel = open_bin(True)
            versions = item(panel, '/volume/sample.txt')
            expect(versions).to_have_count(2)
            versions.first.get_by_role('button', name='Restore', exact=True).click()
            expect(versions).to_have_count(1)
            purge(panel, versions)
            assert mounted_file.read_bytes() in {b'first', b'second'}
            assert not list((shared_bin / 'info').iterdir())
            panel.get_by_role('button', name='Close', exact=True).click()

            # Read a record produced by another FreeDesktop implementation.
            external, metadata, contents = external_record(args.ubuntu_gio)
            fields = dict(line.split('=', 1) for line in metadata.splitlines()[1:])
            original = unquote(fields['Path'])
            restored = peer.root / original.lstrip('/')
            restored.parent.mkdir(parents=True, exist_ok=True)
            (bin_dir / 'files' / external).write_text(contents)
            (bin_dir / 'info' / (external + '.trashinfo')).write_text(metadata)
            # With no shell, the SFTP login directory still locates the home Trash.
            restricted = peer.root / 'fixture-sftp-only'
            restricted.touch()
            app.workspace('Desktop')
            p.locator('.desktop-shortcuts').get_by_role('button', name='Trash', exact=True).click()
            panel = p.locator('.desktop-window.active.app-trash')
            row = item(panel, original)
            expect(row).to_contain_text(fields['DeletionDate'].replace('T', ' '))
            row.get_by_role('button', name='Restore', exact=True).click()
            expect(row).to_have_count(0)
            assert restored.read_text() == contents
            restricted.unlink()
            panel.locator('.window-close').click()

            shortcut = p.locator('.desktop-shortcuts').get_by_role('button', name='desktop.txt', exact=True)
            shortcut.click(button='right')
            p.locator('.desktop-context-menu').get_by_role('button', name='Move to Trash', exact=True).click()
            expect(shortcut).to_have_count(0)
            app.application('Trash')
            panel = p.locator('.desktop-window.active.app-trash')
            purge(panel, item(panel, '/home/arcnix/Desktop/desktop.txt'))
            assert not desktop_file.exists()

            app.workspace('Transfer files')
            local_entry = app.file(local_file.name, remote=False)
            local_entry.click()
            local_entry.press('Delete')
            expect(local_entry).to_have_count(0)
            panel = open_bin(False)
            row = item(panel, local_file)
            row.get_by_role('button', name='Restore', exact=True).click()
            expect(row).to_have_count(0)
            assert local_file.read_bytes() == b'local original'
            panel.get_by_role('button', name='Close', exact=True).click()
            local_entry.click(button='right')
            app.action('Move to Trash')
            expect(local_entry).to_have_count(0)
            panel = open_bin(False)
            purge(panel, item(panel, local_file))
            assert not local_file.exists()
    print('Passed native local Trash, FreeDesktop SFTP interoperability, desktop and transfer actions, restore conflicts and confirmed permanent removal.')


if __name__ == '__main__':
    main()
