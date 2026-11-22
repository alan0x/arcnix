# Arcnix

[简体中文](README.zh-CN.md) · [Download](https://arcnix.org/#downloads) · [Quick start](https://arcnix.org/docs/quick-start/) · [Documentation](https://arcnix.org/docs/)

Arcnix is a desktop app for managing Linux servers over SSH. The interface runs on your computer. Arcnix uses your SSH account to open shells and run server commands, and SFTP to read and write remote files. The server needs an SSH service. It does not need an Arcnix agent.

## What you can do

- **Files and code.** Transfer files between your computer and the server, edit remote text and preview Markdown. On Windows x64, stream remote MP4 and MKV files.
- **Terminal.** Open an interactive shell in a remote folder or use the terminal inside the editor.
- **Monitoring and Docker.** Check resource usage, services and logs. Inspect containers, ports and volumes on Docker hosts.
- **Tasks and backups.** Review scripts across hosts, check the results of server-owned tasks and restore files from restic snapshots.

Desktop downloads are available for Windows x64, macOS on Apple Silicon, and Linux x64 and ARM64.

## Try the local desktop

Choose **Try the local desktop** on the home screen. Linux and macOS use your current account. Windows opens an installed WSL distribution using its default account and offers a choice when several are installed. No SSH service or credentials are required.

Open Files to browse your home directory and create a file. Double-click a text file to edit and save it, or choose **View → Edit source** in a Markdown preview. Choose **Terminal here** on a file or directory to open a shell in that location. Changes affect real files. Desktop preferences are saved on the client separately for each local target. The standalone File Transfer workspace is disabled for local desktops.

## Connect to local WSL on Windows

Open **New server → Choose local WSL distribution**, select an installed distribution, save and connect. Saved WSL profiles also default to a local connection. Arcnix opens an interactive Linux shell through `wsl.exe` and runs file operations as the distribution’s default Linux user. No SSH server, password or keys are required.

Browse directories, create files and folders, preview UTF-8 text, and upload or download individual files. Copies preserve existing destinations. You can also open the current directory in a terminal or Windows File Explorer.

To use the full SSH server desktop, edit the profile, select **SSH**, and enter the port and credentials. This mode requires OpenSSH Server on a port reachable from Windows over loopback. Arcnix verifies the distribution’s SSH host public keys before authentication.

## Feature demos

### SFTP file transfer

Upload a local file, pause and resume the transfer, then download a file from the server.

![Arcnix shows local and remote folders side by side with upload and download progress in the transfer queue.](docs/media/file-transfer.gif)

### Remote code editing

Edit a Markdown file on the server, save it and open the preview beside the source.

![Arcnix opens handover.md in the code editor and renders the saved changes in a split Markdown preview.](docs/media/remote-editing.gif)

### Server monitoring

Check network throughput, then open the Services tab to read Nginx logs.

![Arcnix Monitor switches from network interface statistics to the logs for nginx.service.](docs/media/server-monitoring.gif)

### Docker

Start a stopped container and inspect its logs, volume mounts and port mappings.

![Arcnix starts report-worker, reads its logs, then switches to the Volumes and Ports views.](docs/media/docker.gif)

[Development](docs/development.md) · [Changelog](CHANGELOG.md) · [Apache-2.0 license](LICENSE)
