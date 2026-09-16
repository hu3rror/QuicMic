```markdown
# QuicMic — 持久化身份与免 PIN 重连 Todo

基于 Matt 流程：每个目标先 `/skill:grill-with-docs` →（必要时 `/skill:to-spec`）→ `/skill:implement`，不一次做整包。

---

## 总目标

| 能力 | 用户感知 |
|------|----------|
| 跨重启身份稳定 | 同 LAN IP 下重启后证书 hash、PIN 不变 |
| 跨重启会话可续 | 已 pair 的页面在服务重启后仍能 renew，不必重输 PIN |
| 免 PIN 打开 | 无 `#PIN` 打开/刷新页面可直接进主界面 |
| 可安装入口 | 主屏幕打开走免 PIN 路径（可选体验） |

依赖链：**G1 → G2 → G3 → G4**

---

## 状态总览

| ID | 目标 | 状态 |
|----|------|------|
| G1 | 持久化 TLS 身份 + PIN（ADR-0015） | ✅ 完成（dev，含单测/E2E 双启动） |
| G2 | 持久化 session token + renew 跨重启（ADR-0016） | ✅ 完成（审查通过，82 测绿） |
| G3 | 客户端 localStorage + 无 hash 自动 renew | ✅ 完成（ADR-0017，20 JS 单测 + 82 cargo 测绿） |
| G4 | PWA / 添加到主屏幕 | ⬜ 等 G3 |
| U1 | 上游 PR：Safari createWritable（#2） | 🔍 审查通过，待合/文档策略 |
| U2 | 上游 PR：fake-ip 过滤（#3） | 🔍 审查通过，待合/小修注释 |
| U0 | 上游无 `docs/`：AGENTS ↔ fork ADR 漂移 | 📋 策略待定（接受 / 外链 / 上游建 ADR） |

---

## G1 — 持久化证书与 PIN ✅

**验收：** 同 data-dir 双启动 hash+PIN 一致；IP 变换证 PIN 留；≥13 天启动期轮换；损坏隔离；`--pin` / `--pin random`。

- [x] Grill Q1–Q14 收敛
- [x] ADR-0015；0006 `superseded-in-part`
- [x] `src/persistence.rs`（目录、identity.json、pin、原子写、0600）
- [x] 启动仅年龄轮换（无运行时热换）
- [x] `tls.rs` / `main.rs` 接入；SPKI 校验
- [x] 单测 + 真实双启动；fmt/clippy/test
- [x] code-review 问题已修（commit 记在 dev）

**不重做：** token、renew、前端、PWA、证书热换。

---

## G2 — 持久化 session token ✅

**验收：** 重启后旧 token 能 `/skill:api/renew`；pair/renew write-through；PIN 重置清 token；无 TTL。

- [x] Grill：存盘模型 A、PIN 联动清 token、无 TTL、`session-token` + 64-hex
- [x] ADR-0016；pair/renew 原子写；启动载入 Mutex
- [x] 缺失/损坏 → None；校验与 401 语义不变
- [x] 审查：无硬违规；可选重构（temp_dir 去重、`ResolvedPin`）可记债务
- [x] 残余风险已记 ADR：reset 时 clear 文件失败可能陈旧复活（有 warn）

**不重做：** localStorage、PWA。

---

## G3 — 客户端免 PIN 重连 ✅

**一句话：** token 进 `localStorage`；无 URL hash 时自动 renew 进主界面；有 `#PIN` 时清旧 token 再 pair。

### 流程

- [x] `/skill:grill-with-docs`（仅 Web 客户端；服务端 G1/G2 已就绪）
- [x] 决策收口后 `/skill:to-spec` 或直接 `/skill:implement`（预计单文件 `web/app.js` 量级 → 优先 implement）
- [x] `/skill:implement` + 验收（commit `a6690cf`，ADR-0017）
- [x] 文档：ADR-0017、CONTEXT（Resume / 恢复会话）、architecture.md 已同步

### Grill 开场稿

```text
/grill-with-docs

主题：Web 客户端持久化 session token，使已配对设备在无 URL hash 时
可通过 /api/renew 进入主界面，无需再输入 PIN。

已具备：ADR-0015（证书+PIN）、ADR-0016（服务端 session-token，pair/renew write-through）。
本轮只谈 web/app.js：存储位置、#PIN vs 无 hash、renew 失败与配对 UI、与现有「先 renew 再 pair」衔接。
范围外：PWA/SW/manifest、证书热换、改 /api（除非发现缺口）。
```

