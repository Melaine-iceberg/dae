pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The navigation column: system places, favourites, volumes, WSL distros,
// network storage, cloud accounts and recents in a single sectioned list.
// `SidebarModel` does the resolving; this file only decides what a row looks
// like and where a click goes.
Item {
    id: root

    property bool dark: true
    property string material: "none"
    property string activePath: ""
    property color accent: Theme.interactive
    property bool showHeader: true
    property bool trashActive: false
    property bool overviewActive: false
    property bool spaceActive: false
    property string activeSpaceId: ""

    /// The volume tree expansion lives on the sidebar, not in the delegate:
    /// ListView recycles delegates, and a collapsed-then-restored volume must
    /// come back expanded. `volumeRevision` exists only to make the delegate's
    /// binding re-evaluate when the map is replaced.
    property var expandedVolumes: ({})
    property int volumeRevision: 0

    signal navigateRequested(string path)
    signal trashRequested()
    signal overviewRequested()
    signal spaceRequested(string id)
    signal createSpaceRequested()
    signal addRequested(string kind)
    signal contextRequested(string kind, string path, real x, real y)
    signal remoteOpenRequested(string path)

    readonly property bool surfaceActive: trashActive || overviewActive || spaceActive

    readonly property color panelColor: material === "none"
        ? Theme.chrome
        : (dark ? Qt.rgba(0.05, 0.06, 0.07, 0.30) : Qt.rgba(0.95, 0.96, 0.97, 0.34))
    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkFaint
    readonly property color hoverColor: Theme.fill
    readonly property color activeColor: Theme.fill

    SidebarModel {
        id: model
    }

    function pathEquals(left, right) {
        if (left === "" || right === "")
            return false
        let a = left.replace(/[\\/]+$/, "")
        let b = right.replace(/[\\/]+$/, "")
        if (Qt.platform.os === "windows") {
            a = a.toLowerCase()
            b = b.toLowerCase()
        }
        return a === b
    }

    function volumeExpanded(path) {
        return volumeRevision >= 0 && expandedVolumes[path] === true
    }

    function toggleVolume(path) {
        if (path === "")
            return
        const next = ({})
        for (const key in expandedVolumes)
            next[key] = expandedVolumes[key]
        if (next[path])
            delete next[path]
        else
            next[path] = true
        expandedVolumes = next
        volumeRevision += 1
    }

    function sectionLabel(value) {
        switch (value) {
        case "Places": return qsTr("位置")
        case "Favorites": return qsTr("收藏")
        case "Spaces": return qsTr("空间")
        case "Disks": return qsTr("磁盘")
        case "Network Storage": return qsTr("网络存储")
        case "Cloud Storage": return qsTr("云存储")
        case "Recent": return qsTr("最近")
        default: return value
        }
    }

    /// `name` comes from `SidebarModel`; translating only the built-in places
    /// and action rows keeps a favourite or recent entry named "文档" intact.
    function rowLabel(name) {
        switch (name) {
        case "Home": return qsTr("主目录")
        case "Desktop": return qsTr("桌面")
        case "Documents": return qsTr("文档")
        case "Downloads": return qsTr("下载")
        case "Pictures": return qsTr("图片")
        case "Music": return qsTr("音乐")
        case "Videos": return qsTr("视频")
        case "New space…": return qsTr("新建空间…")
        case "Connect network storage…": return qsTr("连接网络存储…")
        case "Add cloud account…": return qsTr("添加网盘账户…")
        default: return name
        }
    }

    function detailLabel(value) {
        if (value === "Empty")
            return qsTr("空")
        const count = /^(\d+) items?$/.exec(value)
        if (count)
            return qsTr("%1 项").arg(count[1])
        return value
    }

    function glyph(icon) {
        switch (icon) {
        case "home": return qsTr("主")
        case "desktop": return qsTr("桌")
        case "documents": return qsTr("文")
        case "downloads": return qsTr("下")
        case "pictures": return qsTr("图")
        case "music": return qsTr("音")
        case "videos": return qsTr("视")
        case "star": return "★"
        case "disk": return qsTr("盘")
        case "wsl": return "W"
        case "recent": return qsTr("近")
        case "server": return qsTr("服")
        case "cloud": return qsTr("云")
        case "space": return "◇"
        case "add": return "+"
        default: return "•"
        }
    }

    /// Which ids carry a drawn mark. Kept next to `glyph` so the two tables
    /// cannot disagree about what a row shows.
    function hasDrawnGlyph(icon) {
        switch (icon) {
        case "home": case "desktop": case "documents": case "downloads":
        case "pictures": case "music": case "videos":
            return true
        default:
            return false
        }
    }

    function tint(icon) {
        switch (icon) {
        case "disk": return dark ? "#8a93a6" : "#6b7482"
        case "wsl": return dark ? "#5b9bd5" : "#2f6fbf"
        case "star": return dark ? "#d8b45a" : "#b8892a"
        case "recent": return dark ? "#6fa0d8" : "#3f78bd"
        case "space": return root.accent
        case "add": return root.mutedColor
        default: return root.accent
        }
    }

    Component.onCompleted: {
        Spaces.refresh()
        model.refresh()
    }

    Connections {
        target: Spaces

        function onChanged() {
            model.refresh()
        }
    }

    Connections {
        target: Workspace

        function onFavoritesChanged() {
            model.refresh()
        }

        function onVolumesChanged() {
            model.refresh()
        }

        function onConnectionsChanged() {
            model.refresh()
        }
    }

    Rectangle {
        anchors.fill: parent
        color: root.panelColor

        Rectangle {
            anchors.right: parent.right
            width: 1
            height: parent.height
            color: Theme.fill
        }
    }

    Text {
        id: heading
        visible: root.showHeader
        x: 14
        y: 10
        text: qsTr("导航")
        color: root.mutedColor
        font.pixelSize: 11
        font.letterSpacing: 1
    }

    /// The landing page is a fixed destination too; it sits above the bin so
    /// the two shell surfaces stay adjacent however long the model gets.
    Rectangle {
        id: overviewRow

        x: 6
        y: root.showHeader ? heading.bottom + 6 : 4
        width: parent.width - 12
        height: 28
        radius: 6
        color: root.overviewActive
            ? root.activeColor
            : (overviewMouse.containsMouse ? root.hoverColor : "transparent")

        Accessible.role: Accessible.Button
        Accessible.name: qsTr("概览")
        Accessible.onPressAction: root.overviewRequested()

        Rectangle {
            x: 6
            anchors.verticalCenter: parent.verticalCenter
            width: 16
            height: 16
            radius: 4
            color: Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.18)

            Text {
                anchors.centerIn: parent
                text: "◫"
                color: root.accent
                font.pixelSize: 11
            }
        }

        Text {
            anchors.left: parent.left
            anchors.leftMargin: 30
            anchors.right: parent.right
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("概览")
            color: root.overviewActive ? root.textColor
                : Theme.inkFaint
            font.pixelSize: 12
            elide: Text.ElideRight
        }

        MouseArea {
            id: overviewMouse
            anchors.fill: parent
            hoverEnabled: true
            onClicked: root.overviewRequested()
        }
    }

    /// The bin is a fixed destination above the sectioned list, not a path the
    /// model resolves.
    Rectangle {
        id: trashRow
        x: 6
        y: overviewRow.bottom + 1
        width: parent.width - 12
        height: 28
        radius: 6
        color: root.trashActive
            ? root.activeColor
            : (trashMouse.containsMouse ? root.hoverColor : "transparent")

        Accessible.role: Accessible.Button
        Accessible.name: qsTr("回收站")
        Accessible.onPressAction: root.trashRequested()

        Rectangle {
            x: 6
            anchors.verticalCenter: parent.verticalCenter
            width: 16
            height: 16
            radius: 4
            color: Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.18)

            Text {
                anchors.centerIn: parent
                text: qsTr("回")
                color: root.accent
                font.pixelSize: 11
            }
        }

        Text {
            anchors.left: parent.left
            anchors.leftMargin: 30
            anchors.right: parent.right
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("回收站")
            color: root.trashActive ? root.textColor
                : Theme.inkFaint
            font.pixelSize: 12
            elide: Text.ElideRight
        }

        MouseArea {
            id: trashMouse
            anchors.fill: parent
            hoverEnabled: true
            onClicked: root.trashRequested()
        }
    }

    ListView {
        id: list
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: trashRow.bottom
        anchors.bottom: parent.bottom
        anchors.topMargin: 4
        model: model
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        spacing: 1
        section.property: "section"
        section.criteria: ViewSection.FullString
        section.delegate: Item {
            required property string section

            width: ListView.view.width
            height: 24

            Text {
                x: 14
                anchors.verticalCenter: parent.verticalCenter
                text: root.sectionLabel(section)
                color: root.mutedColor
                font.pixelSize: 11
                font.letterSpacing: 0.6
            }
        }

        delegate: Item {
            id: row

            required property string name
            required property string path
            required property string kind
            required property string detail
            required property string icon
            required property bool isDir

            width: ListView.view.width
            height: 28 + (treeLoader.active ? treeLoader.height : 0)

            readonly property bool isVolume: row.kind === "volume"
            readonly property bool treeOpen: row.isVolume && root.volumeExpanded(row.path)
            readonly property bool current: !root.surfaceActive
                && root.pathEquals(row.path, root.activePath)
            readonly property bool spaceCurrent: root.spaceActive
                && row.kind === "space" && row.path === root.activeSpaceId
            readonly property bool isAction: row.kind === "addConnection"
                || row.kind === "addCloud" || row.kind === "addSpace"
            readonly property bool builtInName: row.isAction
                || row.icon === "home" || row.icon === "desktop"
                || row.icon === "documents" || row.icon === "downloads"
                || row.icon === "pictures" || row.icon === "music"
                || row.icon === "videos"

            /// The visible row is only the first 27px; an expanded volume's
            /// tree hangs below it. Keeping that distinction explicit means the
            /// icon and label still centre on the row, not on the whole subtree.
            Item {
                id: rowBody

                width: parent.width
                height: 28

                Rectangle {
                    x: 6
                    width: rowBody.width - 12
                    height: rowBody.height
                    radius: 6
                    color: (row.current || row.spaceCurrent)
                        ? root.activeColor
                        : (mouse.containsMouse ? root.hoverColor : "transparent")
                }

                Rectangle {
                    id: iconTile

                    x: row.isVolume ? 24 : 12
                    anchors.verticalCenter: parent.verticalCenter
                    width: 16
                    height: 16
                    radius: 4
                    color: Qt.rgba(root.tint(row.icon).r, root.tint(row.icon).g,
                                   root.tint(row.icon).b, 0.18)

                    // A drawn mark where there is one; a monogram otherwise.
                    // The monogram is what the built-in rows used to show, and
                    // showing the label's own first character beside the label
                    // is what made those rows look like a layout collision.
                    Glyph {
                        anchors.centerIn: parent
                        visible: root.hasDrawnGlyph(row.icon)
                        kind: row.icon
                        tint: root.tint(row.icon)
                    }

                    Text {
                        anchors.centerIn: parent
                        visible: !root.hasDrawnGlyph(row.icon)
                        text: root.glyph(row.icon)
                        color: root.tint(row.icon)
                        font.pixelSize: 11
                    }
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: row.isVolume ? 46 : 36
                    anchors.right: detailText.left
                    anchors.rightMargin: 6
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.builtInName ? root.rowLabel(row.name) : row.name
                    color: (row.current || row.spaceCurrent) ? root.textColor
                        : (row.isAction ? root.mutedColor
                            : Theme.inkFaint)
                    font.pixelSize: 12
                    elide: Text.ElideRight
                    textFormat: Text.PlainText
                }

                Text {
                    id: detailText
                    anchors.right: parent.right
                    anchors.rightMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.detailLabel(row.detail)
                    color: root.mutedColor
                    font.pixelSize: 11
                    elide: Text.ElideRight
                    width: Math.min(74, implicitWidth)
                    horizontalAlignment: Text.AlignRight
                }

                MouseArea {
                    id: mouse
                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onClicked: function (event) {
                        if (event.button === Qt.RightButton) {
                            if (row.isAction)
                                return
                            const point = row.mapToItem(null, event.x, event.y)
                            root.contextRequested(row.kind, row.path, point.x, point.y)
                            return
                        }
                        if (row.kind === "addConnection") {
                            root.addRequested("network")
                            return
                        }
                        if (row.kind === "addCloud") {
                            root.addRequested("cloud")
                            return
                        }
                        if (row.kind === "addSpace") {
                            root.createSpaceRequested()
                            return
                        }
                        if (row.kind === "space") {
                            root.spaceRequested(row.path)
                            return
                        }
                        if (row.kind === "recent" && !row.isDir) {
                            if (/^(smb|sftp|ftp|webdav|webdavs|cloud):\/\//i.test(row.path))
                                root.remoteOpenRequested(row.path)
                            else
                                Workspace.openPath(row.path)
                        } else {
                            root.navigateRequested(row.path)
                        }
                    }
                }

                // A volume's chevron is a separate hit target from the row
                // itself: the label still navigates to the drive, only the
                // glyph toggles the subtree.
                Item {
                    visible: row.isVolume
                    x: 0
                    width: 22
                    height: 28

                    Text {
                        anchors.centerIn: parent
                        text: row.treeOpen ? "▾" : "▸"
                        color: row.treeOpen ? root.textColor : root.mutedColor
                        font.pixelSize: 11
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: root.toggleVolume(row.path)
                    }
                }
            }

            // One lazily loaded tree per expanded volume. Collapsing deactivates
            // the loader, which releases that volume's children again; the
            // expansion flags above make reopening it cheap and predictable.
            Loader {
                id: treeLoader

                anchors.left: parent.left
                anchors.right: parent.right
                y: 27
                height: active && status === Loader.Ready && item !== null
                    ? item.implicitHeight : 0
                active: row.treeOpen
                visible: active

                sourceComponent: Component {
                    SidebarTree {
                        width: treeLoader.width
                        dark: root.dark
                        material: root.material
                        accent: root.accent
                        activePath: root.activePath
                        showHidden: ExplorerPrefs.showHiddenFiles

                        onNavigateRequested: function (path) {
                            root.navigateRequested(path)
                        }
                        onContextRequested: function (path, x, y) {
                            root.contextRequested("tree", path, x, y)
                        }
                    }
                }

                onLoaded: {
                    if (item !== null)
                        item.rootPath = row.path
                }
            }
        }
    }
}
