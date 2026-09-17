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
| G4 | PWA / 添加到主屏幕 | ✅ 完成（ADR-0018 + #4；手动验收；iOS 存储隔离已文档化） |
| G5 | 网页内相机扫码入口 | ✅ 完成（ADR-0019 + #5；自动 31/31 + cargo 83 + 手动验收通过） |
| U1 | 上游 PR：Safari createWritable（#2） | 📤 已发上游，fork 自维护（不追求合入） |
| U2 | 上游 PR：fake-ip 过滤（#3） | 📤 已发上游，fork 自维护（不追求合入） |
| U0 | 上游无 `docs/`：AGENTS ↔ fork ADR 漂移 | ✅ 接受分叉：fork 文档自成一派（ADR-0001..0019） |

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

## G4 — PWA 可安装入口 ✅

**依赖：** G3 完成（✅ 已满足）。

### Grill 已收口（ADR-0018）

- [x] spec 已发布：`hu3rror/QuicMic#4`（ready-for-agent）；测试 seams 已确认（web/ 一致性 node 测试 + CSP header oneshot + 手动验收）
- [x] 平台范围：iOS 优先、Android 顺带；manifest 按 W3C 标准写全；不承诺 Chrome 自动安装（自签非 secure context）
- [x] 图标：独立 PNG（`web/icons/` 180/192/512），替换内联 apple-touch-icon；favicon 保持内联
- [x] **极简 SW**（仅注册 + 空 fetch 不缓存，为日后受信 CA 场景预留 Chromium 安装条件）—— 用户拍板方案 B
- [x] 不加「添加到主屏幕」引导 UI（iOS 无法探测已安装，提示无法精准消失）
- [x] `start_url: "/"`（无 hash → 走 G3 resume）、`display: standalone`；CSP `worker-src 'self'`（初始提交已有，实现时仅测试锁定）
- [x] 文档：ADR-0018 已写；CONTEXT.md 已加「Home-screen entry / 主屏入口」

### 待实现

- [x] `web/manifest.webmanifest`、`web/sw.js`、`web/icons/icon-{180,192,512}.png`；index.html 链接 manifest + apple-touch-icon 换文件；app.js 注册 SW（渐进增强）
- [x] 自动验收：`web/pwa.test.js`（manifest 字段/图标存在+尺寸/HTML 接线/SW stub 无缓存）+ CSP header oneshot 测试；fmt/clippy/test（83）与 `node --test`（24）全绿
- [x] 手动验收（设备）：iOS Safari 添加主屏幕 → standalone → resume 免 PIN ✅；**发现 iOS 存储隔离**：主屏 Web App 与 Safari 的 localStorage/cookie 完全隔离 → 图标首次打开需配一次 PIN；Safari 与图标交替使用互踢（服务端单 token 槽，ADR-0016）→ 已采纳方案 A（接受限制 + 文档化），ADR-0018/README 已记录；CSP 已在初始提交含 `worker-src 'self'`（仅测试锁定，无 header 改动）

---

## 上游相关（与 G 并行，勿绑进 persistence PR）

### U1 — PR #2 Safari `createWritable`

- [x] 行为正确（feature-detect）
- [x] 合入策略（拍板）：PR 挂上游、不追求合入，fork 自维护——上游 AGENTS 无 ADR 接受，不引 fork ADR
- [x] 可选：BCD 单源不做（不动 PR 分支）

### U2 — PR #3 fake-ip 过滤

- [x] 行为与排名逻辑正确
- [x] 可选：收紧 IPv6 注释不做（不动 PR 分支）
- [x] README 改动保留（良性 scope，随 PR 挂上游）

### U0 — 文档体系

- [x] 选定：接受上游无 ADR；fork 文档自成一派（不外链、不推动上游建 `docs/adr`）
- [x] fork `dev` 与 upstream 合入时 **不要**把 G1/G2 与 U1/U2 打成同一大 PR（约定已确认——AGENTS.md fork 段；PR 分支基于 `main` 仅含 PR 提交）

---

## G5 — 网页内相机扫码入口 ✅

**动机：** iOS 主屏 Web App 与 Safari 存储隔离（G4-A 已文档化），图标首次打开仍需输 PIN；在网页 app 内直接调用摄像头扫 PC 端 QR，免去"另开相机 app"的步骤，可提升首次配对体验。

### Grill 已收口（ADR-0019，评估通过）

- [x] 范围：纯 `web/`、服务端零改动；配对屏扫码按钮 → 复用 `doPair`；手输永远可用（渐进增强，不 gate 配对）
- [x] QR 内容不变（`URL+#PIN`，OS 相机深链依赖）；应用内只提取 `#(\d{6})$`，其余视为扫描失败
- [x] 解码：vendor jsQR 单文件（Apache-2.0 + license 头）；弃 BarcodeDetector（iOS 不可用）/ 自研（成本不成比例）
- [x] 扫描 UX：实时取景循环（降采样 ~5-10fps）；成功/取消/切后台即停相机
- [x] 权限：两次独立弹窗（扫码时相机、连接时麦克风）；iOS standalone 相机不可靠 → 已知风险 + 降级手输，真机验收定性
- [x] 语义：扫码 ≡ 手输 PIN（不动 hash/token/history）；每成功解码只 pair 一次、不自动重试（防 ADR-0012 节流锁）
- [x] 测试：qr.js 纯模块 parse 单测 + PPM fixture 解码集成测试 + iOS Safari/standalone/Android 手动验收；CI 不动
- [x] 文档：ADR-0019 已写；CONTEXT.md 已加 Pairing QR / In-app scan

### 待实现

- [x] 实现（ADR-0019 落码）+ 自动验收（qr.test 7 项，全量 node 31 + cargo 83 全绿）
- [x] 手动验收（真机）：主屏 PWA 内扫码 → 自动配对进主界面；相机权限弹窗正常；iOS standalone 内 getUserMedia 可用（当前实机定性，ADR-0019 已知风险缓解）；降级路径全过——相机拒绝授权 / 扫描中取消 / 切后台，均为 toast 提示 + 手输兑底

---

## 明确不做（除非新开目标）

- [ ] 运行时证书热换（G1 已否决；≥14 天不重启靠文档 + 重启即愈）
- [ ] 签名 token / TTL / generation（G2 已否决）
- [ ] 二进制多 OS E2E harness（G1 用模块单测 + 有限真实双启动）
- [ ] 一机多实例文件锁（文档约定单实例）

---

## 建议执行顺序

1. ~~G1~~ → ~~G2~~ → ~~G3~~ → ~~G4~~ → ~~G5~~
2. U1/U2 PR 挂上游、fork 自维护（不追求合入）；U0 已拍板接受分叉
3. 维护循环：`dev` 与 `upstream/main` 零落后（sync 时 `git merge --ff-only upstream/main` 进 `main`）；需要时 push `origin/dev`（需你确认再让 Agent push）

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
