# 容器化部署指南

本目录提供两种 Linux 上常用的 Podman 部署方式：

| 文件 | 用途 |
| --- | --- |
| `podman-compose.yaml` | Podman Compose 示例（推荐先用它试跑） |
| `bmp-status.container` | systemd Quadlet 容器单元示例 |
| `bmp-status.volume` | Quadlet 命名卷单元 |
| `bmp-status.env.example` | 环境变量样例（复制为 `bmp-status.env` 并修改） |

根目录的 [`Dockerfile`](../../Dockerfile) 是自包含的三阶段构建：

1. `node:22-bookworm-slim` 构建 Next.js 静态前端；
2. `rust:1.94-alpine` 编译 Rust 后端（musl 静态链接），`build.rs` 将前端嵌入二进制；
3. `alpine:3.22` 作为运行时，只包含 musl 二进制和 busybox（健康检查用 `wget`）。

Makefile 还提供不依赖 qemu 的交叉构建：

- `make cross-binaries`：在原生架构的 `rust:1.94-alpine` 容器中，用
  `rust-lld` 链接 `x86_64-unknown-linux-musl` / `aarch64-unknown-linux-musl`；
- `make cross-images`：用两个 musl 二进制组装 amd64/arm64 的 Alpine 镜像，
  并在 `prebuilt-images/` 生成按架构区分的可加载归档和 `load.sh`。

构建容器默认使用 **CERNET（教育网联合镜像站）** 的 Alpine apk / Debian apt
源，说明见 <https://help.mirrors.cernet.edu.cn/debian/>。可通过
`ALPINE_MIRROR`、`DEBIAN_MIRROR`、`DEBIAN_SECURITY_MIRROR` 覆盖。

镜像特点：

- 非 root 运行（UID/GID `10001`）；
- 仅 `/var/lib/bmp-status` 需要持久化，建议挂载卷；
- 支持只读根文件系统 + `/tmp` tmpfs；
- 推荐 `cap_drop: ALL` + `no-new-privileges`；
- 端口：`11019/tcp`（BMP）、`8787/tcp`（页面/API/WebSocket）；
- 健康检查：`GET /api/health`（镜像内使用 busybox `wget`）；
- 运行时约 18 MB，静态 musl，无 apt/curl；
- 仪表盘、管理员面板、WebSocket 和业务 API 共用页面密码，支持 HttpOnly Cookie 自动登录；
- `/api/health` 保持公开，仅用于容器健康检查。

---

## 1. 构建镜像

### 1.1 使用 Makefile（推荐）

```bash
# 本机单一二进制（前端 + 后端）
make single-binary

# amd64/arm64 静态 musl 二进制（Podman 容器内交叉编译）
make cross-binaries

# 两个架构的 Alpine 镜像，写入 prebuilt-images/
make cross-images

# 全部：本机二进制 + 交叉二进制 + 跨架构镜像
make all
```

产物：

| 路径 | 内容 |
| --- | --- |
| `bin/bmp-status-linux-<arch>-native` | 本机原生二进制（动态链接） |
| `bin/bmp-status-linux-amd64-musl` | amd64 静态 musl 二进制 |
| `bin/bmp-status-linux-arm64-musl` | arm64 静态 musl 二进制 |
| `prebuilt-images/bmp-status-<version>-linux-amd64.tar` | amd64 Docker 归档 |
| `prebuilt-images/bmp-status-<version>-linux-arm64.tar` | arm64 Docker 归档 |
| `prebuilt-images/load.sh` | 目标机器自动选择架构、加载并打 `latest` 标签 |
| `prebuilt-images/SHA256SUMS` | 归档校验和 |

`bin/` 和 `prebuilt-images/` 只在构建成功后创建，已加入 `.gitignore`。构建
主机上的 `localhost/bmp-status:latest` 是由两个架构组成的 manifest list，可
直接用于本机 `podman run`；复制到其他机器时使用对应架构的 tar（Podman 不能
把本地 OCI index 归档直接当作 manifest list 加载）。

中国网络下，容器脚本会自动检测并提示输入代理（例如
`http://192.168.1.114:7890`），缓存到被忽略的 `.build.env`：

