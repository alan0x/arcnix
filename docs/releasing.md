# Releases

## GitHub Release binaries

Only pushing a matching `v*` tag runs `.github/workflows/release.yml`. Ordinary
pushes, pull requests and manually running the normal CI workflow do not run the
release matrix or publish assets. The tag must match every workspace package and
internal dependency version. The normal CI checks must pass before release builds.
Normal CI checks the complete workspace on Linux and Windows, including the Linux
desktop with GTK/WebKit dependencies, before a release tag is created.

| Platform | Native runner | Rust target | Downloads |
| --- | --- | --- | --- |
| macOS ARM64 | `macos-15` | `aarch64-apple-darwin` | `arcnix-vVERSION-macos-arm64.tar.gz` |
| Windows AMD64 | `windows-2025` | `x86_64-pc-windows-msvc` | `arcnix-vVERSION-windows-amd64-setup.exe` (installer), `arcnix-vVERSION-windows-amd64.zip` (portable) |
| Linux ARM64 | `ubuntu-24.04-arm` | `aarch64-unknown-linux-gnu` | `arcnix-vVERSION-linux-arm64.tar.gz` |
| Linux AMD64 | `ubuntu-24.04` | `x86_64-unknown-linux-gnu` | `arcnix-vVERSION-linux-amd64.tar.gz` |

Each native runner tests the workspace, builds with Rust **1.99.0** in release
mode, and packages the binary, READMEs and licenses. All four builds must succeed
before the downloads and `SHA256SUMS` are attached to a GitHub Release. Tags containing
a prerelease suffix produce a prerelease. Rerunning the release job replaces assets
on the existing release. GitHub assets do not depend on crates.io publication.

`tools/package_release.py` reads Cargo metadata to locate the actual workspace
target directory. It does not change Cargo home, the build directory or TEMP.

Windows packaging uses Inno Setup. Local packaging requires `ISCC.exe` on `PATH`.
The installer defaults to `%LOCALAPPDATA%\Programs\Arcnix`, runs without
administrator privileges, creates a Start menu shortcut and registers an
uninstaller in Windows Settings → Apps. User settings are stored separately and
survive uninstall. Windows release CI verifies silent installation, the shortcut,
the installed executable and uninstallation before uploading the assets.

## Desktop update channel

Windows AMD64 packaging also emits `arcnix-vVERSION-windows-amd64.exe`, copied
from the same binary as the ZIP. The release job uploads this executable and adds
it to `SHA256SUMS`. The updater reads the published stable release from the fixed
`arcnix-org/arcnix` GitHub API, requires the exact executable name and download path,
and verifies its size and GitHub API `sha256:` asset digest. Prereleases,
downgrades, missing digests and unexpected URLs are rejected. HTTPS metadata
plus an asset hash provides transport integrity, not an independent publisher
signature or protection against a compromised release account.

No new release is created by these local source changes. Until a matching stable
release and executable asset are published, the UI reports that no accessible
release is available or that automatic installation is unavailable. A missing
digest must be fixed in the release channel; there is no insecure fallback.

Only Windows x64 supports automatic replacement. Other platforms expose the
release page. Background checks/installation are enabled in release builds and
paused in development builds. The loopback fixture override is compiled into
debug builds only. Check intervals are daily (default), weekly or monthly;
automatic installation can be disabled independently.

Verified files are staged beside the configuration under `updates`. A trusted
copy of the current program validates the plan and acknowledges readiness before
the current process exits. Installation requires a writable application folder,
no SSH connection, no queued/running tasks and no open dialogs. After a deferrable
30-second countdown it replaces the executable, preserves `arcnix.exe.previous`,
starts the new program with the same configuration, and requires a startup
acknowledgement within 30 seconds. An early exit or missing acknowledgement
restores/restarts the old program. Failure details survive restart and suppress
automatic retries of that version. Release assets are never executed before
their API digest, architecture and reported version have been checked.

Validate replacement and rollback only on disposable copies:

