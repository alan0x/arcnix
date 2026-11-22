"""Exercise persistent application preferences in an isolated real desktop WebView."""
import json
import re
import sys
import time

from desktop_fixture import Desktop, Peer, ROOT, eventually, expect


def exercise_native_picker(app, dialog, output):
    previous = app.out / "previous key"
    selected = app.out / "mock key 中文"
    previous.write_text("TEST FILE ONLY", encoding="utf-8")
    selected.write_text("TEST FILE ONLY", encoding="utf-8")

    field = dialog.get_by_role("textbox", name="Private key file", exact=True)
    browse = dialog.get_by_role("button", name="Browse…", exact=True)
    ready = output / "picker-ready.json"
    cancelled = output / "picker-cancelled"
    cancelled.unlink(missing_ok=True)
    field.fill(str(previous))
    try:
        for phase in ("select", "cancel"):
            browse.click()
            if dialog.locator(".form-error").is_visible():
                raise RuntimeError(dialog.locator(".form-error").inner_text())
            ready.write_text(json.dumps(dict(pid=app.process.pid, path=str(selected), phase=phase)), encoding="utf-8")
            print(f"Native picker ready for {phase}", flush=True)
            if phase == "cancel":
                eventually(cancelled.is_file, timeout=60)
            expect(field).to_have_value(str(selected), timeout=120000)
    finally:
        ready.unlink(missing_ok=True)
        cancelled.unlink(missing_ok=True)


def main():
    output = ROOT / "artifacts/settings-ui"
    run = output / str(time.time_ns())
    local = run / "files"
    local.mkdir(parents=True)
    (local / "visible.txt").write_text("Fixture", encoding="utf-8")
    (local / ".hidden-test").write_text("Fixture", encoding="utf-8")
    checks = []

    with Peer(run / "peer") as peer:
        config = peer.config([peer.profile("Settings fixture")], local)
        # An older profile without preferences exercises the application's defaults.
        del config["config"]["settings"]
        with Desktop(run / "app", config) as app:
            page = app.page
            app.connect()
            expect(app.file("visible.txt", False)).to_be_visible()
            expect(app.file(".hidden-test", False)).to_be_visible()

            app.settings("General")
            language = page.get_by_role("radiogroup", name="Language", exact=True)
            expect(language.get_by_role("radio", name="English", exact=True)).to_be_checked()
            theme = page.get_by_role("radiogroup", name="Theme", exact=True)
            theme.get_by_role("radio", name="Dark", exact=True).check()
            expect(page.locator(".app")).to_have_class(re.compile(r"\btheme-dark\b"))
            page.screenshot(path=str(output / "settings-dark.png"))
            theme.get_by_role("radio", name="Light", exact=True).check()
            expect(page.locator(".app")).to_have_class(re.compile(r"\btheme-light\b"))
            page.screenshot(path=str(output / "settings-light.png"))

            app.settings("Files")
            page.get_by_role("radio", name="Hide hidden files", exact=True).check()
            app.servers()
            expect(app.file("visible.txt", False)).to_be_visible()
            expect(app.file(".hidden-test", False)).to_have_count(0)
            app.settings("Monitoring")
            page.get_by_role("combobox", name="Resource refresh interval").select_option(label="Every 10 seconds")
            app.settings("General")
            language.get_by_role("radio", name="简体中文", exact=True).check()
            expect(page.locator(".page-header-title")).to_have_text("常规")
            page.screenshot(path=str(output / "settings-chinese.png"))
            saved = json.loads(app.config.read_text(encoding="utf-8"))["config"]["settings"]
            assert saved["language"] == "SimplifiedChinese" and saved["theme"] == "Light"
            assert saved["show_hidden_files"] is False and saved["resource_interval_seconds"] == 10
            checks.append("Default English, live theme and language changes, and local file visibility")

            app.stop()
            app.launch()
            page = app.page
            expect(page.locator(".app")).to_have_class(re.compile(r"\btheme-light\b"))
            app.settings("常规")
            expect(page.get_by_role("radio", name="简体中文", exact=True)).to_be_checked()
            page.get_by_role("radio", name="English", exact=True).check()
            app.settings("Monitoring")
            selected = page.get_by_role("combobox", name="Resource refresh interval").locator("option:checked")
            expect(selected).to_have_text("Every 10 seconds")
            app.servers()
            app.connect()
            expect(app.file("visible.txt", False)).to_be_visible()
            expect(app.file(".hidden-test", False)).to_have_count(0)
            app.application("Monitor")
            expect(page.get_by_text("Every 10 s", exact=True)).to_be_visible()
            checks.append("A fresh process restores preferences and Monitor uses the saved sampling interval")

            app.servers()
            page.locator(".sidebar-heading").get_by_role("button", name="New server", exact=True).click()
            dialog = page.get_by_role("dialog", name="New server", exact=True)
            authentication = dialog.get_by_role("radiogroup", name="Authentication", exact=True)
            password = authentication.get_by_role("radio", name="Password", exact=True)
            expect(password).to_be_checked()
            password.press("ArrowRight")
            expect(authentication.get_by_role("radio", name="Private key", exact=True)).to_be_checked()
            expect(dialog.get_by_role("textbox", name="Private key file", exact=True)).to_be_visible()
            expect(dialog.get_by_role("button", name="Browse…", exact=True)).to_be_enabled()
            checks.append("New server editor exposes its private key path and native browser")
            if "--picker" in sys.argv:
                exercise_native_picker(app, dialog, output)
                checks.append("Native picker accepts an extensionless Unicode test file and cancellation preserves it")
            page.keyboard.press("Escape")
            expect(dialog).to_have_count(0)

    print(json.dumps(dict(passed=len(checks), checks=checks, artifacts=str(run))), flush=True)


if __name__ == "__main__":
    main()