```bash
PROXY=http://192.168.1.114:7890 make cross-images
FORCE_CN=1 make cross-images       # 强制按中国网络处理
ALLOW_NO_PROXY=1 make cross-images # 明确允许直连
```

删除 `.build.env` 可清除缓存的代理，下一次构建会重新检测并提示。

构建镜像的拉取由 Podman 自身的 `HTTP_PROXY/HTTPS_PROXY` 环境变量控制；容器内
的 apk/cargo 下载由脚本传给构建参数/容器环境。CI 可使用隔离存储：

```bash
PODMAN_ROOT=/tmp/bmp-podman/storage \
PODMAN_RUNROOT=/tmp/bmp-podman/run \
make cross-images
```

目标机器部署：

```bash
# 只复制 prebuilt-images/ 即可
cd prebuilt-images
./load.sh
podman run -d --name bmp-status -p 8787:8787 \
  -v bmp-status-data:/var/lib/bmp-status localhost/bmp-status:latest
```

### 1.2 单独构建当前架构（Dockerfile）

在仓库根目录执行。网络受限时先设置代理；Podman 拉取基础镜像使用环境变量，
Dockerfile 内的 RUN 使用 build args：

```bash
export HTTP_PROXY=http://192.168.1.114:7890
export HTTPS_PROXY=http://192.168.1.114:7890
export NO_PROXY=localhost,127.0.0.1

podman build --format docker --network=host \
  --build-arg HTTP_PROXY="$HTTP_PROXY" \
  --build-arg HTTPS_PROXY="$HTTPS_PROXY" \
  --build-arg NO_PROXY="$NO_PROXY" \
  --build-arg ALPINE_MIRROR=mirrors.cernet.edu.cn \
  --build-arg DEBIAN_MIRROR=mirrors.cernet.edu.cn \
  --build-arg DEBIAN_SECURITY_MIRROR=mirrors.cernet.edu.cn \
  -t localhost/bmp-status:latest .
```

> Podman 默认使用 OCI 镜像格式，会忽略 Dockerfile 中的 `HEALTHCHECK`。
> 使用 `--format docker` 可以保留镜像级健康检查；Compose/Quadlet 示例本身
> 也各自定义了健康检查，因此即使使用默认 OCI 格式也能正常探活。

> Alpine apk 和 Debian apt 源默认都替换为 CERNET。构建阶段不再安装 Debian
> 包；前端 Debian 阶段仅切换 apt 配置，实际下载来自 npm。若需要其他镜像站，
> 覆盖 `ALPINE_MIRROR` / `DEBIAN_MIRROR` / `DEBIAN_SECURITY_MIRROR`。

### 1.3 清理构建产物

```bash
# 删除 Makefile 创建的本地镜像标签/manifest（保留 prebuilt-images/）
make clean-images

# 删除前端/后端输出、bin/、prebuilt-images/ 和本地镜像
make clean

# 可选：清理所有未引用的 dangling 镜像（执行前确认）
podman image prune -f
```

`make cross-images` 结束时会自动清理带 `BMP Status` 标签的 dangling 镜像；
`make clean-images` 还会额外清理所选存储中的 dangling manifest list。生产环境
请保留镜像；清理命令主要用于开发机/CI。使用隔离的 `--root`/`--runroot` 临时存储时，构建结束后直接
删除临时目录即可，完全不污染用户镜像仓库。

---

## 2. Podman Compose

### 2.1 准备环境变量

```bash
cd docs/deploy
cp bmp-status.env.example bmp-status.env
chmod 600 bmp-status.env
# bmp-status.env 已在 .gitignore 中，不会误提交
$EDITOR bmp-status.env
```

至少把 `ADMIN_PASSWORD` 改成强密码（8 字符以上）。

> 设置 `ADMIN_PASSWORD` 时，每次容器启动都会重置页面密码。
> 若希望使用首次启动生成的随机密码，删除该变量，然后从卷中的
> `initial-admin-password.txt` 读取。

### 2.2 启动

在 `docs/deploy` 目录中执行（podman-compose 1.x 对相对 `-f` 路径的处理依赖
当前目录，因此建议直接 `cd` 到该目录）：

