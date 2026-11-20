"""Format remote documents through Code's visible editor context menu."""
import json
import re
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def main():
    out = ROOT / 'artifacts/code-format' / str(time.time_ns())
    with Peer(out / 'peer') as peer:
        project = peer.root / 'project'
        project.mkdir()
        cases = {
            'main.js': ('const config={items:[1,2]};\n', 'const config = { items: [1, 2] };'),
            'types.ts': ('type Entry={label:string};\n', 'type Entry = { label: string };'),
            'data.json': ('{"large":12345678901234567889,"text":"你好"}\r\n', '12345678901234567889'),
            'config.jsonc': ('// keep comment\n{"value":1,}\n', '// keep comment'),
            'style.css': ('body{color:red}\n', 'color: red;'),
            'index.html': ('<!DOCTYPE html><html><body><p>Hi</p></body></html>\n', '<p>Hi</p>'),
        }
        for name, (text, _) in cases.items():
            (project / name).write_bytes(text.encode('utf-8'))
        (project / 'bad.json').write_text('{"broken":}', encoding='utf-8')
        profile = {**peer.profile('Code format'), 'initial_remote_path': '/project'}
        with Desktop(out / 'desktop', peer.config([profile], out)) as app:
            app.connect()
            app.application('Code')
            code = app.code()
            lines = code.locator('.monaco-editor:visible .view-lines').last
            editor = code.locator('.monaco-editor:visible textarea.inputarea').last
            dirty = code.locator('.tabs-container .tab.active.dirty')

            def format_file(name):
                code.get_by_role('treeitem', name=name, exact=True).dblclick()
                expect(lines.locator('.view-line').first).to_be_visible()
                lines.locator('.view-line').first.click(button='right', position={'x': 18, 'y': 8})
                action = code.get_by_role('menuitem', name=re.compile(r'^Format Document\b')).last
                expect(action).to_be_enabled()
                code.wait_for_timeout(160)
                action.click()

            for name, (original, expected) in cases.items():
                path = project / name
                format_file(name)
                expect(dirty).to_be_visible()
                expect(lines).to_contain_text(expected)
                if name in ('data.json', 'style.css', 'index.html'):
                    expect(lines.locator('.view-line')).not_to_have_count(1)
                assert path.read_bytes() == original.encode('utf-8')
                editor.press('Control+s')
                eventually(lambda: path.read_bytes() != original.encode('utf-8'))
                expect(dirty).to_have_count(0)
                if name == 'data.json':
                    assert b'\r\n' in path.read_bytes()
                    assert json.loads(path.read_bytes()) == json.loads(original)
                if name == 'config.jsonc':
                    assert path.read_text(encoding='utf-8').startswith('// keep comment\n')

            format_file('bad.json')
            expect(code.locator('.notification-toast').get_by_text(re.compile('Unable to format document'))).to_be_visible()
            expect(dirty).to_have_count(0)
            assert (project / 'bad.json').read_text() == '{"broken":}'
            print('Code format context menu, JS/TS/JSON/JSONC/CSS/HTML, save boundaries and syntax errors passed.')


if __name__ == '__main__':
    main()
