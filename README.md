# Arcnix

[简体中文](README.zh-CN.md) · [Download](https://arcnix.org/#downloads) · [Quick start](https://arcnix.org/docs/quick-start/) · [Documentation](https://arcnix.org/docs/)

Arcnix is a desktop app for managing Linux servers over SSH. The interface runs on your computer. Arcnix uses your SSH account to open shells and run server commands, and SFTP to read and write remote files. The server needs an SSH service. It does not need an Arcnix agent.

## What you can do

- **Files and code.** Transfer files between your computer and the server, edit remote text and preview Markdown. On Windows x64, stream remote MP4 and MKV files.
- **Terminal.** Open an interactive shell in a remote folder or use the terminal inside the editor.
- **Monitoring and Docker.** Check resource usage, services and logs. Inspect containers, ports and volumes on Docker hosts.
- **Tasks and backups.** Review scripts across hosts, check the results of server-owned tasks and restore files from restic snapshots.

Desktop downloads are available for Windows x64, macOS on Apple Silicon, and Linux x64 and ARM64.

## Connect to local WSL on Windows

Open **New server → Choose local WSL distribution** to see installed distributions and their running state. Selecting one starts it and fills in its default Linux username. Enter its SSH port, choose password or private key authentication, then save and connect. Commands and file edits affect that distribution directly.

Run OpenSSH Server in the distribution on a port reachable from Windows at `127.0.0.1`. On Ubuntu or Debian with systemd, use `systemctl status ssh` to check the service and `ss -ltn` to check listening ports. For local-only access, set the sshd `ListenAddress` to `127.0.0.1`. Give concurrently used distributions distinct ports and host keys. Before sending credentials, Arcnix checks the SSH peer against the selected distribution’s public keys in `/etc/ssh/ssh_host_*_key.pub`.

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
