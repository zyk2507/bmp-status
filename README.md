# BMP Status

基于 **Rust + bgpkit-parser** 的 [BMP（BGP Monitoring Protocol）](https://datatracker.ietf.org/doc/html/rfc7854) 收集器与实时可视化面板。BIRD / FRR / 支持 BMP 的路由器把消息推送到本服务后，页面会实时展示消息流、BGP 对等体、路由前缀和流量速率；登录后可登记多个 BIRD 实例、设置来源白名单、运行数据上限和页面密码。

前端使用 **Next.js 16（App Router / 静态导出）**，构建时自动编译并通过 `rust-embed` **嵌入 Rust 二进制**，部署时只需要一个可执行文件。

## 最短上手

```bash
# 1) 构建：自动构建 Next.js 前端并嵌入 Rust 二进制
make release

# 2) 启动：BMP 监听 0.0.0.0:11019，页面/API 监听 0.0.0.0:8787
ADMIN_PASSWORD='change-this-password' make run

# 3) 打开页面并输入页面密码
#    仪表盘   http://localhost:8787/
#    管理员   http://localhost:8787/admin/  （登录后直接进入，不再重复要密码）
#    BMP 端口 tcp://<本机 IP>:11019
```

- 没有路由器：`make run ARGS="--demo"`。
- 不设置 `ADMIN_PASSWORD` 时，首次启动会生成随机页面密码并写入 `data/initial-admin-password.txt`。
- 仪表盘、管理员面板、WebSocket 和所有业务 API 共用这一个页面密码；登录后使用 HttpOnly Cookie 自动登录。
- BIRD / FRR 接入配置见下方 [接入 BIRD / FRR](#接入-bird--frr)；实例登记和白名单在 `/admin/`。
- 两台 RustyBGP + RIPE RIS 的完整模拟测试：`RUSTYBGP_DIR=/tmp/rustybgp make e2e`（详见 [真实端到端测试](#真实端到端测试rustybgp--ripe-ris)）。

## 功能一览

- **BMP v3 TCP 收集器**：Tokio accept loop、逐连接任务、6 字节公共头 + 长度分帧、最大消息长度限制、空闲超时、TCP keepalive、最大并发连接数、优雅关闭。
- **bgpkit-parser 解析**：Route Monitoring、Statistics Report、Peer Up / Down、Initiation、Termination、Route Mirroring，以及内嵌 BGP UPDATE 的 ORIGIN / AS_PATH / NEXT_HOP / MED / LOCAL_PREF / COMMUNITIES / LARGE_COMMUNITIES / MP_REACH。
- **实时可视化**：消息速率曲线、通告/撤销堆叠柱、消息类型环图、Top Origin AS / Peer AS、对等体会话状态、实时消息流、原始 hex 详情、由通告/撤销重建的当前前缀表（Adj-RIB-In）。
- **多实例管理**：管理员面板登记 BIRD / FRR 实例（名称、BMP 源 IP、描述、启用状态），仪表盘按实例名称展示和筛选。
- **来源白名单**：可选“仅允许已登记实例接入”；回环地址始终允许（方便本机 BIRD 与演示数据）。
- **页面安全**：Argon2id 密码哈希、HttpOnly + SameSite=Lax 会话 Cookie（12 小时）、登录失败限速、修改密码后使其他会话失效、可选 Secure Cookie。
- **内嵌前端 + 单二进制**：`cargo build` 时自动执行 `npm run build` 并嵌入静态资源；`--frontend-dir` 可在开发时改用磁盘目录。
- **合成演示流**：通过真实 TCP 连接向本机 BMP 端口发送合法的 BMP v3 消息，走完整解析链路，便于没有路由器时体验。

## 架构

```text
BIRD / FRR / Router ──BMP/TCP :11019──┐
                                      ▼
                          ┌───────────────────────┐
                          │ Tokio BMP listener    │
                          │ framing / limits /    │
                          │ allowlist / idle      │
                          └──────────┬────────────┘
                                     │ bgpkit-parser::parse_bmp_msg
                                     ▼
                          ┌───────────────────────┐
                          │ AppState (in-memory)  │
                          │ peers / RIB / buckets │
                          │ counters / ring buffer│
                          └──────┬──────────┬─────┘
                                 │          │
                    REST /api/*  │          │  WebSocket /ws
                                 ▼          ▼
                          ┌───────────────────────┐
                          │ Next.js dashboard     │
                          │ (embedded in binary)  │
                          └───────────────────────┘
                                 ▲
                    Argon2 + HttpOnly cookie
                                 │
                    ┌────────────┴────────────┐
                    │ / 登录 → 仪表盘 /admin  │
                    │ 实例 · 白名单 · 上限    │
                    └─────────────────────────┘
```

目录：

```text
status-page/
├── backend/                 # Rust collector + HTTP/WS 服务
│   ├── build.rs             # 构建时调用 npm run build（可用 BMP_SKIP_FRONTEND=1 跳过）
│   └── src/
│       ├── main.rs          # 启动、信号、优雅关闭
│       ├── bmp.rs           # Tokio BMP 连接管理
│       ├── demo.rs          # 合成 BMP 数据流（编码器）
│       ├── api.rs           # REST / WebSocket / 管理 API
│       ├── admin.rs         # Argon2 页面密码、会话、实例持久化
│       ├── runtime_config.rs # SQLite 运行数据上限
│       ├── assets.rs        # rust-embed 静态资源
│       ├── model.rs         # JSON 视图模型
│       └── state.rs         # 消息处理、RIB、聚合
├── frontend/                # Next.js 16 静态导出前端
│   ├── app/                 # / 与 /admin
│   ├── components/
│   └── lib/
├── tools/ripe-replay/       # RIPE RIS → RustyBGP gRPC 路由回放工具
├── tests/e2e-rustybgp/      # 两台北向 RustyBGP + BMP 端到端测试
├── docs/deploy/             # Podman Compose / systemd Quadlet 部署示例
├── container/               # Alpine 运行时 Dockerfile + 数据目录占位文件
├── Dockerfile               # 自包含多阶段构建（Alpine/musl 运行时）
├── scripts/                 # 前端/后端/交叉编译/镜像/代理检测脚本
├── .dockerignore
├── Makefile                 # 构建、交叉编译、镜像打包、清理
├── bin/                     # 构建后生成：本机/交叉二进制（不入库）
├── prebuilt-images/         # 构建后生成：amd64/arm64 镜像归档（不入库）
└── data/                    # admin.json / runtime.sqlite3 / 初始密码 / RIPE 缓存（不入库）
```

## 快速开始

### 环境要求

- Rust 1.85+（当前使用 2024 edition）
- Node.js 20+ 与 npm（仅构建时需要）
- C 编译器（`rusqlite` 使用 bundled SQLite；Debian/Ubuntu 安装 `build-essential`）
- Podman 5+（仅交叉编译和容器镜像时需要；交叉构建脚本会安装 Zig 作为 SQLite 的 C 交叉编译器）
- 网络受限时先设置代理（本项目开发时使用 `192.168.1.114:7890`）：

```bash
export http_proxy=http://192.168.1.114:7890
export https_proxy=http://192.168.1.114:7890
```

### 构建与运行

```bash
# build.rs 会自动 npm install（如缺依赖）+ npm run build，并嵌入前端
make release

# 启动：BMP 监听 0.0.0.0:11019，HTTP/WS 监听 0.0.0.0:8787
make run

# 或者直接运行二进制（可在任意目录，前端已嵌入）
./backend/target/release/bmp-status \
  --bmp-bind 0.0.0.0:11019 \
  --http-bind 0.0.0.0:8787
```

打开：

- 仪表盘：<http://localhost:8787/>
- 管理员面板：<http://localhost:8787/admin/>

首次启动会自动生成页面密码，并输出到日志、写入 `<data-dir>/initial-admin-password.txt`（`make run` 时为项目根目录的 `data/`）（权限 `0600`）。仪表盘、管理员面板、WebSocket 与业务 API 共用该密码。也可以显式设置/重置：

```bash
ADMIN_PASSWORD='请换一个足够强的密码' make run
# 等价于 --admin-password
```

> 设置了 `ADMIN_PASSWORD` 时，每次启动都会用该值重置页面密码。

### 没有路由器？启动演示数据

```bash
make run ARGS="--demo"
```

演示流从 `127.0.0.1` 建立真实 TCP 连接，生成 4 个对等体（AS13335 / AS15169 / AS6939 / AS9808）的 Route Monitoring、Statistics、Peer Up/Down、Initiation 和 Termination 消息。该功能只用于开发/演示，不在前端管理界面提供入口。

### 开发模式

```bash
make dev
# 后端 http://localhost:8787（--frontend-dir 指向磁盘构建目录）
# 前端 http://localhost:3000（Next.js dev，HMR）
```

开发模式下前端 API 请求直连 `http://<hostname>:8787`（后端已配置带凭据的 CORS）；WebSocket 同样直连 `:8787`。如果修改了后端端口，请同时设置 `NEXT_PUBLIC_API_BASE` / `NEXT_PUBLIC_WS_URL` 并重新启动 `next dev`。

> `make dev` 需要先在 `frontend/` 执行过一次构建吗？不需要，`build.rs` 会为 Rust 侧准备 `frontend/out`，Next dev 自己使用 `.next`。

## 构建、交叉编译与打包

Makefile 提供完整的构建流水线，所有目标都只依赖本机已有的 Rust/Node.js
或 Podman：

| 目标 | 产物 |
| --- | --- |
| `make frontend` | `frontend/out`（Next.js 静态导出） |
| `make frontend-force` | 清理 `.next/out` 后重新构建前端 |
| `make backend` | 本机 Rust 工具链构建，并复制到 `bin/bmp-status-linux-<arch>-native` |
| `make backend-force` | 清理后端包后重新构建 |
| `make single-binary` | 前端 + 后端合并为单个本机二进制 |
| `make cross-binaries` | 在 `rust:1.94-alpine` 容器中交叉编译 `bin/bmp-status-linux-{amd64,arm64}-musl` |
| `make cross-images` | 基于 musl 二进制构建两个架构的 Alpine 运行时镜像，保存到 `prebuilt-images/` |
| `make all` | 依次完成本机二进制、交叉二进制和跨架构镜像 |
| `make clean-images` | 删除本机构建的镜像标签/manifest（不删除 `prebuilt-images/`） |
| `make clean` | 删除前端/后端构建输出、`bin/`、`prebuilt-images/` 和镜像 |

`bin/` 与 `prebuilt-images/` 只在构建成功后创建，且已加入 `.gitignore`。
交叉二进制使用 musl 静态链接，镜像使用 `alpine:3.22` 作为运行时，体积约
预计约 18 MB；容器内不包含 apt/curl，健康检查使用 busybox 的 `wget`。镜像构建
结束时会定向清理带 BMP Status 标签的 dangling 镜像；`make clean-images`
还会清理所选存储中的 dangling manifest list。

```bash
# 常用流程
make single-binary      # 本机可直接运行的单一二进制
make cross-binaries     # amd64/arm64 静态 musl 二进制
make cross-images       # 两个架构的 Alpine 镜像 + prebuilt-images/load.sh
make all                # 全部完成
```

跨架构构建不依赖 qemu：在 arm64 主机上使用原生 arm64 的 Rust 容器，x86_64
目标通过 `rust-lld` 链接纯 Rust 依赖；反之亦然。构建容器会优先使用
**CERNET（教育网联合镜像站）** 的 Alpine apk / Debian apt 源，说明见
<https://help.mirrors.cernet.edu.cn/debian/>。

中国网络下，容器脚本会自动检测并提示输入代理（例如
`http://192.168.1.114:7890`），缓存到被忽略的 `.build.env`。也可以显式传入：

```bash
PROXY=http://192.168.1.114:7890 make cross-images
FORCE_CN=1 make cross-images       # 强制按中国网络处理
ALLOW_NO_PROXY=1 make cross-images # 明确允许直连
```

删除 `.build.env` 可清除缓存的代理，下一次构建会重新检测并提示。

CI 或多套构建环境可使用隔离的 Podman 存储，构建结束后直接删除目录：

```bash
PODMAN_ROOT=/tmp/bmp-podman/storage \
PODMAN_RUNROOT=/tmp/bmp-podman/run \
make cross-images
```

## 容器化部署

提供 **Podman Compose** 与 **systemd Quadlet** 两种配置示例。推荐先用
`make cross-images` 构建跨架构镜像，或把 `prebuilt-images/` 复制到目标机器后
执行其中的 `load.sh`：

```bash
# 构建镜像（amd64 + arm64）
make cross-images

# 目标机器：加载本机架构的镜像并打上 latest 标签
# （rootful Quadlet 请用 sudo ./prebuilt-images/load.sh）
./prebuilt-images/load.sh

# Podman Compose
cp docs/deploy/bmp-status.env.example docs/deploy/bmp-status.env
chmod 600 docs/deploy/bmp-status.env
(cd docs/deploy && podman compose -f podman-compose.yaml up -d)

# systemd Quadlet（rootful）
sudo install -d /etc/containers/systemd /etc/bmp-status
sudo cp docs/deploy/bmp-status.volume docs/deploy/bmp-status.container /etc/containers/systemd/
sudo cp docs/deploy/bmp-status.env.example /etc/bmp-status/bmp-status.env
sudo systemctl daemon-reload && sudo systemctl start bmp-status
```

根目录 [`Dockerfile`](Dockerfile) 也可单独构建当前架构的镜像：Node.js 构建
前端，`rust:1.94-alpine` 编译静态 musl 二进制，最后使用 `alpine:3.22`
运行时。详细的 host/bridge 网络、数据卷、权限、健康检查、升级、清理、
rootless Quadlet 与反向代理说明见
[`docs/deploy/README.md`](docs/deploy/README.md)。

> 容器示例使用 host 网络以保留 BMP 真实源 IP；若改成 bridge，白名单将看到
> NAT 网关地址。容器默认以非 root 运行，示例还启用了 `cap_drop: ALL`、
> `no-new-privileges` 和只读根文件系统。

## 配置

| 参数 | 环境变量 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--bmp-bind` | `BMP_BIND` | `0.0.0.0:11019` | BMP TCP 监听地址 |
| `--http-bind` | `HTTP_BIND` | `0.0.0.0:8787` | 仪表盘 / API / WebSocket 监听地址 |
| `--frontend-dir` | `FRONTEND_DIR` | 未设置 | 开发用：从磁盘目录提供前端，覆盖内嵌资源 |
| `--data-dir` | `DATA_DIR` | `data` | `admin.json`、`runtime.sqlite3` 与初始密码文件所在目录 |
| `--max-msg-size` | `MAX_MSG_SIZE` | `16777216` | 单条 BMP 消息最大字节数 |
| `--idle-timeout` | `IDLE_TIMEOUT` | `600` | BMP 连接空闲关闭时间（秒，最小 5） |
| `--max-connections` | `MAX_CONNECTIONS` | `256` | 最大并发 BMP 连接数 |
| `--demo` | `DEMO` | `false` | 启动时开启合成演示流 |
| `--demo-interval-ms` | `DEMO_INTERVAL_MS` | `350` | 演示消息间隔（毫秒，20–10000） |
| `--admin-password` | `ADMIN_PASSWORD` | 未设置 | 设置/重置页面密码（至少 8 字符） |
| `--cookie-secure` | `COOKIE_SECURE` | `false` | 为管理会话 Cookie 加 `Secure`（HTTPS 部署时开启） |
| - | `RUST_LOG` | `bmp_status=info` | 日志级别，如 `RUST_LOG=debug` |

构建相关环境变量：

- `BMP_SKIP_FRONTEND=1`：跳过 `npm run build`，复用现有 `frontend/out`；若不存在则写入一个占位页。适用于没有 Node.js 的构建机。
- `NEXT_PUBLIC_API_BASE` / `NEXT_PUBLIC_WS_URL`：前端构建时覆盖 API / WebSocket 地址（一般不需要）。

## 接入 BIRD / FRR

### BIRD 2.16+

```text
protocol bmp {
    station address ip 192.168.1.50 port 11019;
    monitoring rib in pre_policy;
    # 也可以同时开启 post_policy
    # monitoring rib in post_policy;
}
```

BIRD 会把 BMP 消息推送到 `192.168.1.50:11019`。请在管理员面板登记 **BMP TCP 连接的源 IP**（通常是 BIRD 所在主机的路由源地址；经过 NAT 时填写 NAT 后的地址），名称会显示在仪表盘上。

### FRR

```text
bmp targets bmp-status
 bmp connect 192.168.1.50 port 11019 min-retry 1000 max-retry 10000
 bmp monitor ipv4 unicast pre-policy
```

> 不同版本的 BIRD / FRR 语法可能略有差异，请以对应版本文档为准。BMP 是无认证的明文 TCP 协议，请用防火墙 / VPN 限制 11019 端口的访问来源。

## 页面密码与权限

- **统一页面密码**：仪表盘 `/`、管理员面板 `/admin/`、WebSocket `/ws` 和所有业务 API 共用同一个密码。
- 首次访问 `/` 时显示登录页；登录成功后写入 HttpOnly 会话 Cookie。
- Cookie 自动登录：关闭页面、重新打开或刷新都会通过 `/api/auth/session` 恢复会话，直到过期或退出登录。
- 管理员面板不再单独显示登录框；未登录访问 `/admin/` 会返回仪表盘登录页。
- `/api/auth/*` 的登录/会话接口和 `/api/health` 探针保持公开；其他业务 API 全部需要会话。
- 密码哈希与实例登记保存在 `<data-dir>/admin.json`，原子写入、权限 `0600`。
- 运行时上限保存在 `<data-dir>/runtime.sqlite3`；不保存密码。
- 会话 Cookie：`HttpOnly; SameSite=Lax; Path=/; Max-Age=43200`；通过 HTTPS 部署时设置 `COOKIE_SECURE=true`。
- 登录失败 5 次后按来源 IP 锁定 60 秒。
- 生产环境建议：Nginx/Caddy 终止 TLS，只对外开放 HTTPS；BMP 端口只允许路由器网段访问；将 `data/` 放在受保护位置。

管理员面板功能：

- 登记多个 BIRD / FRR 实例（名称、源 IP、描述、启用/禁用）。
- 开启来源白名单后，未登记源 IP 的新 BMP 连接会被拒绝；已有连接不受影响。
- 查看活动/历史 BMP 连接、拒绝记录、服务端配置。
- 设置消息/路由/peer 的内存保留上限（保存到 SQLite）。
- 修改页面密码（Argon2id，至少 8 字符）。

登录后，浏览器持有 HttpOnly Cookie；前端 API 请求自动携带凭据，WebSocket 也会校验会话。管理接口在服务端统一由认证中间件保护，前端隐藏页面不是唯一防线。

## 仪表盘路由脉冲

活跃前缀卡片会在实时消息中检测路由变化：

- 通告数大于撤销数：卡片和数值短暂闪绿色；
- 撤销数大于通告数：短暂闪红色；
- 如果一条消息同时包含通告和撤销且数量相等，不触发脉冲。

脉冲只影响显示，不改变统计口径。路由表本身使用服务端分页，不再一次性加载全部前缀。

## API

认证接口（公开）：

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST | `/api/auth/login` | 页面密码登录，成功后设置 HttpOnly Cookie |
| GET | `/api/auth/session` | 自动登录/会话状态 |
| POST | `/api/auth/logout` | 退出并清除 Cookie |
| GET | `/api/health` | 公开健康探针，仅供容器/负载均衡检查 |

业务接口（需要会话 Cookie）：

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/api/overview` | 聚合指标、对等体、实例、最近 120 秒桶 |
| GET | `/api/messages?limit=100&type=all&q=` | 最近消息（`type`：`all`/`updates`/`peer`/`errors`/具体类型；`q` 搜索） |
| GET | `/api/routes?page=1&limit=50&q=&peer=&router=` | 当前重建的 RIB 分页快照；单页最多 200 条 |
| GET | `/api/peers` | 对等体列表 |
| WS | `/ws` | `overview` / `message` / `lagged` 事件流；握手和会话过期都会校验 |

管理接口（需要会话 Cookie）：`/api/admin/password`、`/api/admin/instances`（GET/POST）、`/api/admin/instances/{id}`（PUT/DELETE）、`/api/admin/settings`（PUT）、`/api/admin/runtime`（GET/PUT）、`/api/admin/connections`、`/api/admin/demo/start`、`/api/admin/demo/stop`。

## 内存与分页

BMP 解析出来的消息、peer 状态和活跃路由都保存在内存中；运行上限由管理员面板 / SQLite 动态配置：

- 消息缓存：默认 **500** 条；可设置为 `0`（不限制）或任意正整数。
- 活跃路由：默认 **不限制**；超过配置上限时按最早更新的顺序淘汰。
- Peer 状态：默认 **不限制**；超过上限时淘汰最久未活动的 peer，并清理其路由。
- 运行配置保存在 `<data-dir>/runtime.sqlite3`；修改后立即裁剪当前内存数据，不需要重启。
- 原始 hex 每条消息最多保留前 **1024** 字节，避免单条超大消息占满内存。
- 路由 API 使用服务端分页：默认 50 条/页，单页最多 200 条；浏览器只保留当前页。
- 前端消息流缓存最多 400 条、展示最多 200 条；路由表只保留当前页 50 条。
- RIB 由 Route Monitoring 的 Announce / Withdraw 维护；Peer Down 会清除该对等体的活跃前缀。
- BMP 连接断开时，相关对等体会话标记为 `unknown`，但已重建的路由保留（BGP 会话可能仍然存活）。

## 大 RIB 性能设计

面向约百万级路由、多个 BMP peer 的场景，`high-perf` 分支把 RIB 和查询路径做了压缩：

- 路由内部使用紧凑的 `RouteRecord`，不再为每条路由立即构造完整的 JSON 视图；
- 只有在 API 返回当前页时，才按页生成 50 条 `RouteEntry`；
- peer 元数据、next-hop、origin、community 等低基数字符串使用 `Arc<str>` 和有限大小的 interner 共享；
- 更新顺序使用 `BTreeMap` 索引，未筛选的路由页不需要排序或克隆整张 RIB；
- 带搜索/peer/实例筛选时只做一次扫描并仅物化当前页，不创建全量临时 `Vec<RouteEntry>`；
- 路由 HashMap 使用 `AHashMap`，mass withdrawal 后会按高水位/活跃数量比释放多余容量；
- 前端路由表仍然只保留当前页 50 条，浏览器不会加载整张 RIB。

这些优化不改变公开 API 字段、路由筛选、搜索、分页、白名单或页面行为。

## 测试

```bash
# Rust 单测：演示 BMP 编码 → bgpkit-parser 解析回环
cd backend && cargo test

# 前端类型检查 + 静态构建
cd frontend && npm run typecheck && npm run build
```

手工验证 BMP 端口：

```bash
# 另开终端，确认端口监听
ss -ltnp | grep 11019

# 健康检查
curl http://127.0.0.1:8787/api/health
```

### 真实端到端测试：RustyBGP + RIPE RIS

仓库包含一个完整的两台 BGP 路由器模拟测试：

```text
RIPE RIS MRT (缓存) ──bgpkit-parser──▶ ripe-bgp-replay ──gRPC──▶ rusty-r2 (AS65002)
                                                                    │ eBGP
                                                                    ▼
                                                              rusty-r1 (AS65001)
                                                                    │ BMP pre-policy
                                                                    ▼
                                                              bmp-status :11019
```

```bash
# 首次运行会自动 clone/构建 RustyBGP；也可复用已有源码：
RUSTYBGP_DIR=/path/to/rustybgp ./tests/e2e-rustybgp/run.sh

# 或
make e2e
```

- RIPE 历史日志使用 RRC26 的 `updates.20250101.0000.gz`（约 275 KB）。过滤后约有 24,845 条可用通告，包含 704 个唯一 IPv4 前缀和 551 个唯一 IPv6 前缀；测试默认注入 704 条 IPv4 + 300 条 IPv6。第一次运行下载并缓存到 `data/ripe-cache/`，之后直接读缓存。
- 所有路由都来自 RIPE RIS 日志，不读取系统 RIB 作为数据源。路由通过 GoBGP 兼容 gRPC `AddPathStream` 注入 `rusty-r2` 的 Loc-RIB；两台 RustyBGP 通过真实 IPv4 + IPv6 eBGP 交换，`rusty-r1` 用真实 BMP v3 连接发给本项目。
- **绝不写系统路由表**：测试配置中完全没有 `zebra` 段，因此 RustyBGP 的 kernel FIB 集成与 next-hop tracking（NHT）保持关闭；脚本拒绝以 root 运行，检查进程没有 `CAP_NET_ADMIN`，并在测试前后对完整 IPv4/IPv6 路由表做逐字节 diff，任何变化都会立即失败。
- RIPE next hop（如 `185.1.8.50`）在测试环境不可达；由于没有 NHT，路由仍会正常进入 BGP RIB 并通过 BMP 上报，正好验证“纯模拟、不做可达性检测”。
- 输出纪律：回放工具只打印计数、Top origin AS 和每个地址族最多 2 条样例；测试脚本只打印断言结果、计数和一条样例路径，不会把完整 RIB 输出到终端/AI 窗口。失败时路由 diff 也只显示前 60 行。
- 当前 E2E 覆盖范围：**IPv4 unicast 与 IPv6 unicast 均已完成真实 BGP + BMP 端到端验证**（1,004 条活跃前缀：704 IPv4 + 300 IPv6）。
- 详见 [`tests/e2e-rustybgp/README.md`](tests/e2e-rustybgp/README.md)。

## 已知限制

- 仅支持 BMP v3（RFC 7854）；不支持 TLS 或 BMP 认证（协议本身没有）。
- 单进程、内存态；重启后统计、会话、RIB 都会清空（实例与密码哈希除外）。
- 实例匹配以 BMP TCP 源 IP 为准；同一 NAT 后的多个实例无法区分。
- `--frontend-dir` 仅用于开发；正式发布建议不设置，使用内嵌前端。
- 当前未做前端虚拟滚动；消息流滚动展示最近 200 条，路由表改为服务端分页（每页 50 条），可翻阅全部活跃路由。
- 端到端测试使用回环地址和 RIPE RIS 历史数据；不写入系统路由表。
