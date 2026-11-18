"""Application-first catalog, source selection and merged installed inventory."""
import json,time,re
from desktop_fixture import Desktop,Peer,ROOT,expect,eventually
from docker_desktop_smoke import seed

def main():
    out=ROOT/'artifacts/software-store'/f'run-{time.time_ns()}';out.mkdir(parents=True)
    local=out/'local';local.mkdir();checks=[]
    with Peer(out/'ubuntu') as ubuntu,Peer(out/'fedora') as fedora:
        (ubuntu.root/'docker-fixture.json').write_text(json.dumps(seed()))
        (ubuntu.root/'software-snap-enabled').touch()
        (fedora.root/'software-manager').write_text('dnf')
        config=ubuntu.config([ubuntu.profile('Ubuntu applications'),fedora.profile('Fedora applications')],local)
        config['config']['trusted_hosts']+=fedora.config([],local)['config']['trusted_hosts']
        with Desktop(out/'desktop',config) as app:
            p=app.page;errors=[];p.on('pageerror',lambda e:errors.append(str(e)))
            def card(name):return p.locator('.store-app-card').filter(has=p.get_by_role('button',name=name,exact=True))
            def install(name):
                card(name).get_by_role('button',name='Install…',exact=True).click()
                return p.get_by_role('dialog',name='Install software',exact=True)
            def close(d):d.get_by_role('button',name='Close dialog',exact=True).click();expect(d).not_to_be_visible()
            app.connect();app.application('Software')
            expect(p.locator('.store-app-card')).to_have_count(18)
            expect(p.locator('.install-sources')).to_have_count(0)
            expect(p.locator('.software-sources')).to_have_count(0)
            expect(p.locator('.store-featured')).to_be_visible()
            app.screenshot('store-light.png')
            p.locator('.store-categories').get_by_role('button',name='Databases',exact=True).click();expect(p.locator('.store-app-card')).to_have_count(4)
            p.locator('.store-categories').get_by_role('button',name='All applications',exact=True).click()
            p.get_by_role('textbox',name='Search applications',exact=True).fill('restic');expect(p.locator('.store-app-card')).to_have_count(1)
            card('restic').get_by_role('button',name='restic',exact=True).click();expect(p.locator('.store-detail')).to_contain_text('Encrypted snapshots')
            expect(p.locator('.store-detail .store-website')).to_have_text('https://restic.net/')
            expect(p.locator('.install-sources')).to_have_count(0)
            p.get_by_role('textbox',name='Search applications',exact=True).fill('unfindable-application');expect(p.locator('.store-empty')).to_be_visible()
            p.get_by_role('textbox',name='Search applications',exact=True).fill('')
            checks.append('Catalog shows 18 application cards, categories, search, product details and an empty state without exposing provider tabs during browsing')

            d=install('Caddy');expect(d.locator('.install-app-name')).to_have_text('Caddy')
            for source in ['System packages','Snap','Docker Compose']:expect(d.get_by_role('button',name=source,exact=True)).to_be_enabled()
            d.get_by_role('button',name='Snap',exact=True).click();expect(d.locator('.install-package-name')).to_have_text('caddy');expect(d).to_contain_text('aoilinux')
            d.get_by_role('button',name='Docker Compose',exact=True).click();expect(d.get_by_role('textbox',name='Container image',exact=True)).to_have_value('caddy:2-alpine')
            expect(d.get_by_role('textbox',name='Project name',exact=True)).to_have_value('caddy')
            app.screenshot('caddy-methods.png');close(d)
            d=install('Git');expect(d.get_by_role('button',name='Snap',exact=True)).to_be_disabled();expect(d.get_by_role('button',name='Docker Compose',exact=True)).to_be_disabled();expect(d).to_contain_text('No Compose template');close(d)
            d=install('Certbot');d.get_by_role('button',name='Snap',exact=True).click();expect(d.get_by_role('button',name='Review installation',exact=True)).to_be_disabled()
            d.get_by_role('checkbox',name='Allow classic confinement',exact=True).check();expect(d.get_by_role('button',name='Review installation',exact=True)).to_be_enabled();close(d)
            checks.append('Install opens app-specific provider cards; supported sources carry correct names/images and publisher, unsupported sources are disabled, and classic confinement requires an explicit choice')

            p.locator('.store-navigation').get_by_role('button',name='Installed',exact=True).click()
            p.get_by_role('checkbox',name='Show all system packages',exact=True).check()
            expect(p.locator('.store-inventory-list')).to_contain_text('curl')
            expect(p.locator('.store-installed-row').filter(has_text='curl').locator('.store-source-badge')).to_have_text('APT')
            p.locator('.store-installed-row').filter(has_text='curl').get_by_role('button',name='Upgrade…',exact=True).click()
            task=p.get_by_role('dialog',name='Run script on servers',exact=True);expect(task).to_contain_text('apt-get install -y --only-upgrade');task.get_by_role('button',name='Cancel',exact=True).click()
            app.servers();app.application('Docker');p.locator('.docker-toolbar').get_by_role('button',name='Install software',exact=True).click()
            expect(p.locator('.software-store')).to_be_visible();expect(p.locator('.store-app-card')).to_have_count(3);expect(p.get_by_role('dialog')).to_have_count(0)
            d=install('Nginx');expect(d.get_by_role('button',name='Docker Compose',exact=True)).to_have_attribute('aria-pressed','true');close(d)
            p.get_by_role('button',name='Show all applications',exact=True).click();expect(p.locator('.store-app-card')).to_have_count(18)
            checks.append('Installed inventory retains source-specific management, while Docker installation enters the same catalog with Compose-compatible apps and opens provider choice only after Install')

            p.locator('.host-select').nth(1).click();app.connect(1);app.application('Software')
            d=install('PostgreSQL');expect(d.locator('.install-package-name')).to_have_text('postgresql-server');expect(d.locator('.install-provider')).to_contain_text('DNF')
            expect(d.get_by_role('button',name='Snap',exact=True)).to_be_disabled();close(d)
            d=install('htop');expect(d.get_by_role('button',name='Snap',exact=True)).to_be_disabled();expect(d).to_contain_text('Requires Snap');close(d)
            app.application('Settings');settings=p.locator('.desktop-settings-page');settings.get_by_role('button',name='Desktop',exact=True).click();settings.get_by_role('button',name='Dark',exact=True).click();expect(settings.get_by_role('button',name='Dark',exact=True)).to_have_attribute('aria-pressed','true');app.application('Software store')
            expect(p.locator('.app')).to_have_class('app theme-light')
            app.screenshot('store-dark.png')
            for width in [1100,900]:
                p.set_viewport_size(dict(width=width,height=800));assert p.evaluate('document.documentElement.scrollWidth <= innerWidth')
                app.screenshot(f'store-dark-{width}.png')
            p.set_viewport_size(dict(width=1440,height=980))
            app.settings('General');p.get_by_role('button',name='简体中文',exact=True).click();app.servers()
            p.get_by_role('textbox',name='搜索应用',exact=True).fill('数据库');expect(p.locator('.store-app-card')).to_have_count(4)
            p.get_by_role('textbox',name='搜索应用',exact=True).fill('');app.screenshot('store-chinese.png')
            assert not errors,errors
            checks.append('Fedora uses distro-specific package identities, missing runtimes remain unavailable, and light/dark, Chinese search and narrow layouts preserve desktop theme boundaries')
    (out/'results.json').write_text(json.dumps(checks,indent=2));print(json.dumps(dict(output=str(out),checks=checks),indent=2))

if __name__=='__main__':main()
