# Arcnix

[English](README.md) · [下载](https://arcnix.org/zh-cn/#downloads) · [快速上手](https://arcnix.org/zh-cn/docs/quick-start/) · [使用文档](https://arcnix.org/zh-cn/docs/)

Arcnix 是一个通过 SSH 管理 Linux 服务器的桌面应用。界面运行在你的电脑上，终端和管理命令使用你的 SSH 账号在服务器上执行，远程文件通过 SFTP 读写。服务器开启 SSH 即可连接，无需安装 Arcnix 服务端。

## 主要功能

- **文件与代码**　在本机和服务器之间传输文件，直接编辑远程文本、预览 Markdown。Windows x64 还支持播放远程 MP4 和 MKV。
- **终端**　在指定远程目录打开交互式终端，也可以在代码编辑器里使用终端。
- **监控与 Docker**　查看资源用量、服务和日志。在运行 Docker 的服务器上检查容器、端口和卷。
- **任务与备份**　在多台服务器上审核并运行脚本，查询已提交任务的结果，从 restic 快照恢复文件。

客户端提供 Windows x64、macOS Apple Silicon，以及 Linux x64 和 ARM64 下载包。

## 功能演示

### SFTP 文件传输

上传本地文件，在传输队列中暂停和恢复上传，再从服务器下载文件。

![Arcnix 并排显示本地和远程目录，传输队列显示上传与下载进度。](docs/media/file-transfer.gif)

### 远程代码编辑

编辑并保存服务器上的 Markdown 文件，在源码右侧查看渲染效果。

![Arcnix 在代码编辑器中打开 handover.md，保存修改并在分栏中显示 Markdown 预览。](docs/media/remote-editing.gif)

### 服务器监控

查看网络吞吐量，再切换到服务页面读取 Nginx 日志。

![Arcnix 的监控窗口从网络接口统计切换到 nginx.service 的日志。](docs/media/server-monitoring.gif)

### Docker

启动已停止的容器，查看容器日志、卷挂载和端口映射。

![Arcnix 启动 report-worker 容器并查看它的日志，然后切换到卷和端口页面。](docs/media/docker.gif)

[开发说明](docs/development.md) · [更新记录](CHANGELOG.md) · [Apache-2.0 许可证](LICENSE)
