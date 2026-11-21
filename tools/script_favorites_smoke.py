"""Exercise persistent script references without changing execution ownership."""
import json
import time
from pathlib import Path

from desktop_fixture import Peer, Desktop, expect, eventually, ROOT

out = ROOT / 'artifacts/script-favorites-ui' / str(time.time_ns())
out.mkdir(parents=True)
checks = []


def passed(message):
    checks.append(message)
    print(message, flush=True)


def saved(app):
    return json.loads(app.config.read_text(encoding='utf-8'))['config']


def library(app, section='Favorites'):
    p = app.page
    p.locator('.rail-scripts').click()
    p.locator('.context-links').get_by_role('button', name=section, exact=True).click()
    return p


def row(p, name, personal=False):
    result = p.locator('.script-row').filter(has=p.get_by_text(name, exact=True))
    return result.filter(has=p.get_by_text('Shell', exact=True)) if personal else result


def launches(peer):
    path = peer.root / 'task-launches.jsonl'
    return path.read_bytes() if path.exists() else b''


with Peer(out/'alpha') as a, Peer(out/'beta') as b:
    config = a.config([a.profile('Alpha'), b.profile('Beta')], out,
                      updates=dict(automatic_checks=False, automatic_install=False))
    host, port = b.data['address'].rsplit(':', 1)
    config['config']['trusted_hosts'].append(dict(
        host=host, port=int(port), fingerprint=b.data['host_key_rsa_fingerprint']))
    config['config']['snippets'] = [dict(id='builtin:nginx', name='Install Nginx',
        description='Personal example', command="printf '%s\\n' {{message}}", administrator=False)]
    for peer in (a, b):
        (peer.root/'trace-task-queries').touch()
    with Desktop(out/'desktop', config) as app:
        p = library(app)
        expect(p.locator('.script-row')).to_have_count(0)
        expect(p.get_by_text('No favorite scripts yet', exact=True)).to_be_visible()
        before_auth = [Path(peer.data['auth_audit']).read_bytes() for peer in (a, b)]
        library(app, 'Official scripts')
        row(p, 'Install Nginx').get_by_role('button', name='Add to favorites', exact=True).click()
        expect(row(p, 'Install Nginx').get_by_role('button', name='Remove from favorites', exact=True)).to_be_visible()
        assert saved(app)['script_favorites'] == [dict(source='official', script='builtin:nginx')]
        assert not app.config.with_suffix('.tasks.json').exists()
        passed('Legacy configuration opens an empty Favorites view; an official star saves a local reference without SSH or task creation')

        library(app, 'My scripts')
        row(p, 'Install Nginx', True).get_by_role('button', name='Add to favorites', exact=True).click()
        library(app)
        expect(p.locator('.script-row')).to_have_count(2)
        expect(p.get_by_role('button', name='Edit', exact=True)).to_have_count(1)
        expect(p.get_by_role('button', name='Delete', exact=True)).to_have_count(1)
        assert saved(app)['script_favorites'] == [dict(source='official', script='builtin:nginx'), dict(source='user', script='builtin:nginx')]
        assert before_auth == [Path(peer.data['auth_audit']).read_bytes() for peer in (a, b)]
        passed('Official and personal scripts with the same ID and name remain distinct; only the personal entry offers edit/delete')

        row(p, 'Install Nginx', True).get_by_role('button', name='Edit', exact=True).click()
        d = p.get_by_role('dialog', name='Save command', exact=True)
        d.get_by_role('textbox', name='Command name', exact=True).fill('Personal probe')
        d.get_by_role('textbox', name='Shell command', exact=True).fill("printf 'edited %s\\n' {{message}}")
        d.get_by_role('button', name='Save command', exact=True).click()
        expect(row(p, 'Personal probe').get_by_role('button', name='Remove from favorites', exact=True)).to_be_visible()
        row(p, 'Personal probe').get_by_role('button', name='Copy to my scripts', exact=True).click()
        d = p.get_by_role('dialog', name='Save command', exact=True)
        d.get_by_role('textbox', name='Command name', exact=True).fill('Personal copy')
        d.get_by_role('button', name='Save command', exact=True).click()
        expect(p.locator('.script-row')).to_have_count(2)
        library(app, 'My scripts')
        expect(row(p, 'Personal copy').get_by_role('button', name='Add to favorites', exact=True)).to_be_visible()
        passed('Renaming and editing retain the original favorite; a copied script receives a separate identity without inheriting it')

        incoming = out/'replacement.json'
        incoming.write_text(json.dumps(dict(kind='arcnix-scripts', version=1, scripts=[dict(
            name='Personal probe', description='Imported replacement',
            command="printf 'replacement %s\\n' {{message}}", administrator=False)])), encoding='utf-8')
        p.get_by_role('button', name='Import scripts…', exact=True).click()
        d = p.get_by_role('dialog', name='Import script bundle', exact=True)
        d.get_by_role('textbox', name='Script bundle path', exact=True).fill(str(incoming))
        d.get_by_role('button', name='Read bundle', exact=True).click()
        d.get_by_role('button', name='Replace scripts with matching names', exact=True).click()
        d.get_by_role('button', name='Review import…', exact=True).click()
        d.get_by_role('button', name='Apply import', exact=True).click()
        expect(row(p, 'Personal probe').get_by_role('button', name='Remove from favorites', exact=True)).to_be_visible()
        p.get_by_role('button', name='Export scripts…', exact=True).click()
        d = p.get_by_role('dialog', name='Export script bundle', exact=True)
        d.get_by_role('button', name='Select all scripts', exact=True).click()
        d.get_by_role('textbox', name='Export folder', exact=True).fill(str(out))
        d.get_by_role('textbox', name='Bundle file name', exact=True).fill('shared.json')
        d.get_by_role('button', name='Review export…', exact=True).click()
        d.get_by_role('button', name='Write new bundle', exact=True).click()
        expect(d).not_to_be_visible()
        shared = json.loads((out/'shared.json').read_text(encoding='utf-8'))
        assert 'script_favorites' not in shared and all('favorite' not in s and 'id' not in s for s in shared['scripts'])
        assert len(saved(app)['script_favorites']) == 2
        passed('Reviewed import replacement preserves the existing favorite; script sharing exports definitions without local favorite references')

        library(app)
        p.get_by_role('textbox', name='Search scripts', exact=True).fill('replacement')
        expect(p.locator('.script-row')).to_have_count(1)
        expect(p.locator('.script-row')).to_contain_text('Personal probe')
        p.get_by_role('textbox', name='Search scripts', exact=True).fill('no-such-script')
        expect(p.get_by_text('No matching scripts', exact=True)).to_be_visible()
        p.get_by_role('textbox', name='Search scripts', exact=True).fill('')
        app.stop()
        app.launch()
        p = library(app)
        expect(p.locator('.script-row')).to_have_count(2)
        assert before_auth == [Path(peer.data['auth_audit']).read_bytes() for peer in (a, b)]
        passed('Favorites search uses current definitions, labels empty search results correctly, and survives a fresh desktop process without network activity')

        row(p, 'Personal probe').get_by_role('button', name='Run on servers…', exact=True).click()
        d = p.get_by_role('dialog', name='Run script on servers', exact=True)
        d.get_by_role('textbox', name='message', exact=True).fill("favorite '; $(id)")
        d.locator('.target-choice').nth(0).click()
        d.locator('.target-choice').nth(1).click()
        d.get_by_role('button', name='Check targets', exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Ready', 'Ready'])
        expect(p.locator('.task-target').first).to_contain_text('replacement')
        p.get_by_role('button', name='Execute ready targets', exact=True).click()
        expect(p.locator('.task-status')).to_have_text(['Running', 'Running'])
        app.stop()
        time.sleep(4.5)
        app.launch()
        p = app.page
        p.locator('.rail-tasks').click()
        p.locator('.task-heading').first.click()
        for index in range(2):
            p.locator('.task-target').nth(index).get_by_role('button', name='Query server result', exact=True).click()
            p.get_by_role('dialog', name='Query server result', exact=True).get_by_role('button', name='Query', exact=True).click()
            expect(p.locator('.task-status').nth(index)).to_have_text('Succeeded')
        assert all(len(launches(peer).splitlines()) == 1 for peer in (a, b))
        passed('A favorite opens the normal reviewed multi-server form with the current script; accepted jobs survive desktop exit and query without resubmission')

        app.settings('General')
        p.locator('.setting-theme:visible').get_by_role('radio', name='Dark', exact=True).check()
        p.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check()
        p.set_viewport_size(dict(width=900, height=700))
        p = library(app, '收藏')
        expect(p.get_by_text('安装 Nginx', exact=True)).to_be_visible()
        expect(p.get_by_text('Personal probe', exact=True)).to_be_visible()
        expect(p.get_by_role('button', name='取消收藏', exact=True)).to_have_count(2)
        button = p.get_by_role('button', name='帮助：服务器任务', exact=True)
        button.hover()
        expect(p.get_by_role('tooltip')).to_contain_text('不固定旧版本')
        button.click()
        p.mouse.move(880, 680)
        expect(p.get_by_role('tooltip')).to_be_visible()
        p.keyboard.press('Escape')
        assert p.evaluate('document.documentElement.scrollWidth <= innerWidth')
        assert not p.locator('.script-page *:visible').evaluate_all("els => els.filter(e => !e.children.length && e.textContent.trim() && parseFloat(getComputedStyle(e).fontSize)<12).map(e=>e.textContent)")
        star = p.locator('.script-favorite').first
        star.focus()
        p.keyboard.press('Enter')
        expect(p.locator('.script-row')).to_have_count(1)
        p.locator('.context-links').get_by_role('button', name='官方脚本', exact=True).click()
        row(p, '安装 Nginx').get_by_role('button', name='加入收藏', exact=True).click()
        p.locator('.context-links').get_by_role('button', name='收藏', exact=True).click()
        app.screenshot('favorites-chinese-dark-900.png')
        passed('Chinese dark narrow layout keeps distinct source names, usable SVG stars, keyboard toggles and hover/click help without tiny text or overflow')

        app.settings('常规')
        p.locator('.setting-language:visible').get_by_role('radio', name='English', exact=True).check()
        p = library(app)
        row(p, 'Personal probe').get_by_role('button', name='Delete', exact=True).click()
        d = p.get_by_role('dialog', name='Delete command', exact=True)
        d.get_by_role('button', name='Cancel', exact=True).click()
        assert len(saved(app)['script_favorites']) == 2
        row(p, 'Personal probe').get_by_role('button', name='Delete', exact=True).click()
        p.get_by_role('dialog', name='Delete command', exact=True).get_by_role('button', name='Delete command', exact=True).click()
        expect(p.locator('.script-row')).to_have_count(1)
        assert saved(app)['script_favorites'] == [dict(source='official', script='builtin:nginx')]
        before = app.config.read_bytes()
        with app.config.open('rb'):
            p.get_by_role('button', name='Remove from favorites', exact=True).click()
            expect(p.locator('.script-page .form-error')).to_be_visible()
            assert app.config.read_bytes() == before
            expect(p.locator('.script-favorite .glyph-star-filled')).to_have_count(1)
        p.get_by_role('button', name='Remove from favorites', exact=True).click()
        expect(p.locator('.script-row')).to_have_count(0)
        assert 'script_favorites' not in saved(app)
        passed('Cancelled deletion keeps references; confirmed personal deletion preserves the same-ID official favorite, and failed saves never change the star or old bytes')

    recovery = Desktop(out/'read-only', config)
    bad = b'{ corrupt config'
    recovery.config.write_bytes(bad)
    with recovery as app:
        p = library(app, 'Official scripts')
        expect(p.locator('.script-row')).to_have_count(11)
        assert all(not button.is_enabled() for button in p.locator('.script-favorite').all())
        assert app.config.read_bytes() == bad
        passed('An unreadable startup configuration keeps the official catalog browsable and disables local favorite writes without changing corrupt bytes')

(out/'summary.json').write_text(json.dumps(dict(passed=len(checks), checks=checks), ensure_ascii=False, indent=2), encoding='utf-8')
print(json.dumps(dict(passed=len(checks), artifacts=str(out)), ensure_ascii=False), flush=True)