```bash
cd docs/deploy

# 推荐：先在仓库根目录 make cross-images（或先执行 prebuilt-images/load.sh），
# 然后直接启动，不再触发构建
podman compose -f podman-compose.yaml up -d

# 如果尚未构建，也可以让 compose 用根 Dockerfile 构建当前架构
podman compose -f podman-compose.yaml up -d --build

# 查看状态与日志
podman compose -f podman-compose.yaml ps
podman compose -f podman-compose.yaml logs -f bmp-status
```

也可以使用传统的 `podman-compose`：

```bash
cd docs/deploy
podman-compose -f podman-compose.yaml up -d --build
```

### 2.3 访问

- 仪表盘：<http://<服务器IP>:8787/>
- 管理员：<http://<服务器IP>:8787/admin/>
- BMP 端口：`tcp://<服务器IP>:11019`

### 2.4 网络模式说明

示例默认使用 **host 网络**：

```yaml
network_mode: host
```

这样做是为了让 BMP 连接保留真实源 IP，管理员面板的“来源白名单”才能按
路由器地址匹配。

如果改成 bridge 网络：

```yaml
network_mode: bridge
ports:
  - "11019:11019/tcp"
  - "8787:8787/tcp"
```

那么 BMP 的源 IP 会变成 NAT 网关地址，白名单无法识别真实路由器。此时可以
关闭白名单，或者继续使用 host 网络。

### 2.5 停止与删除

```bash
cd docs/deploy

# 停止（保留数据卷）
podman compose -f podman-compose.yaml down

# 停止并删除数据卷（会清空实例/页面密码配置）
podman compose -f podman-compose.yaml down -v
```

---

## 3. systemd Quadlet

Quadlet 适合长期运行的服务器。以下以 rootful 为例；rootless 见 3.4。

### 3.0 最短流程（rootful）

先在仓库根目录构建并加载镜像：

```bash
# 构建 amd64/arm64 Alpine 镜像，并生成 prebuilt-images/
make cross-images

# rootful Quadlet 使用 root 的 Podman 存储
sudo ./prebuilt-images/load.sh

# 确认镜像存在
sudo podman images | grep bmp-status
```

安装单元文件和环境文件：

```bash
sudo install -d /etc/containers/systemd /etc/bmp-status
sudo cp docs/deploy/bmp-status.volume /etc/containers/systemd/
sudo cp docs/deploy/bmp-status.container /etc/containers/systemd/
sudo cp docs/deploy/bmp-status.env.example /etc/bmp-status/bmp-status.env
sudo chmod 600 /etc/bmp-status/bmp-status.env
sudo $EDITOR /etc/bmp-status/bmp-status.env
```

至少修改 `ADMIN_PASSWORD`。启动：

```bash
sudo systemctl daemon-reload
sudo systemctl start bmp-status

sudo systemctl status bmp-status
sudo journalctl -u bmp-status -f
```

Quadlet 生成的是 transient service，不能使用 `systemctl enable`。示例
`bmp-status.container` 已经包含 `[Install] WantedBy=default.target`，Quadlet
在每次 `daemon-reload`/开机生成时会处理它；只要单元文件留在
`/etc/containers/systemd/`，重启后会自动启动。

浏览器访问：

```text
http://<服务器IP>:8787/
```

先在 `/` 输入页面密码；登录后 `/admin/` 直接进入，不会再次要求密码。
如果环境文件没有设置 `ADMIN_PASSWORD`，从容器内读取首次生成的页面密码：

```bash
sudo podman exec bmp-status cat /var/lib/bmp-status/initial-admin-password.txt
```

查看/修改运行数据上限：

```text
登录后打开 /admin/ → “运行数据上限”
```

配置文件保存在卷内的 `/var/lib/bmp-status/runtime.sqlite3`，不会随容器重建丢失。

### 3.1 准备镜像

Quadlet 单元引用 `localhost/bmp-status:latest`。rootful Quadlet 必须把镜像
放进 **rootful** Podman 存储，rootless 构建的镜像 rootful 服务看不到：

```bash
# 普通用户构建（写入 prebuilt-images/）
make cross-images

# rootful Quadlet：以 root 加载对应架构的预编译归档
sudo ./prebuilt-images/load.sh

# rootless Quadlet：普通用户加载即可
./prebuilt-images/load.sh
```

验证：