### 预期决策点（grill 中确认）

- [x] 存储：`safeStorage` wrapper，quota/安全异常 fail-open 降级到内存（不误报死服务器）
- [x] 有 `#PIN`：QR 即明确配对意图，先丢弃本地 token 再 pair（QR-clear）
- [x] 无 hash：validate-only resume（`/api/stats` 校验，不轮换 token；200 → 主界面 / 401 → 配对 / 503 → bounded wait）
- [x] 401（taken-over）vs 网络失败（server-gone）区分；401 就地 re-pair，server-gone 进 ~3s×~60s 有界等待后回落 Reload 锁
- [x] XSS 可读存储接受（对齐 LAN 明文 PIN 模型）

### 验收（实现后勾）

- [x] 已 pair 后刷新/新开同 origin：无 PIN 进入主界面（resume validation，多 tab 不互踢）
- [x] QR/`#PIN` 路径会重新 pair，不误用旧 token（QR-clear）
- [x] 服务端 PIN 重置后客户端 renew 失败并回到配对（401 → 就地 re-pair）
- [x] 服务未启动时的行为可预期（bounded wait ~60s → Reload 锁，服务回来自动进主界面）

---

## G4 — PWA 可安装入口 ⬜ 实现中

**依赖：** G3 完成（✅ 已满足）。

### Grill 已收口（ADR-0018）

- [x] 平台范围：iOS 优先、Android 顺带；manifest 按 W3C 标准写全；不承诺 Chrome 自动安装（自签非 secure context）
- [x] 图标：独立 PNG（`web/icons/` 180/192/512），替换内联 apple-touch-icon；favicon 保持内联
- [x] **极简 SW**（仅注册 + 空 fetch 不缓存，为日后受信 CA 场景预留 Chromium 安装条件）—— 用户拍板方案 B
- [x] 不加「添加到主屏幕」引导 UI（iOS 无法探测已安装，提示无法精准消失）
- [x] `start_url: "/"`（无 hash → 走 G3 resume）、`display: standalone`、CSP 加 `worker-src 'self'`
- [x] 文档：ADR-0018 已写；CONTEXT.md 已加「Home-screen entry / 主屏入口」

### 待实现

- [ ] `web/manifest.webmanifest`、`web/sw.js`、`web/icons/icon-{180,192,512}.png`；index.html 链接 manifest + apple-touch-icon 换文件；app.js 注册 SW
- [ ] 验收：已 pair 设备从主屏打开无需 PIN（iOS Safari 添加主屏幕；Android 手动添加）；manifest 字段/图标静态检查；fmt/clippy/test + `node --test` 保持绿

---

## 上游相关（与 G 并行，勿绑进 persistence PR）

### U1 — PR #2 Safari `createWritable`

- [x] 行为正确（feature-detect）
- [ ] 合入策略：接受 AGENTS 无 ADR，或 PR 说明见 fork ADR
- [ ] 可选：BCD 矩阵只保留一处权威来源

### U2 — PR #3 fake-ip 过滤

- [x] 行为与排名逻辑正确
- [ ] 可选：收紧 IPv6 注释（勿过度承诺 multicast）
- [ ] README 改动保留（良性 scope）

### U0 — 文档体系

- [ ] 选定：接受上游无 ADR / AGENTS 外链 fork / 推动上游建 `docs/adr`
- [ ] fork `dev` 与 upstream 合入时 **不要**把 G1/G2 与 U1/U2 打成同一大 PR

---

## 明确不做（除非新开目标）

- [ ] 运行时证书热换（G1 已否决；≥14 天不重启靠文档 + 重启即愈）
- [ ] 签名 token / TTL / generation（G2 已否决）
- [ ] 二进制多 OS E2E harness（G1 用模块单测 + 有限真实双启动）
- [ ] 一机多实例文件锁（文档约定单实例）

---

## 建议执行顺序

1. ~~G1~~ → ~~G2~~ → ~~G3~~  
2. **G4 grill → implement（可选）**  
3. U1/U2 按维护者节奏合；U0 定策略  
4. 需要时再 `origin/dev` push（需你确认再让 Agent push）

---

## 快速命令备忘

| 阶段 | 命令 |
|------|------|
| 对齐设计 | `/skill:grill-with-docs` |
| 成文 | `/skill:to-spec`（小变更可跳过） |
| 多会话切片 | `/skill:to-tickets`（G3 多半不需要） |
| 落地 | `/skill:implement` |
| 审查 | `/skill:code-review`（或等价双轴） |
```
