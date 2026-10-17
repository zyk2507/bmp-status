# RustyBGP + RIPE RIS 端到端测试

这是一个**纯模拟**测试：两台上游 RustyBGP 路由器通过真实 BGP 会话交换从 RIPE RIS 历史日志回放的路由，其中 `rusty-r1` 通过真实 BMP v3 连接把路由发送给本项目。

## 拓扑

```text
RIPE RIS updates (rrc26, 2025.01.01 00:00, cached .gz)
        │  bgpkit-parser 解析
        ▼
tools/ripe-bgp-replay ──gRPC AddPathStream──▶ rusty-r2 (AS65002)
                                                    │
                          eBGP IPv4 127.0.0.1:1179 / 127.0.0.2:2179
                          eBGP IPv6 [::1]:1179 / [::1]:2179
                                                    ▼
                                              rusty-r1 (AS65001)
                                                    │ BMP pre-policy
                                                    ▼
                                        bmp-status :11019 / :8787
```

- 两台 RustyBGP 路由器在回环地址上建立两条真实 eBGP 会话：IPv4（`127.0.0.1` / `127.0.0.2`）和 IPv6（`::1`，使用不同端口）。
- **所有路由数据只来自 RIPE RIS MRT 文件**（`bgpkit-parser` 解析），不读取、也不使用系统路由表作为来源。系统路由表仅用于测试前后的只读 diff 验证。
- RIPE 路由通过 GoBGP 兼容的 gRPC `AddPathStream` 注入 `rusty-r2` 的 Loc-RIB，然后由 BGP 正常通告给 `rusty-r1`。
- `rusty-r1` 配置了 `bmp-servers`，以 **pre-policy** 模式把收到的 IPv4/IPv6 路由发到本项目的 BMP 端口。
- 路由的 next hop 来自 RIPE（IPv4 如 `185.1.8.50`，IPv6 如 `2001:7f8:73::c188:0:1`），在测试环境不可达。RustyBGP **没有开启 kernel/zebra 集成，因此没有 NHT 可达性检查**，这些路由仍会被接受并通过 BMP 上报。

## 安全保证：绝不写系统路由表

RustyBGP 的 kernel/FIB 集成和 next-hop tracking 只会在以下情况启用：

- 配置文件中的 `zebra` 段（`enabled: true`），或
- gRPC `EnableZebra` RPC。

测试配置**完全没有 `zebra` 段**，回放工具只调用 `AddPathStream`。脚本还额外做了这些保护：

- **拒绝以 root 运行**，避免任何意外的 FIB 权限。
- 启动前检查配置中不含 `zebra`。
- 启动后检查两个 `rustybgpd` 进程的 `CapEff` 不包含 `CAP_NET_ADMIN`。
- 测试前后分别快照 `ip -4 route show table all` 与 `ip -6 route show table all`，只要有任何差异就判定失败。
- 检查 RustyBGP 日志中从未出现 `kernel route integration enabled`。
- 检查回放的 RIPE 前缀没有出现在内核 RIB 中。
- 任何失败都会清理测试进程。

我们已经在非 root 用户下验证：测试结束后系统 IPv4/IPv6 路由表逐字节不变，`proto bird` 等既有路由不受影响。

## 当前覆盖范围

- E2E 同时覆盖 **IPv4 unicast 和 IPv6 unicast**。
- IPv4 和 IPv6 路由都来自同一份 RIPE RIS MRT 日志，由 `bgpkit-parser` 解析；没有从系统 RIB 提取任何路由。
- 测试使用两条真实 eBGP 会话：IPv4 会话承载 IPv4 路由，IPv6 会话承载 IPv6 路由，`rusty-r1` 通过同一 BMP 连接上报两个对等体。
- 测试断言 IPv6 对等体（`::1`, AS65002）为 `ipv6=true`、会话 up，且 BMP 消息和重建 RIB 中都存在 IPv6 前缀与 IPv6 next hop。