```bash
sudo podman images | grep bmp-status   # rootful
podman images | grep bmp-status        # rootless
```

### 3.2 安装单元与环境文件

在仓库根目录执行：

```bash
sudo install -d /etc/containers/systemd /etc/bmp-status
sudo cp docs/deploy/bmp-status.volume /etc/containers/systemd/
sudo cp docs/deploy/bmp-status.container /etc/containers/systemd/
sudo cp docs/deploy/bmp-status.env.example /etc/bmp-status/bmp-status.env
sudo chmod 600 /etc/bmp-status/bmp-status.env
sudo $EDITOR /etc/bmp-status/bmp-status.env
```

### 3.3 启动

```bash
sudo systemctl daemon-reload
sudo systemctl start bmp-status
sudo systemctl status bmp-status
sudo journalctl -u bmp-status -f
```

Quadlet 会自动创建：

- `bmp-status-data` 命名卷（由 `bmp-status.volume` 声明）；
- `/var/lib/bmp-status` 挂载点；
- 健康检查和自动重启。

### 3.4 rootless Quadlet

先准备镜像到当前用户的 Podman 存储：

```bash
make cross-images
./prebuilt-images/load.sh
podman images | grep bmp-status
```

然后安装 rootless Quadlet 单元：

```bash
install -d ~/.config/containers/systemd ~/.config/bmp-status
cp docs/deploy/bmp-status.volume ~/.config/containers/systemd/
cp docs/deploy/bmp-status.container ~/.config/containers/systemd/
cp docs/deploy/bmp-status.env.example ~/.config/bmp-status/bmp-status.env
chmod 600 ~/.config/bmp-status/bmp-status.env

# 修改 EnvironmentFile= 为：
# EnvironmentFile=%h/.config/bmp-status/bmp-status.env
# 如果 %h 在你的系统中不生效，可写绝对路径：
# EnvironmentFile=/home/<user>/.config/bmp-status/bmp-status.env

systemctl --user daemon-reload
systemctl --user start bmp-status
systemctl --user status bmp-status
journalctl --user -u bmp-status -f
loginctl enable-linger "$USER"
```

rootless 也不要使用 `systemctl --user enable`；同样由 Quadlet 的
`[Install] WantedBy=default.target` 处理开机启动。

rootless 模式下 host 网络端口 `11019`、`8787` 都大于 1024，不需要 root 权限。

### 3.5 更新

```bash
# 1) 重新构建镜像（推荐跨架构）
make cross-images
# rootful Quadlet 需要加载到 root 存储：
sudo ./prebuilt-images/load.sh
# 或只构建当前架构：podman build --format docker ... -t localhost/bmp-status:latest .

# 2) 重启单元（数据卷保留）
sudo systemctl restart bmp-status
# 或 rootless:
systemctl --user restart bmp-status
```

如果使用远端 registry，可以在 `.container` 中启用 `AutoUpdate=registry`，
并配合 `podman auto-update`。

---

## 4. 数据、密码与持久化

容器内数据目录：

```text
/var/lib/bmp-status/
├── admin.json                  # Argon2 密码哈希 + 实例登记 + 白名单开关
├── runtime.sqlite3             # 运行数据上限（消息/路由/peer），不含密码
└── initial-admin-password.txt  # 仅在未设置 ADMIN_PASSWORD 且首次启动时生成
```

- 命名卷：`bmp-status-data`，删除容器不会丢数据；`down -v` / 删除卷才会清空。
- 使用 bind mount 时，目录必须对 UID `10001` 可写：

  ```bash
  sudo install -d -o 10001 -g 10001 /srv/bmp-status/data
  # 然后把 compose/quadlet 的 Volume 改成 /srv/bmp-status/data:/var/lib/bmp-status:Z
  ```

- 读取自动生成的页面密码（未设置 `ADMIN_PASSWORD` 时）：

  ```bash
  podman exec bmp-status cat /var/lib/bmp-status/initial-admin-password.txt
  ```

- 备份：

  ```bash
  podman volume export bmp-status-data --output bmp-status-data.tar
  ```

- 恢复：

  ```bash
  podman volume import bmp-status-data bmp-status-data.tar
  ```

---

## 5. 安全与端口