```powershell
python tools/update_smoke.py
```

The test uses loopback release metadata, compiled test executables and isolated
configuration/WebView profiles. It never replaces the working installation.

## Platform requirements

- **Windows:** MSVC build tools, native Windows Perl and NASM for building;
  Inno Setup for packaging and WebView2 Runtime for running.
- **macOS:** Xcode Command Line Tools for building; macOS 13+ on Apple Silicon
  for running. Extract and move `Arcnix.app` to Applications. CI creates an ad-hoc
  signature; the app is not Developer ID signed or notarized.
- **Linux:** native builds on Ubuntu 24.04 with `build-essential`, `pkg-config`,
  `perl`, `libssl-dev`, `libgtk-3-dev` and `libwebkit2gtk-4.1-dev`. Archives contain
  the `arcnix` executable and require a graphical desktop, glibc 2.39+, GTK 3 and
  WebKitGTK 4.1 at runtime. On Ubuntu 24.04, install `libgtk-3-0t64` and
  `libwebkit2gtk-4.1-0`. These are not fully static or AppImage packages.

Native file selection and clipboard integration currently have Windows-only
implementations. Building other platforms does not establish full UI parity.

References: [GitHub runner platforms](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
[Wry Linux dependencies](https://github.com/tauri-apps/wry#linux-dependencies).

## Publish a version

1. Update `workspace.package.version` and the `arcnix-core` dependency version in
   the root `Cargo.toml`; update `Cargo.lock` and `CHANGELOG.md`.
2. Commit and push the changes to `main` and wait for CI to pass.
3. Create and push a new matching tag, for example:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

The example requires first updating the manifests to `0.2.0`. Existing release
tags retain their original workflow and are not moved by CI configuration changes.
No release is triggered by editing these workflows.

Before pushing a tag, validate its version locally:

```sh
python tools/check_release.py v0.2.0
```

Rustfmt uses **nightly-2026-11-04** to honor the project's existing unstable
formatting settings. Compilation, Clippy, tests and publishing use **Rust 1.99.0**.

## crates.io publication

crates.io publication is enabled with the repository Actions variable
`PUBLISH_CRATES_IO` set to exactly `true`. Set it to `false` to pause uploads for
future tags. The separate publish job uses the existing
organization Actions secret `CRATES_TOKEN` as `CARGO_REGISTRY_TOKEN` and runs
`cargo publish --workspace --locked` on Windows. The token must allow publishing
`arcnix-core` and `arcnix`, including creating new crate names. Verify the account
email and resolve any deleted-name reservation before enabling this job. Never
put a token in the repository or release commands.

For an existing tag whose crates.io job was skipped, enable the variable and
re-run that specific **Publish to crates.io** job. This uses the original tag and
commit without rebuilding or replacing GitHub Release assets. Check the registry
versions before retrying a partially completed upload.

## 发布说明

- 仅推送匹配工作区版本的 `v*` tag 时运行四个平台的原生测试、release 构建和打包；普通提交、PR 和手动运行普通 CI 不运行版本构建。
- ARM 指 ARM64；macOS 为 `.app` 压缩包，Windows 提供 `-setup.exe` 安装器和便携 ZIP，Linux 为两种架构的 tar.gz。全部成功后发布 GitHub Release 附件和 `SHA256SUMS`。
- macOS 目前仅临时签名，未经公证；Linux 包依赖系统 GTK 3、WebKitGTK 4.1 和 glibc 2.39+。原生文件选择、剪贴板当前仍以 Windows 为实现平台。
- crates.io 发布已通过 Actions 变量 `PUBLISH_CRATES_IO=true` 开启，设为 `false` 可暂停未来上传；GitHub 附件发布不依赖 crates.io。已有 tag 的上传被跳过时，可开启变量并单独重跑该上传任务，继续使用原提交；部分上传后重试前先核对仓库版本。
- 已存在的 tag 不会被移动；本次配置更新不触发发布。新版本先更新版本与锁文件，普通 CI 通过后再推送新 tag。
