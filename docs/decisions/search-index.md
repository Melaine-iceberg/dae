# 平台搜索索引：作为候选源，而不是替代品

> **状态：已实施（2026-11-07）。** 范围：`crates/dae-core/src/fs/search_index.rs`、
> `crates/dae-core/src/fs/search.rs`（合并）、`crates/dae-core/Cargo.toml`（Windows
> feature 与 `Win32_System_Variant`）。
> 相关：`docs/files-parity.md` §5.5 阶段 2、§12.1 的 R-1。

## 0. 结论

搜索框的含义是「**名字包含这些字**」。平台索引的含义不一样：Windows Search 连
正文一起匹配，tracker3/baloo 也是全文。因此索引在这里只做一个**额外的候选源**：
它给出路径，`fs::search` 用与遍历完全相同的规则再筛一遍（名字包含查询、位于某个
根之下、通过类型/时间/大小过滤），最后与遍历结果按路径去重。

这样做的代价是索引不能提高召回（遍历本来就覆盖根内全部条目），收益是**跨多个
卷时索引能在几毫秒内给出候选**，而遍历要读完每个卷；并且语义、取消与标签过滤
仍然只有一套实现。

失败的索引不是错误：查询失败只记 debug 日志，结果就是遍历的那一份。索引是加速
器，不是依赖。

## 1. Windows：ISearchQueryHelper + CollatorDSO

`ISearchQueryHelper` 只负责把用户输入变成 SQL；执行由
`Provider=Search.CollatorDSO;EXTENDED PROPERTIES="Application=Windows"` 的 OLE DB
提供方完成。实现走 `IDataInitialize::GetDataSource`，避免手写 `DBPROP` 属性集：

1. `CoCreateInstance(MSDAINITIALIZE)` → `IDataInitialize`
2. `GetDataSource(…, IID_IDBInitialize)` → `Initialize()`
3. `CreateSession(IID_IDBCreateCommand)` → `CreateCommand(IID_ICommandText)`
4. `ISearchQueryHelper`: `SetQueryMaxResults`、`SetQuerySelectColumns("System.ItemUrl")`、
   `GenerateSQLFromUserQuery(text)`（`PWSTR` 用 `CoTaskMemFree` 归还）
5. `Execute(IID_IRowset)`，`IRowset::GetNextRows` 逐行读

几个必须记住的细节，全部来自实机调试：

- **列类型是 VARIANT（0xc），不是 WSTR。** 按 `DBTYPE_WSTR` 绑定会被
  `CreateAccessor` 拒绝（`DBBINDSTATUS_BADBINDINFO`）。绑定
  `DBTYPE_VARIANT`，读 `VT_BSTR` 再 `VariantClear`。
- **`GetNextRows` 返回的是提供方分配的 HROW 数组指针**，不是调用方数组的填充。
  `*prghRows` 才是数组地址，`(*prghRows)[0]` 是第一个句柄；释放行后数组用
  `CoTaskMemFree` 归还。
- **用 `System.ItemUrl`，不用 `System.ItemPathDisplay`。** 后者会把 `Users` 等
  已知目录本地化（中文系统上返回 `C:\用户\...`），那不是磁盘上的路径。
- **COM 在本线程初始化**（MTA）；`RPC_E_CHANGED_MODE` 视为「调用方已经初始化过」，
  守卫不负责反初始化别人的 apartment。

探测：`CoCreateInstance(CSearchManager)` + `GetCatalog("SystemIndex")` +
`GetQueryHelper()` 成功即视为可用，结果按进程缓存。MSIX 形态下索引可见性未验证
（计划本来就要求先验证再承诺），不可用时搜索自动退回遍历。

## 2. Linux：tracker3 / baloo 的 CLI

不引入自己的爬虫，也不常驻守护：`tracker3 search --limit N <text>` 优先，
`baloosearch -l N <text>` 兜底。两条命令的输出都按「一行一个路径」解析：

- `file://` URI 先剥前缀再 percent-decode（tracker 会转义空格与 CJK）；
- 绝对路径直接接受；
- 横幅、计数、相对路径一律忽略。

命令不存在或非零退出都只算空贡献。CLI 没有设超时：它连的是本机索引服务，挂起时
遍历结果已经返回，用户在结果列表里不会看到空洞。

## 3. 合并

`fs::search` 在遍历之后调用索引，然后：

- 只保留位于请求根之下的路径（`path_is_under`，Windows/macOS 折叠大小写——索引
  返回的 `System.ItemUrl` 会把用户名小写）；
- 名字必须包含查询（与遍历同一规则）；
- 每个路径 `stat` 一次填列，文件已消失就丢弃；
- 与遍历结果按**折叠路径**去重，最多再补 200 条；
- 索引候选同样要通过 `SearchFilters`，也仍然要被 `tag:` 过滤裁一遍。

索引查询在同一个阻塞线程上同步做，不额外开线程：遍历已经占住搜索的墙钟时间，
索引通常只占其中很小一段。

## 4. 验证

Windows（本机，Windows Search 服务运行中）：

- `search_index::try_query("Cargo", 10)` 返回 5 条真实路径（`C:/users/...`），
  `readme`、`dae` 同；
- 合并后的 `search::search` 对 `C:\Users\mt342` 搜 `path-user-backup` 返回 2 条，
  对 `C:\dae-listing-bench` 搜 `IMG_5869` 返回 12 条，相对路径列正确；
- `cargo test -p dae-core --lib`：374 passed（新增 URI 解析、Windows 路径换算、
  大小写折叠等用例）。

Linux 运行时未验证（无设备）：tracker3/baloo 的命令行与输出格式按各自文档实现，
解析器有单测；真机按 `docs/qt-platform-parity.md` §4 补测。

## 5. 仍然存在的边界

- **索引滞后**：刚创建/改名的文件可能还查不到；遍历结果仍会给出它，所以合并后
  不会缺。
- **正文命中被丢弃**：索引按内容匹配到的条目，名字不含查询时不会显示——这是
  刻意保持与遍历一致的语义。
- **没有索引取消令牌**：单次查询受 `TOP N` 限制，通常几十毫秒；遍历的取消不影响
  已发出的索引查询。
- **MSIX 形态未验证**：打包后的进程对 `SystemIndex` 的可见性可能不同，探测失败
  时入口不存在、搜索退回遍历。
