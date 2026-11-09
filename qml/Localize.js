.pragma library

// Maps the fixed English strings the Rust models emit to the current UI
// language. The backend stays language-neutral so the same status feeds an
// English or a Chinese shell; only this table decides what the user reads.
//
// Anything not recognised is returned unchanged: a file name, a path or an
// unexpected error is more useful in its own words than hidden behind a
// generic replacement.

/// Translate a status line or a user-facing message.
function message(value) {
    if (!value)
        return value
    // A few model strings are several lines already — a conflict card's
    // `kind · size` / `Modified …` pair, or a media block. Each line is its
    // own sentence, so it is translated on its own.
    if (value.indexOf("\n") >= 0)
        return value.split("\n").map(line).join("\n")
    return line(value)
}

/// One line of a message, or the whole of a single-line one.
function line(value) {
    if (!value)
        return value

    let match = /^Items: (\d+) · (.+)$/.exec(value)
    if (match)
        return qsTr("%1 项 · %2").arg(match[1]).arg(match[2])
    match = /^(?:Items: )?(\d+)$/.exec(value)
    if (match)
        return qsTr("%1 项").arg(match[1])
    match = /^Subfolders: (\d+)$/.exec(value)
    if (match)
        return qsTr("%1 个子文件夹").arg(match[1])
    match = /^(\d+) items?$/.exec(value)
    if (match)
        return qsTr("%1 项").arg(match[1])
    match = /^(\d+) hidden$/.exec(value)
    if (match)
        return qsTr("已隐藏 %1 项").arg(match[1])
    match = /^Results: (\d+)( · Truncated)?$/.exec(value)
    if (match) {
        return qsTr("%1 项结果").arg(match[1])
            + (match[2] ? " · " + qsTr("已截断") : "")
    }
    match = /^(\d+) changes?$/.exec(value)
    if (match)
        return qsTr("%1 项变更").arg(match[1])
    match = /^(Copy|Move) (\d+) items? → (.+)$/.exec(value)
    if (match) {
        return qsTr("%1 %2 项 → %3")
            .arg(match[1] === "Copy" ? qsTr("复制") : qsTr("移动"))
            .arg(match[2]).arg(match[3])
    }
    match = /^(Folder|File|Symlink|Other) · (.+)$/.exec(value)
    if (match)
        return qsTr("%1 · %2").arg(line(match[1])).arg(match[2])
    match = /^Modified (.+)$/.exec(value)
    if (match)
        return qsTr("修改于 %1").arg(match[1])
    match = /^(Duration|Dimensions|Bitrate|Title|Artist|Album|Genre|Camera|Lens|Taken|Exposure|Aperture|ISO|Focal length|GPS): (.+)$/.exec(value)
    if (match) {
        const labels = {
            "Duration": qsTr("时长"),
            "Dimensions": qsTr("分辨率"),
            "Bitrate": qsTr("码率"),
            "Title": qsTr("标题"),
            "Artist": qsTr("艺术家"),
            "Album": qsTr("专辑"),
            "Genre": qsTr("流派"),
            "Camera": qsTr("相机"),
            "Lens": qsTr("镜头"),
            "Taken": qsTr("拍摄时间"),
            "Exposure": qsTr("曝光"),
            "Aperture": qsTr("光圈"),
            "ISO": qsTr("ISO"),
            "Focal length": qsTr("焦距"),
            "GPS": qsTr("位置")
        }
        return (labels[match[1]] || match[1]) + qsTr("：") + match[2]
    }
    match = /^Switched to branch (.+)$/.exec(value)
    if (match)
        return qsTr("已切换到分支 %1").arg(match[1])
    match = /^Created branch (.+)$/.exec(value)
    if (match)
        return qsTr("已创建分支 %1").arg(match[1])
    match = /^Could not create log directory: (.+)$/.exec(value)
    if (match)
        return qsTr("无法创建日志目录：%1").arg(match[1])
    match = /^Unsupported connection protocol: (.+)$/.exec(value)
    if (match)
        return qsTr("不支持的网络协议：%1").arg(match[1])

    switch (value) {
    case "No matches": return qsTr("没有匹配项")
    case "No matching content": return qsTr("没有匹配内容")
    case "Invalid search query": return qsTr("搜索表达式无效")
    case "Invalid search expression": return qsTr("搜索表达式无效")
    case "Loading…": return qsTr("读取中…")
    case "Loading folder…": return qsTr("读取文件夹…")
    case "No subfolders": return qsTr("没有子文件夹")
    case "Folder": return qsTr("文件夹")
    case "File": return qsTr("文件")
    case "Symlink": return qsTr("符号链接")
    case "Symbolic link": return qsTr("符号链接")
    case "Other": return qsTr("其他")
    case "Could not read folder": return qsTr("无法读取文件夹")
    case "No path": return qsTr("没有路径")
    case "Closed": return qsTr("已关闭")
    case "Loading Trash…": return qsTr("读取回收站…")
    case "Could not read Trash": return qsTr("无法读取回收站")
    case "Trash is empty": return qsTr("回收站是空的")
    case "Searching…": return qsTr("搜索中…")
    case "Searching file contents…": return qsTr("搜索文件内容…")
    case "Creating…": return qsTr("新建中…")
    case "Renaming…": return qsTr("重命名中…")
    case "Moving to Trash…": return qsTr("移到回收站…")
    case "Deleting…": return qsTr("删除中…")
    case "Duplicating…": return qsTr("创建副本…")
    case "Creating shortcut…": return qsTr("创建快捷方式…")
    case "Copying…": return qsTr("复制中…")
    case "Moving…": return qsTr("移动中…")
    case "Compressing…": return qsTr("压缩中…")
    case "Extracting…": return qsTr("解压中…")
    case "Undoing…": return qsTr("撤销中…")
    case "Redoing…": return qsTr("重做中…")
    case "Restoring…": return qsTr("恢复中…")
    case "Deleting permanently…": return qsTr("永久删除中…")
    case "Emptying Trash…": return qsTr("清空回收站…")
    case "Opening…": return qsTr("正在打开…")
    case "Setting as default…": return qsTr("正在设为默认打开方式…")
    case "Calculating…": return qsTr("计算中…")
    case "Created": return qsTr("已创建")
    case "Renamed": return qsTr("已重命名")
    case "Moved to Trash": return qsTr("已移到回收站")
    case "Permanently deleted": return qsTr("已永久删除")
    case "Duplicated": return qsTr("已创建副本")
    case "Shortcut created": return qsTr("已创建快捷方式")
    case "Copied": return qsTr("已复制")
    case "Moved": return qsTr("已移动")
    case "Compressed": return qsTr("已压缩")
    case "Extracted": return qsTr("已解压")
    case "Undone": return qsTr("已撤销")
    case "Redone": return qsTr("已重做")
    case "Opened": return qsTr("已打开")
    case "Restored": return qsTr("已恢复")
    case "Trash emptied": return qsTr("已清空回收站")
    case "Done": return qsTr("完成")
    case "Opened in system terminal": return qsTr("已在系统终端中打开")
    case "Added to favorites": return qsTr("已加入收藏")
    case "Removed from favorites": return qsTr("已从收藏移除")
    case "Hidden from sidebar": return qsTr("已从侧栏隐藏")
    case "Restored hidden places": return qsTr("已恢复隐藏的位置")
    case "Connection removed": return qsTr("已移除连接")
    case "Cloud account removed": return qsTr("已移除云盘账户")
    case "Connection test succeeded": return qsTr("连接测试成功")
    case "Synced with remote": return qsTr("已同步远程仓库")
    case "Pulled remote updates": return qsTr("已拉取远程更新")
    case "Pushed to remote": return qsTr("已推送到远程")
    case "Fetched remote updates": return qsTr("已获取远程更新")
    case "Shortcuts saved": return qsTr("快捷键已保存")
    case "Terminal settings saved": return qsTr("终端设置已保存")
    case "Opened log directory": return qsTr("已打开日志目录")
    case "Enter a Client ID": return qsTr("请输入 Client ID")
    case "Unknown shortcut action": return qsTr("未知的快捷键操作")
    case "Invalid shortcut format": return qsTr("快捷键格式无效")
    case "Could not set the default file manager": return qsTr("无法设置默认文件管理器")
    case "Could not read the default file manager status": return qsTr("无法读取默认文件管理器状态")
    case "Nothing to undo": return qsTr("没有可撤销的操作")
    case "Nothing to redo": return qsTr("没有可重做的操作")
    case "Search is only available in local directories": return qsTr("搜索仅适用于本地目录")
    case "Content search is only available in local directories": return qsTr("内容搜索仅适用于本地目录")
    case "Trash is only available for local items": return qsTr("回收站仅适用于本地项目")
    case "Terminal can only open local directories": return qsTr("终端只能打开本地目录")
    case "Open With is only available for local items": return qsTr("打开方式仅适用于本地项目")
    case "This item no longer exists": return qsTr("此项已不存在")
    case "Office document is too large to preview.": return qsTr("Office 文档过大，无法预览")
    case "This Office document has no readable text.": return qsTr("此 Office 文档没有可显示的正文")
    case "This Office file cannot be read. It may be encrypted or damaged.": return qsTr("无法读取此 Office 文件，它可能已加密或已损坏")
    case "Legacy Office formats cannot be previewed. Save a .docx, .xlsx or .pptx copy first.": return qsTr("旧版 Office 格式无法预览，请先另存为 .docx、.xlsx 或 .pptx")
    case "This document format has no preview yet.": return qsTr("此文档格式暂不支持预览")
    case "Open With is not supported on this system": return qsTr("当前系统不支持打开方式")
    case "The selected items are no longer in Trash": return qsTr("所选项目已不在回收站中")
    case "The item's original location could not be determined": return qsTr("无法确定项目的原始位置")
    case "This directory is not in a Git repository": return qsTr("此目录不在 Git 仓库中")
    case "Branch not found": return qsTr("找不到分支")
    case "A branch with this name already exists": return qsTr("同名分支已存在")
    case "Invalid branch name": return qsTr("分支名称无效")
    case "Cannot create a branch because the repository has no commits": return qsTr("仓库还没有提交，无法创建分支")
    case "No remote is configured for this repository": return qsTr("此仓库未配置远程")
    case "The current branch has no upstream branch": return qsTr("当前分支没有上游分支")
    case "HEAD is detached. Switch to a branch first.": return qsTr("HEAD 处于分离状态，请先切换到分支。")
    case "Could not switch branches. Commit or stash your local changes first.": return qsTr("无法切换分支，请先提交或暂存本地修改。")
    case "Could not create branch": return qsTr("无法创建分支")
    case "Could not fetch remote updates. Check the network connection and remote URL.": return qsTr("无法获取远程更新，请检查网络连接和远程地址。")
    case "Could not pull. Resolve local changes or conflicts first.": return qsTr("无法拉取，请先处理本地修改或冲突。")
    case "Could not push. Pull remote updates first.": return qsTr("无法推送，请先拉取远程更新。")
    case "Git was not found. Install Git first.": return qsTr("未找到 Git，请先安装 Git。")
    case "The server does not have the SFTP subsystem enabled": return qsTr("服务器未启用 SFTP 子系统")
    case "Could not connect to the server. Check the address, port, and network connection.": return qsTr("无法连接到服务器，请检查地址、端口和网络连接。")
    case "SFTP authentication failed. Check the username and password.": return qsTr("SFTP 认证失败，请检查用户名和密码。")
    case "The server rejected the connection. Enter your username and password, then try again.": return qsTr("服务器拒绝了连接，请输入用户名和密码后重试。")
    case "The server rejected the username or password": return qsTr("服务器拒绝了用户名或密码")
    case "SFTP requires a username. Enter one and try again.": return qsTr("SFTP 需要用户名，请填写后重试。")
    case "No cloud account is connected. Add a cloud account in the sidebar and complete authorization first.": return qsTr("未连接云盘账户，请先在侧栏添加云盘账户并完成授权。")
    case "No update is available to install": return qsTr("没有可安装的更新")
    case "No update has been released for this platform": return qsTr("当前平台尚未发布更新")
    case "Update package signature verification failed. Installation was blocked.": return qsTr("更新包签名验证失败，已阻止安装。")
    case "The update package is incomplete. The current version was kept.": return qsTr("更新包不完整，已保留当前版本。")
    case "This build does not support automatic updates": return qsTr("此版本不支持自动更新")
    case "This build has no updatable installation location": return qsTr("此版本没有可更新的安装位置")
    default: return detail(value)
    }
}

