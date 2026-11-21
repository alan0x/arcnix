"""Launch the Windows feature-test desktop with a local test page."""

import argparse
import time

from desktop_browser_smoke import site
from desktop_fixture import Desktop, Peer, ROOT, executable, expect


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--public-web", action="store_true", help="Allow forwarding to public HTTP/HTTPS sites")
    parser.add_argument("--url", help="Open a web address in a second tab after the local test page")
    args = parser.parse_args()

    source = executable()
    if not source.is_file():
        parser.error("Build Arcnix first with cargo build -p arcnix --locked")
    out = ROOT / "artifacts" / "manual-preview" / f"desktop-{time.time_ns()}"
    out.mkdir(parents=True)

    with site() as localhost, Peer(out / "ssh", browser_port=localhost.server_port, browser_web=args.public_web) as peer:
        label = "功能测试服务器（公网预览）" if args.public_web else "功能测试服务器"
        profile = peer.profile(label)
        config = peer.config([profile], out)
        with Desktop(out / "app", config, binary=source, viewport=None) as app:
            app.connect()
            app.application("Browser")
            browser = app.page.locator(".desktop-window.app-browser")
            address = browser.get_by_role("textbox", name="Web address", exact=True)
            address.fill(f"http://browser-fixture.test:{localhost.server_port}/first")
            address.press("Enter")
            expect(browser.locator(".browser-tab.active .browser-tab-label")).to_have_text(
                "Routed first page", timeout=20_000,
            )
            if args.url:
                browser.get_by_role("button", name="New web tab", exact=True).last.click()
                address.fill(args.url)
                address.press("Enter")
            print(f"Arcnix preview ready (PID {app.process.pid}). Logs: {out}", flush=True)
            return app.process.wait()


if __name__ == "__main__":
    raise SystemExit(main())
