# 标签索引的完整性：身份、改名、重建与定义导入

> **状态：已实施（2026-11-07）。** 范围：`crates/dae-core/src/fs/tags.rs`、
> `crates/dae-qt/src/tags.rs`、`qml/SettingsPanel.qml`。
> 相关：`docs/files-parity.md` §12.1 的 R-3、R-4，§5.1 的存储设计。

## 0. 结论

标签关联以路径为键，索引是 `tag:` 搜索、侧栏计数和按标签排序的唯一真相。
路径会因为四件事变得不可信：dae 内的重命名、dae 外的重命名、文件被删除、
同一路径被新文件复用。之前的实现只处理了第一种的一半（没有任何迁移），
并且把第二种写成了「已知边界」。现在四种都由索引自己解决，不需要文件系统
watcher 报告 rename 对：

| 情况 | 处理 |
| --- | --- |
| dae 内重命名（单文件、批量、目录） | `tags::note_rename` 迁移条目，目录按前缀迁移 |
| dae 外重命名 | 读取新路径时按文件身份找回条目并迁到新键 |
| 文件删除 | 重建索引时丢弃；读取时身份不匹配也不会命中 |
| 路径被新文件复用 | 身份不匹配 → 视为未命中，新文件不继承旧标签 |
| 索引分片损坏 | 重建时隔离为 `*.corrupt-<millis>`，不删除 |

## 1. 文件身份

`IndexEntry.file_id` 在每次写入时记录：

- **Windows**：`GetFileInformationByHandle` 的卷序列号 + 文件索引
  （`nFileIndexHigh`/`nFileIndexLow`）。打开时带 `FILE_FLAG_BACKUP_SEMANTICS`，
  所以目录也有身份——重命名一个目录要迁移它下面所有子项。
- **Unix**：`dev` + `ino`（原有行为）。

身份只在本地路径上读取；`sftp://`、`smb://`、云盘与压缩包路径没有可用的
`metadata`，返回 `None`，相关条目退化为纯路径关联。

## 2. 读取路径的两个检查

`tags_for` 是可见行的批量读取，仍然不碰枚举热路径。每次读：

1. **命中**：如果条目带身份、文件当前身份不同，视为未命中。名字被删后新建的
   文件复用，旧标签不能凭空继承这个新文件。
2. **未命中**：先读文件自身的流（xattr/ADS），流里有 id 就以流为准——这是
   「标签随文件走」的载体；流为空时，用当前身份在同一个分片里找 `file_id`
   相同的条目，找到就把它迁到新键并写回。这就是 dae 外重命名的恢复路径。

代价：只有未命中的行多一次 `open` + 身份读取（Windows 一次
`GetFileInformationByHandle`，Unix 一次 `stat`）。已标记的行只在条目带身份时
比较一次。目录枚举仍然零额外系统调用。

## 3. dae 内重命名

`tags::note_rename(from, to)` 在以下位置调用，全部在后端报告成功之后：

- `fs::service::rename_entry`（单文件重命名）；
- `fs::undo::apply_rename_pairs` 的每个落地重命名，以及它内部的 `stage_sibling`
  暂存改名（批量重命名 / 交换 / 大小写翻转会先暂存）；
- `fs::undo` 的 `undo_rename` / `redo_rename`。

`migrate` 先迁移精确键，再按 `<from_key><分隔符>` 前缀迁移目录下的所有条目。
存储的路径拼写保留用户的大小写；键按平台折叠。跨卷的移动会写两个分片，
目标分片已有的条目与新条目合并 id。

这是**同步**调用：一次索引读 + 一次写，发生在重命名之后，不在列表热路径上。
路径没有索引条目时只读一次分片就返回。

## 4. 重建索引有界

`TagStore::repair_index`（界面入口：设置 → 标签 → 重建索引）**只遍历索引，
不扫描磁盘**，所以它永远不会比索引本身更贵：

1. 逐个解析分片；解析失败的改名为 `*.corrupt-<millis>` 保留现场，计入
   `corruptShards`。
2. 每个条目：路径不存在 → 丢弃（`droppedMissing`）；存在 → 重算身份，
   文件流里有 id 且与索引不同 → 采用文件流（`adoptedFromFile`）。
