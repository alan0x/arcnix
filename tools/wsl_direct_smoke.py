"""Check a saved WSL profile, built-in terminal and files without SSH credentials."""
import argparse
import json
import re
import subprocess
import tempfile
from pathlib import Path

from desktop_fixture import Desktop, eventually, expect


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--distribution', required=True)
    args = parser.parse_args()
    distro = args.distribution
    def linux(*command):
        return subprocess.check_output(['wsl.exe', '--distribution', distro, '--cd', '/', '--exec', *command])
    user = linux('id', '-un').decode().strip()
    scratch = linux('mktemp', '-d', '/tmp/arcnix-direct-XXXXXXXX').decode().strip()
    assert re.fullmatch(r'/tmp/arcnix-direct-[A-Za-z0-9]+', scratch)
    root = Path(tempfile.mkdtemp(prefix='wsl-direct-', dir=Path(__file__).resolve().parents[1] / 'artifacts'))
    profile = dict(id='local-wsl', name='WSL fixture', wsl_distribution=distro,
                   host=distro+'.wsl.localhost', port=1, username=user, auth='Password', initial_remote_path='.')
    cfg = dict(version=1, config=dict(groups=[], servers=[profile], trusted_hosts=[], local_directory=str(root)))
    try:
        linux('touch','--',scratch+'/directory-ready')
        with Desktop(root, cfg) as app:
            p = app.page
            def button(name):
                return p.get_by_role('button', name=name, exact=True)
            p.locator('.host-select').click()
            button('Connect').click()
            expect(p.locator('.wsl-connected .connection-state')).to_have_text('WSL · '+user)
            expect(p.get_by_role('dialog')).to_have_count(0)
            path = p.get_by_role('textbox', name='Linux directory')
            path.fill(scratch)
            button('Open directory').click()
            expect(button('directory-ready')).to_be_visible()
            entry = p.get_by_role('textbox', name='New entry name')
            entry.fill("space ' directory")
            button('New folder').click()
            button("space ' directory").click()
            expect(path).to_have_value(scratch+"/space ' directory")
            expect(button('Open directory')).to_be_enabled()
            entry.fill('example.txt')
            button('New file').click()
            button('example.txt').click()
            button('Terminal').click()
            screen = p.locator('.wsl-terminal .terminal-surface')
            expect(screen.locator('.xterm-accessibility-tree')).to_contain_text("space ' directory", timeout=15000)
            keyboard = screen.locator('.xterm-helper-textarea')
            keyboard.focus()
            p.keyboard.type("printf '%s\\n' Arcnix-WSL > example.txt; printf 'user=%s tty=%s\\n' \"$(id -un)\" \"$(test -t 0 && echo yes)\"")
            p.keyboard.press('Enter')
            expect(screen.locator('.xterm-accessibility-tree')).to_contain_text('user='+user+' tty=yes', timeout=15000)
            def send(line):
                keyboard.focus()
                p.keyboard.type(line)
                p.keyboard.press('Enter')
            def size(report):
                send('stty size > '+report)
                saved = scratch+"/space ' directory/"+report
                def read():
                    value = subprocess.run(['wsl.exe','-d',distro,'--cd','/','--exec','cat',saved],
                                           capture_output=True).stdout.strip()
                    return value if re.fullmatch(rb'[1-9][0-9]* [1-9][0-9]*',value) else None
                return eventually(read)
            old_size = size('before-size')
            p.set_viewport_size(dict(width=1050,height=720))
            expect(screen.locator('.xterm-screen')).to_be_visible()
            assert size('after-size') != old_size
            send('printf "waiting-%s\\n" interrupt; sleep 30')
            expect(screen.locator('.xterm-accessibility-tree')).to_contain_text('waiting-interrupt')
            p.keyboard.press('Control+c')
            send('printf "resumed-%s\\n" interrupt')
            expect(screen.locator('.xterm-accessibility-tree')).to_contain_text('resumed-interrupt')
            send("printf 'shell-%s\\n' finished; exit")
            expect(screen.locator('.xterm-accessibility-tree')).to_contain_text('shell-finished')
            expect(p.locator('.wsl-terminal .terminal-status')).to_have_text('Terminal closed', timeout=10000)
            button('Files').click()
            button('Preview text').click()
            expect(p.locator('.wsl-preview')).to_contain_text('Arcnix-WSL')
            button('Disconnect').click()
            expect(p.locator('.wsl-connected')).to_have_count(0)
            button('Connect').click()
            expect(p.locator('.wsl-connected')).to_be_visible()
            assert not json.loads(app.config.read_text(encoding='utf-8'))['config']['trusted_hosts']
            p.locator('.tree-new-server').click()
            dialog = p.get_by_role('dialog', name='New server')
            expect(dialog.get_by_role('textbox', name='Name', exact=True)).to_be_visible()
            expect(dialog.get_by_role('button', name='Choose local WSL distribution')).to_have_count(0)
            dialog.get_by_role('button', name='Cancel').click()
            p.locator('.host-select').hover()
            p.locator('.host-more').click()
            button('Edit server').click()
            transport = p.get_by_role('radiogroup', name='WSL connection method', exact=True)
            direct = transport.get_by_role('radio', name='Local connection', exact=True)
            expect(direct).to_be_checked()
            expect(p.get_by_role('textbox',name='Port',exact=True)).to_have_count(0)
            expect(p.get_by_role('textbox',name='Username',exact=True)).to_have_count(0)
            direct.press('ArrowRight')
            expect(transport.get_by_role('radio', name='SSH', exact=True)).to_be_checked()
            expect(p.get_by_role('textbox',name='Port',exact=True)).to_be_visible()
        cfg['config']['settings'] = dict(language='SimplifiedChinese', show_hidden_files=True, resource_interval_seconds=3)
        with Desktop(root / 'zh', cfg) as app:
            p = app.page
            p.locator('.tree-new-server').click()
            dialog = p.get_by_role('dialog', name='新建服务器')
            expect(dialog.get_by_role('textbox', name='名称', exact=True)).to_be_visible()
            expect(dialog.get_by_role('button', name='选择本机 WSL 发行版')).to_have_count(0)
    finally:
        linux('rm', '-rf', '--', scratch)


if __name__ == '__main__':
    main()