## 输出纪律

- 回放工具只输出：缓存状态、按地址族的解析/注入计数、Top origin AS，以及**每个地址族最多 2 条样例路由**。
- 测试脚本只输出计数、断言结果、样例匹配前缀和一条 AS path。
- 完整 RIB 只保存在内存/API 中，不打印到终端/AI 窗口。
- 失败时只打印日志尾部（tail）和路由表 diff 的前 60 行；完整内容写入 `.cache/e2e-rustybgp/`。
- RIPE MRT 文件缓存在 `data/ripe-cache/`，已加入 `.gitignore`。

## 运行

```bash
# 网络受限时（下载依赖 / 首次拉取 RIPE 日志）
export http_proxy=http://192.168.1.114:7890
export https_proxy=http://192.168.1.114:7890

# 从零开始：会自动 clone + 构建 RustyBGP（需要 protoc、git、cargo）
./tests/e2e-rustybgp/run.sh

# 复用已有 RustyBGP 源码/构建，跳过 clone 和编译
RUSTYBGP_DIR=/tmp/rustybgp ./tests/e2e-rustybgp/run.sh

# 或通过 Makefile
make e2e
```

常用变量：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `RUSTYBGP_DIR` | `.cache/rustybgp` | RustyBGP 源码/构建目录 |
| `RUSTYBGP_BIN` | `<RUSTYBGP_DIR>/target/debug/rustybgpd` | 直接指定二进制 |
| `RIPE_URL` | RRC26 `updates.20250101.0000.gz` | RIPE RIS MRT 文件 |
| `RIPE_CACHE` | `data/ripe-cache` | 下载缓存目录 |
| `LIMIT` | `2000` | 最多注入的 IPv4 唯一前缀数 |
| `LIMIT_V6` | `300` | 最多注入的 IPv6 唯一前缀数 |
| `MIN_ROUTES` | `100` | 期望最少出现在仪表盘的路由数 |
| `KEEP_RUNNING` | `0` | `1` 时测试成功后保留服务，便于打开页面查看 |
| `BMP_BIND` / `HTTP_BIND` | `127.0.0.1:11019` / `127.0.0.1:8787` | 本项目监听地址 |
| `R1_BGP_PORT` / `R2_BGP_PORT` | `1179` / `2179` | 两台 RustyBGP 的 BGP 监听端口 |
| `R1_API_PORT` / `R2_API_PORT` | `50051` / `50052` | gRPC API 端口 |

保留现场查看：

```bash
KEEP_RUNNING=1 RUSTYBGP_DIR=/tmp/rustybgp ./tests/e2e-rustybgp/run.sh
# 输出会给出 dashboard / admin 地址、密码和三个进程 PID
# 查看结束后手动 kill 这三个 PID
```

RRC26 的这份 updates 文件约 **275 KB**。过滤后约有 **24,845 条可用通告**，包含 **704 个唯一 IPv4 前缀** 和 **551 个唯一 IPv6 前缀**；测试默认注入 704 条 IPv4 + 300 条 IPv6。第一次运行会下载并缓存；后续运行会直接读取缓存，不重复消耗流量。

## 测试通过时包含

- 两台 RustyBGP 的 IPv4 与 IPv6 eBGP 会话均 Established。
- BMP `PeerUp` 与 **1,004 条** Route Monitoring 消息被本项目解析（0 parse error）。
- 仪表盘活跃前缀 **1,004**（IPv4 704 / IPv6 300）。
- 两个对等体在线：`127.0.0.2 AS65002`（IPv4）和 `::1 AS65002`（IPv6，`ipv6=true`）。
- 实例 `rusty-r1` 在线，BMP 消息来源 `127.0.0.1`。
- IPv6 前缀具有 IPv6 next hop（来自 RIPE），AS Path 以 `65002` 开头并带有 RIPE 原始路径。
- 系统 IPv4/IPv6 路由表与测试前逐字节一致。
