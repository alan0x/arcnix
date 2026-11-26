"""Check native Windows app colors and explicit overrides in the real WebView."""
import argparse
import json
import time
import winreg
from pathlib import Path

from desktop_fixture import Desktop, ROOT, eventually, expect


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path)
    args = parser.parse_args()
    output = ROOT / 'artifacts/system-theme' / str(time.time_ns())
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,
                        r'Software\Microsoft\Windows\CurrentVersion\Themes\Personalize') as key:
        native = 'light' if winreg.QueryValueEx(key, 'AppsUseLightTheme')[0] else 'dark'
    opposite = 'dark' if native == 'light' else 'light'
    config = dict(version=1, config=dict(groups=[], servers=[], trusted_hosts=[]))
    with Desktop(output, config, binary=args.exe) as app:
        def settings():
            app.page.locator('.settings-link').click()
            return app.page.locator('.setting-theme:visible')

        def check(mode):
            expect(app.page.locator('.app')).to_have_class(f'app theme-{mode}')

        def saved():
            return json.loads(app.config.read_text())['config']['settings'].get('theme')

        check(native)
        choices = settings()
        expect(choices.get_by_role('radio', checked=True)).to_have_accessible_name('Follow system')
        before = app.config.read_bytes()
        # Reproduce the WebView reporting a preference opposite to Windows.
        app.page.emulate_media(color_scheme=opposite)
        assert app.page.evaluate("matchMedia('(prefers-color-scheme: dark)').matches") == (opposite == 'dark')
        check(native)
        assert app.config.read_bytes() == before
        for mode in ('dark', 'light'):
            choices.get_by_role('radio', name=mode.title(), exact=True).check()
            check(mode)
            eventually(lambda: saved() == mode.title())
        choices.get_by_role('radio', name='Follow system', exact=True).check()
        check(native)
        eventually(lambda: saved() == 'System')
        app.screenshot('follow-system.png')

        app.page.locator('.setting-language:visible').get_by_role('radio', name='简体中文', exact=True).check()
        expect(choices.get_by_role('radio', checked=True)).to_have_accessible_name('跟随系统')
        check(native)
        app.stop()
        app.launch()
        check(native)
        choices = settings()
        expect(choices.get_by_role('radio', checked=True)).to_have_accessible_name('跟随系统')

        choices.get_by_role('radio', name='浅色' if opposite == 'light' else '深色', exact=True).check()
        check(opposite)
        app.stop()
        app.launch()
        check(opposite)
    print(f'Passed: Windows {native}, opposite WebView preference, overrides, language and restart.')


if __name__ == '__main__':
    main()
