# 路径类型化：EntryUrl，一次解析一次序列化

> 日期：2026-11-18 · 状态：**已实施**
> 对象：`crates/dae-core/src/fs/url.rs`（新增）、`vfs.rs`、`smb.rs`、`sftp.rs`、
> `ftp.rs`、`webdav.rs`、`cloud.rs`、`tags.rs`

学的是 KIO 的 `QUrl` 与 Rust 生态的 `url::Url`：路径是带结构的值，不是
约定俗成的字符串。此前 `scheme://` 前缀在 `vfs::split_scheme` 解析一遍，
`[user@]host[:port]` authority 在 `smb.rs` 的 `split_authority`/`parse_authority`
再解析一遍（SFTP、FTP、WebDAV 借用），`cloud://provider:email` 在 `cloud.rs`
第三遍；每个后端又各自用 `format!` 把路径拼回去。同一字符串在六处被以六种
方式解释，边界 bug 就住在这里面——最实际的一个：`sftp://user@nas:22/home`
的 `user@` 从没被剥掉，整个用户名被当成了主机名去连。

---

## 0. 结论

| 问题 | 决定 |
|---|---|
| 类型 | `fs::url::EntryUrl`：`Location::{Local, Server, Cloud}`，解析在 `EntryUrl::parse` 一处，规范文本在 `Display` 一处 |
| Scheme | 按拼写保留：`Ftp`/`Ftps`、`WebDav`/`WebDavS` 不再折叠——FTP 的 TLS 模式由 scheme 决定，不再重新嗅探字符串 |
| 用户名 | authority 里的 `user@` 是**输入提示**，喂给连接存储；规范文本永不携带它——面包屑、最近访问、撤销日志都不该学到凭证 |
| 本地路径 | 原样保留（含 `file://`）：Windows 接受两种分隔符，用户敲的字就是最好的身份标识 |
| 远程 segments | 规范化：`\`→`/`、空段/`.`/`..` 折叠——引擎靠按下标切 segments 造父子路径，漂移的规范形式就是错路径 |
| 云端 segments | 不通明不规范化之外的任何处理：它们是 entry id，`..` 可以是合法 id 文本 |

## 1. 后端如何用

- 各后端的私有路径结构（`SmbPath`/`SftpPath`/`FtpPath`/`WebDavPath`/`CloudPath`）
  与 `parse_*_path` 全部删除；`parse_dir` 变为 `EntryUrl::parse` + scheme 守卫，
  报错文案不变。
- SMB 的 share = `segments().first()`；子路径 = `segments()[1..]`。云端
  account_id = `with_segments(vec![]).to_string()`。
- `open_backend` 系列统一收 `&EntryUrl`；会话缓存按 (host, port) 键——SFTP
  现在拿到的是干净的主机名，URL 里的用户名按 FTP 的既有惯例作为连接用户名的
  首选，落空再走连接存储与本地账户名。
- `vfs::resolve` 解析**一次**再按 scheme 分发；`scheme_of`/`is_local_path`
  保留为决策词汇（archive、undo、service、壳层十几处调用），实现退化为
  `EntryUrl::parse` 的一行。`split_scheme` 删除。

## 2. 行为变化（都是修正）

- `sftp://user@nas:22/home`：host 从 `user@nas` 变为 `nas`，用户名进连接
  流程而不是 DNS 查询。
- `ftps://` 与 `webdavs://` 在 `scheme_of` 里不再与各自的无 TLS 拼写等值；
  面包屑与规范路径印出真实拼写。
- `tags.rs` 的 volume key（分片键）改为 `scheme://host[:port]`（以前丢端口、
  且 sftp 会把 `user@` 带进键）。存量远程卷的标签分片会换个名字——远程路径上
  存过标签的场景此前基本不可用（键里带着用户名时根本对不上），不再兼容旧键。

## 3. 明确没有做的

- **把 `Vec<String>` 的函数签名全换成 `EntryUrl`。** QML 边界是 QString，
  cxx-qt 桥不会带 Rust 枚举；本地叶操作是 `std::fs`/`PathBuf` 的世界。本次
  收编的是**歧义**（scheme、authority、规范化、序列化），`service`/`jobs`/
  `undo` 各层在边界处解析一次即可。若后续发现某层大量自行切片，再逐层推进。
- **引入 `url` crate。** 它要求 percent-encoding 与 RFC 3986 语义，而 dae 的
  路径从不 percent-encode（WebDAV 只在线上编码），Windows 路径也不是 RFC
  形状。自写的 350 行贴着这个仓库的真实合同。
- **percent-encoding 的统一。** 线上编解码仍归 WebDAV 后端私有。