/// Translate the known prefixes and sentences produced by the error layer.
function detail(value) {
    let match = /^Operation failed: (.+)$/.exec(value)
    if (match)
        return qsTr("操作失败：%1").arg(match[1])
    match = /^Permission denied: (.+)$/.exec(value)
    if (match)
        return qsTr("权限被拒绝：%1").arg(match[1])
    match = /^Invalid input: (.+)$/.exec(value)
    if (match)
        return qsTr("无效输入：%1").arg(match[1])
    match = /^Not supported on this platform: (.+)$/.exec(value)
    if (match)
        return qsTr("当前平台不支持：%1").arg(match[1])
    match = /^The system could not open (.+)$/.exec(value)
    if (match)
        return qsTr("系统无法打开 %1").arg(match[1])
    match = /^Update check failed: (.+)$/.exec(value)
    if (match)
        return qsTr("检查更新失败：%1").arg(match[1])
    match = /^Update download failed: (.+)$/.exec(value)
    if (match)
        return qsTr("下载更新失败：%1").arg(match[1])
    match = /^Invalid update feed: (.+)$/.exec(value)
    if (match)
        return qsTr("更新源无效：%1").arg(match[1])
    match = /^Could not start the update installer: (.+)$/.exec(value)
    if (match)
        return qsTr("无法启动更新安装程序：%1").arg(match[1])
    return value
}