3. 按当前路径重算分片归属（卷 / 远程主机）；同一身份出现两次 → 保留
   `updatedAt` 较新的（`droppedDuplicate`）。
4. 写回非空分片，删除不再被引用的旧分片文件。

不做的事：不重新发现「文件流里有标签但索引里没有」的文件。那需要遍历磁盘，
与列表预算冲突；这种条目在第一次读取该文件时会从流里恢复。

## 5. 定义的导入与导出

格式就是配置目录里的 `tags.toml`（`[[tags]]` 数组），没有第二种格式：

- `export_document` / `export_to`：按当前定义序列化，原子写。
- `import_document` / `import_from`：合并而不是替换。
  - 已知 id → 覆盖名称与颜色（`updated`）。
  - 未知 id → 追加（`added`）。
  - 空 id/名称、名称超过 32 字符、名称已被**另一个 id** 占用 → 跳过，并在
    `skipped` 里给出原因，不做「自动改名」这种猜测。
  - 颜色按现有规则归一化；无效颜色落到调色板。
- `TagDefinition.color` 加了 `#[serde(default)]`：手改的 `tags.toml` 少一个颜色
  字段不会让整份定义读成空。

界面上是设置 → 标签底部的「导出… / 导入… / 重建索引」，状态行说明结果
（新增/更新/跳过数量，重建的四个计数）。Qt 桥三个入口统一返回
`{"ok":true,…}` / `{"ok":false,"error":"…"}` 的 JSON，QML 只解析一种形状。

## 6. 仍然存在的边界

- **远程路径的外部重命名**：没有文件身份可用，条目留在旧键；下次重建索引时
  按「路径不存在」丢弃。远程重命名如果需要保住标签，要后端事件支持（SMB
  notify、SFTP 无等价物），不在本轮。
- **跨卷移动中索引独有的标签**：复制+删除在目标卷产生新身份，源条目路径消失；
  文件流能带走标签的卷不受影响，索引独有（exFAT、网络）的关联会丢。
- **未命中行多一次身份读取**：可见行级别的成本，不进枚举；12 万条列表的
  p95 基准仍在待办（`docs/files-parity.md` §12.3）。
- **重建不修复文件之间的标签**：它只对账「索引 vs 文件」，不推断用户意图。

## 7. 验证

- `cargo test -p dae-core --lib fs::tags`：17 passed。新增用例：
  - `export_is_a_tags_document_and_import_merges_it`（覆盖、追加、两类跳过）；
  - `export_and_import_round_trip_through_a_file`；
  - `a_malformed_import_is_refused_without_touching_the_definitions`；
  - `an_index_only_tag_follows_a_rename_made_outside_the_app`（删掉流之后
    只靠身份找回；Windows 上实际走 `GetFileInformationByHandle`）；
  - `a_recreated_path_does_not_inherit_the_old_tags`；
  - `repair_drops_missing_files_and_adopts_the_stream`；
  - `repair_quarantines_a_corrupt_shard_instead_of_deleting_it`；
  - `migrating_a_directory_moves_the_entries_beneath_it`。
- `DAE_QML_LINT=1 DAE_QML_TYPEC=1 cargo build -p dae-app` 通过（`qml/SettingsPanel.qml`
  的三个入口经 qmltc 的类型检查）。
- Windows 运行时冒烟（隔离 `APPDATA`）：设置 → 标签页，预置一个指向已删除文件的
  索引分片，按「重建索引」后状态行为「0 条记录，移除 1 条失效、0 条重复、0 个损坏
  分片」，分片文件被删除。导入/导出走系统文件对话框，其桥接测试覆盖了回复形状与
  合并语义，对话框本身未自动点选。

## 8. 何时重新考虑

- 远程后端能给出 rename 事件时，把 `note_rename` 接到事件而不是只在 dae 内；
- 索引条目量级到十万后，再评估 `rusqlite`（替换 JSON 分片，见 §5.1 的原始
  决策条件）；
- 若 Windows 身份读取在可见行批量里被测出明显成本，改成每分片缓存
  `identity → key` 的反向表，按目录批量预取。
