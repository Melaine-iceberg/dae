# SFTP 主机密钥：按 OpenSSH 的 `known_hosts` 做 TOFU（`accept-new`）

> **状态：已实施**。`src-tauri/src/file_system/known_hosts.rs` 是策略与存储，
> `sftp.rs` 的 `KnownHostsHandler` 是接线点。单元测试 10 个（策略/文件）+ 2 个（handler 接线），
> `cargo test --lib` 344 个全过。
> **没有对真实 SSH 服务器跑过**（本机没开 sshd），这一条记在 §5。

## 0. 结论先行

**SFTP 之前接受任何主机密钥**（`AcceptAnyHostKey`，见旧注释 "verify against a known-hosts store
(TOFU) once one is persisted"）。也就是说同一网段里任何人做 ARP/DNS 欺骗就能拿到密码和整棵树。
现在改成 **OpenSSH 的 `accept-new` 语义**：

- 未知主机 → 记录密钥（SHA-256 指纹写日志），连接继续；
- 已知主机 → 必须与记录一致，不一致**拒绝连接**，报错里同时给出两枚指纹、行号、
  `ssh-keygen -R "<host>"` 的修复命令；
- **主机在文件里、只是没记过它现在用的密钥类型** → 也拒绝（不追加新行）。这是 OpenSSH
  `accept-new` 的 `known_hosts_contains_hostname` 规则：用"换个算法"绕过钉住的密钥，
  正是中间人会做的事；
- 记录不下来（文件/目录不可写、没有 home）→ **拒绝**，不降级成"当作受信"。

## 1. 为什么用用户自己的 `~/.ssh/known_hosts`，而不是应用私有的 JSON

- **信任决定属于这台机器，不属于某个应用。** 用户用 `ssh` 接受过的主机（或用 `ssh-keyscan`
  钉过的）在这里直接可用，不需要第二套"应用说了算"的信任。
- **可审计、可修复，工具现成**：`ssh-keygen -F host` 查、`ssh-keygen -R host` 删。
  拒绝信息里给的就是后者。
- **格式的坑不用自己踩**：非 22 端口的 `[host]:port`、逗号分隔的主机列表、`|1|` 加盐哈希条目，
  这些匹配规则是文件格式的一部分；`russh` 的 `known_hosts` 读取器已经实现并测过。
  另起一份私有存储等于把这些规则再实现一遍，只会错在细节上。

## 2. 为什么不做"首次连接弹窗确认"

`russh` 的 `Handler::check_server_key` 在**握手中间**被调用，只能回答是/否；而所有连接入口
（导航到 `sftp://…`、连接对话框的测试、启动时恢复的连接）都是同步命令，没有向 UI 提问的通道。
要做 `ask` 语义，得把握手拆成两步、把"待确认指纹"暂存在后端并在前端做一次模态 —— 而首次连接
的确认框是用户学会无脑点"继续"的那种对话框，收益低于 `accept-new` + 记录指纹。
**重新评估条件**：如果连接对话框愿意加一步"核对指纹"（用户从服务器侧拿到指纹再比对），那就升级成
`ask`；那时 `Banner`/`@cert-authority` 也一并处理。

## 3. 主机证书（`*-cert-v01@openssh.com`）

没有 `@cert-authority` 支持，所以**不信任签发者**。现在的行为是：把证书里的公钥当作普通主机密钥钉住
（同一把密钥续签仍可用，换钥即拒），并在日志里写明"签发 CA 未被核对"。
这条比"整条拒绝"对用户更可用，也比"信 CA"更保守。

## 4. 实现细节

- 主机名在比较前 `trim` + 小写（OpenSSH 也是这么折叠的）；非 22 端口按 `[host]:port` 归档。
- **写入自己实现**（`record`），而不是用 `russh::known_hosts::learn_known_hosts_path`：
  后者在文件末尾不是换行时会先补一个换行，于是**全新的 `known_hosts` 会出现一个空的首行**。
  这是用户的 `~/.ssh`，写进去的样子应当和他自己的工具写出来的一致。格式仍是
  `name SP key`，密钥来自 `PublicKey::to_openssh()`。
- 新建的文件在 Unix 上设 `0600`（`ssh` 的默认）；已存在的文件不改权限。
- **哈希条目**（`HashKnownHosts yes` 是 OpenSSH 的默认）能读能匹配：`|1|` 盐 + HMAC 的
  规则在 `russh` 的读取器里。但我们**新写入的条目是明文主机名** —— 故意的：这个文件的用途
  之一是可审计（`ssh-keygen -F`），而哈希只防「看过这个文件的人」知道去过哪些服务器。
- 读写是同步的，且发生在 tokio worker 上：一次连接一次、文件是 KB 级、握手在等这个答案，
  `spawn_blocking` 只会多一个任务。
- 拒绝原因通过 `Arc<Mutex<Option<String>>>` 从 handler 交给 `connect_session`，
  以 `FileSystemError::PermissionDenied` 返回 —— 否则用户只会看到 `russh` 的通用
  "unknown key" 中止。没有新增 `FileSystemError` 变体：那要同步改 `bindings.ts`、
  `navigation.ts` 的合法 kind 白名单和两套 i18n，而这条路径的错误文本本来就是后端英文句子
  （和"服务器拒绝了账号或密码"同一种）。

## 5. 未验证 / 边界

- **没有对真实服务器跑过一次完整连接**：只做了 handler 与策略的单元测试（含拒绝路径与
  "记录不下来就拒绝"）。真机验证需要一台 sshd，本机没有。
- **首次连接被中间人截获时仍然会记录攻击者的密钥** —— 这是 TOFU 的固有性质，不是实现缺陷。
  防线是日志里的指纹（可带外比对）与不匹配时的拒绝。
- **服务器换钥会连不上**，直到用户执行 `ssh-keygen -R`：与 `ssh` 的 `StrictHostKeyChecking=yes`
  行为一致，是安全默认值，不是回归。
- **`known_hosts` 读不了就拒绝**：只读的 home、别的账号拥有的文件都会让 SFTP 不可用。
  错误信息说明了该改什么。

## 6. 重新评估条件

- 连接对话框愿意承载"核对指纹"这一步 → 升级成 `ask`（见 §2）。
- 支持 `@cert-authority` / `@revoked` → 证书路径才谈得上信任签发者。
- 如果将来有非 `russh` 的 SSH 后端，策略应当复用 `known_hosts::verify_host_key`，
  不要各自实现。