- 容器以 UID/GID `10001` 运行；示例还启用了：
  - `cap_drop: ALL` / `DropCapability=ALL`
  - `no-new-privileges`
  - 只读根文件系统 + `/tmp` tmpfs
- BMP 协议本身无认证、无 TLS；`11019/tcp` 只应对路由器网段开放。
- 仪表盘、管理员面板、WebSocket 和业务 API 由同一个 Argon2 页面密码 + HttpOnly 会话 Cookie 保护；未登录访问页面会先显示登录页。
- 如果通过 HTTPS 反向代理访问（强烈建议），设置 `COOKIE_SECURE=true`。
- 不要给容器添加 `NET_ADMIN` 等能力；本项目不需要，也不应修改宿主路由表。

### 反向代理示例（Caddy）

```caddyfile
bmp.example.com {
    reverse_proxy 127.0.0.1:8787
}
```

同时将 `COOKIE_SECURE=true`，并只对可信网络开放 `11019/tcp`。

---

## 6. 常见问题

**构建时 apk / npm / cargo 很慢或失败**

使用代理构建：

```bash
podman build --format docker --network=host \
  --build-arg HTTP_PROXY=http://192.168.1.114:7890 \
  --build-arg HTTPS_PROXY=http://192.168.1.114:7890 \
  -t localhost/bmp-status:latest .
```

CERNET 源默认已启用；如果仍失败，检查 DNS、代理和
`ALPINE_MIRROR` / `DEBIAN_MIRROR` / `DEBIAN_SECURITY_MIRROR` 参数，或查看
<https://help.mirrors.cernet.edu.cn/debian/>。

**容器启动后健康检查失败**

```bash
podman logs bmp-status
podman exec bmp-status wget -qO- http://127.0.0.1:8787/api/health
```

常见原因：端口被占用、`ADMIN_PASSWORD` 少于 8 字符、数据卷权限不正确。

**BMP 实例登记后仍显示“未登记”**

- 用 host 网络时，检查 BMP 连接的真实源 IP 是否等于实例里登记的 IP；
- 用 bridge 网络时，源 IP 会被 NAT，需要改用 host 网络；
- 如果启用了白名单，未登记的源会被拒绝，并出现在管理员面板的连接记录中。

**页面密码无法登录**

- 设置了 `ADMIN_PASSWORD` 时，每次启动都会重置为该值；
- 没设置时，从 `/var/lib/bmp-status/initial-admin-password.txt` 读取；
- 也可以在容器内删除 `admin.json` 后重启，重新生成密码（会丢失实例登记）。

---

## 7. 已验证内容

本仓库的容器方案已经在 arm64 主机上实际执行过：

- Podman 5.4.2 在隔离的临时存储中构建，使用 `rust:1.94-alpine` 与 CERNET
  镜像；构建过程通过本地代理下载依赖；
- `make cross-binaries` 不依赖 qemu：x86_64 产物为 `static-pie linked`，
  arm64 产物为 `statically linked`，两者均已在对应架构上通过
  `/api/health` 探测（x86_64 通过 `qemu-x86_64-static`）；
- 此前 `make cross-images` 生成 amd64/arm64 Alpine 镜像约 17 MB / 16 MB；
  本机 manifest list 正常，`prebuilt-images/load.sh` 在全新隔离存储中加载
  arm64 归档后成功运行；
- 根 `Dockerfile` 此前单独构建的 Alpine/musl 镜像约 16.7 MB，健康检查通过；
- 容器以 UID `10001` 运行，命名卷初始属主为 `10001:10001`，
  `admin.json` 权限为 `0600`；
- 使用 busybox `wget` 验证 `/api/health`、内嵌首页、`/admin/` 与 favicon；
- 构建/运行结束后删除临时镜像、容器、卷和隔离存储；`make clean-images`
  可清理本机构建标签/manifest 及 dangling manifest list，`podman image prune -f`
  可清理剩余的 dangling 镜像。

> 认证、SQLite 运行配置和前端登录门的改动只做了本地非容器编译与接口测试，
> 没有重新执行跨架构/容器镜像构建。
>
> 具体命令和输出可在项目提交记录中查看；`prebuilt-images/` 与 `bin/` 为
> 本地产物，不进入 Git。
