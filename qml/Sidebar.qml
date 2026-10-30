pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The navigation column: the five places people reach for, then extra places
// and favourites, volumes, spaces, network storage, cloud accounts and recents.
// The trash is parked at the bottom, where a bin belongs, instead of sharing a
// header with the overview. `SidebarModel` decides the order and resolves the
// rows; this file decides what a row looks like and where a click goes.
Item {
    id: root

    property bool dark: true
    property string material: "none"
    property string activePath: ""
    property bool trashActive: false
    property int trashCount: 0
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
    signal spaceRequested(string id)
    signal createSpaceRequested()
    signal addRequested(string kind)
    signal contextRequested(string kind, string path, real x, real y)
    signal remoteOpenRequested(string path)

    /// Narrow columns drop the "used / total" pair for the percentage: at 184 px
    /// the full pair is the first thing that would collide with the name.
    readonly property bool narrow: width < 200
    readonly property bool surfaceActive: trashActive || spaceActive
    readonly property string trashLabel: Qt.platform.os === "osx"
        ? qsTr("废纸篓") : qsTr("回收站")

    readonly property color panelColor: material === "none"
        ? Theme.chrome
        : (dark ? Qt.rgba(0.05, 0.06, 0.07, 0.30) : Qt.rgba(0.95, 0.96, 0.97, 0.34))

    /// The row a file drag is hovering, by path. Empty when nothing is over the
    /// sidebar, so a row asks one question to decide whether it is lit.
    property string dropTargetPath: ""

    SidebarModel {
        id: sidebarModel
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

    /// Moves the keyboard cursor by one row. A negative index means "nothing is
    /// under the cursor yet", which is why the first ↓ starts at 0 rather than
    /// at 1 — otherwise the first press after tabbing in would skip a row.
    function moveCursor(delta) {
        if (list.count === 0)
            return
        let index = list.currentIndex
        if (index < 0)
            index = delta > 0 ? -1 : list.count
        index = Math.max(0, Math.min(list.count - 1, index + delta))
        list.currentIndex = index
        list.positionViewAtIndex(index, ListView.Contain)
    }

    /// Runs the row under the cursor through the same branch the pointer uses.
    function activateCursor() {
        const item = list.currentItem
        if (item !== null && item.activate !== undefined)
            item.activate()
    }

    /// → opens a volume's tree, ← closes it. On any other row the key does
    /// nothing: a sidebar has no depth to walk into.
    function unfoldCursor(open) {
        const item = list.currentItem
        if (item === null || item.isVolume !== true)
            return
        if (root.volumeExpanded(item.path) === open)
            return
        root.toggleVolume(item.path)
    }

    function sectionLabel(value) {
        switch (value) {
        case "Places": return qsTr("位置")
        case "Favorites": return qsTr("收藏")
        case "Spaces": return qsTr("空间")
        case "Disks": return Qt.platform.os === "osx" ? qsTr("宗卷") : qsTr("磁盘")
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

    /// The model names a row's mark; this table is the one place that decides
    /// which drawn glyph that name becomes. A favourite is a star, a disk is a
    /// drive, and a WSL distro is a network location because that is how it is
    /// reached.
    function glyphKind(icon) {
        switch (icon) {
        case "home": return "home"
        case "desktop": return "desktop"
        case "documents": return "documents"
        case "downloads": return "downloads"
        case "pictures": return "pictures"
        case "music": return "music"
        case "videos": return "videos"
        case "star": return "star"
        case "disk": return "drive"
        case "wsl": return "network"
        case "recent": return "clock"
        case "server": return "network"
        case "cloud": return "cloud"
        case "space": return "diamond"
        case "add": return "plus"
        default: return "file"
        }
    }

    /// Accepts a file drag for one destination and lights that row.
    function armDrop(path, drag) {
        root.dropTargetPath = path
        drag.accept(Qt.MoveAction)
    }

    Component.onCompleted: {
        Spaces.refresh()
        sidebarModel.refresh()
    }

    Connections {
        target: Spaces

        function onChanged() {
            sidebarModel.refresh()
        }
    }

    Connections {
        target: Workspace

        function onFavoritesChanged() {
            sidebarModel.refresh()
        }

        function onVolumesChanged() {
            sidebarModel.refresh()
        }

        function onConnectionsChanged() {
            sidebarModel.refresh()
        }
    }

    Rectangle {
        anchors.fill: parent
        color: root.panelColor

        Rectangle {
            anchors.right: parent.right
            width: 1
            height: parent.height
            color: Theme.hairline
        }
    }

    ListView {
        id: list

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: trashArea.top
        anchors.topMargin: 8
        model: sidebarModel
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        spacing: 0
        section.property: "section"
        section.criteria: ViewSection.FullString

        /// The sidebar is a list of destinations, so the keyboard walks it the
        /// way the pointer does: ↑↓ move the cursor, ↵ opens, → and ← unfold
        /// and fold a volume. Until now the column answered only to a mouse,
        /// which left every place it offers out of reach of the keyboard.
        activeFocusOnTab: true
        currentIndex: -1

        Keys.onUpPressed: function (event) {
            root.moveCursor(-1)
            event.accepted = true
        }
        Keys.onDownPressed: function (event) {
            root.moveCursor(1)
            event.accepted = true
        }
        Keys.onReturnPressed: function (event) {
            root.activateCursor()
            event.accepted = true
        }
        Keys.onEnterPressed: function (event) {
            root.activateCursor()
            event.accepted = true
        }
        Keys.onRightPressed: function (event) {
            root.unfoldCursor(true)
            event.accepted = true
        }
        Keys.onLeftPressed: function (event) {
            root.unfoldCursor(false)
            event.accepted = true
        }

        /// Focus lands on the row the pointer last used, not on the first one:
        /// a keyboard user who tabbed in from the listing wants to keep their
        /// place, and the selected row is that place.
        onActiveFocusChanged: {
            if (!activeFocus || currentIndex >= 0 || count === 0)
                return
            for (let index = 0; index < count; index += 1) {
                const item = itemAtIndex(index)
                if (item !== null && item.focused === true) {
                    currentIndex = index
                    return
                }
            }
            currentIndex = 0
        }

        section.delegate: Item {
            required property string section

            width: ListView.view.width
            height: 24

            Text {
                x: 18
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 4
                text: root.sectionLabel(section)
                color: Theme.inkFaint
                font.pixelSize: Theme.typeMicro
                font.weight: Font.Medium
                font.letterSpacing: 0.88
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
            /// 0..1 for a volume, 0 for everything else.
            required property real fill

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
            /// `!== ""` matters: the add rows have an empty path, and an empty
            /// target would light every one of them at once.
            readonly property bool armed: root.dropTargetPath !== ""
                && root.dropTargetPath === row.path
            readonly property bool focused: row.current || row.spaceCurrent
            /// The keyboard cursor, drawn only while the column holds focus:
            /// an always-visible ring would claim focus the sidebar does not have.
            readonly property bool cursor: list.activeFocus
                && list.currentIndex === row.index
            readonly property bool lit: row.focused || row.armed
                || rowMouse.containsMouse
            readonly property int rowHeight: row.isVolume ? 38 : 28
            readonly property int percent: Math.round(
                Math.max(0, Math.min(1, row.fill)) * 100)

            width: ListView.view.width
            height: rowHeight + (treeLoader.active ? treeLoader.height : 0)

            /// A sidebar row is a destination: the name is what the reader says,
            /// the kind is where it goes, and the current row is the selected
            /// one. The volume's tree below it is a separate list of rows.
            Accessible.role: row.isAction ? Accessible.Button : Accessible.ListItem
            Accessible.name: row.name
            Accessible.description: row.detail
            Accessible.selectable: !row.isAction
            Accessible.selected: row.focused
            Accessible.onPressAction: row.activate()

            /// The one place a row turns into a destination, so the pointer and
            /// a reader's press cannot disagree about what it opens.
            function activate() {
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
                    return
                }
                root.navigateRequested(row.path)
            }

            /// The visible row is only the 28 / 38 px band; an expanded volume's
            /// tree hangs below it. Keeping that distinction explicit means the
            /// icon and label still centre on the row, not on the whole subtree.
            Item {
                id: rowBody

                width: parent.width
                height: row.rowHeight

                Rectangle {
                    id: plate

                    x: 8
                    width: rowBody.width - 16
                    height: rowBody.height
                    radius: Theme.radiusSm
                    color: row.armed
                        ? Theme.signalSoft
                        : (row.focused
                            ? Theme.fill
                            : (rowMouse.containsMouse ? Theme.hover : "transparent"))
                    border.width: row.armed || row.cursor ? 1 : 0
                    border.color: row.cursor ? Theme.signal : Theme.hairlineStrong

                    // How full the volume is: two pixels along the bottom, and
                    // only where there is a number behind it. An empty bar on a
                    // home folder would be a promise the data cannot keep.
                    Item {
                        visible: row.isVolume && row.fill > 0
                        x: 10
                        y: parent.height - 7
                        width: parent.width - 20
                        height: 2

                        Rectangle {
                            anchors.fill: parent
                            radius: 1
                            color: Theme.hairlineStrong
                        }

                        Rectangle {
                            width: parent.width * Math.max(0, Math.min(1, row.fill))
                            height: parent.height
                            radius: 1
                            color: row.fill > 0.9 ? Theme.warn : Theme.signal
                            opacity: row.fill > 0.9 ? 0.9 : 0.5
                        }
                    }
                }

                // The current row's left track sits on the sidebar's own edge,
                // not inside the plate: the 8 px inset is what makes it read as
                // a marker rather than a border.
                Rectangle {
                    visible: row.focused
                    x: 0
                    y: row.isVolume ? 7 : 6
                    width: 2
                    height: rowBody.height - (row.isVolume ? 14 : 12)
                    radius: 1
                    color: Theme.signal
                }

                // A volume's chevron is a separate hit target from the row
                // itself: the label still navigates to the drive, only the
                // glyph toggles the subtree.
                Item {
                    id: chevron

                    visible: row.isVolume
                    x: 8
                    y: 4
                    width: 16
                    height: 20
                    z: 2

                    /// Same split as the tree below: the row goes to the drive,
                    /// this 16 px strip opens the subtree.
                    Accessible.role: Accessible.Button
                    Accessible.name: row.treeOpen ? qsTr("折叠") : qsTr("展开")
                    Accessible.onPressAction: root.toggleVolume(row.path)

                    Glyph {
                        anchors.centerIn: parent
                        width: 12
                        height: 12
                        kind: "chevron"
                        tint: row.lit ? Theme.inkMuted : Theme.inkFaint
                        rotation: row.treeOpen ? 90 : 0
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: root.toggleVolume(row.path)
                    }
                }

                Glyph {
                    id: rowIcon

                    x: row.isVolume ? 26 : 18
                    y: row.isVolume ? 6 : 7
                    width: 14
                    height: 14
                    kind: root.glyphKind(row.icon)
                    tint: row.lit ? Theme.ink : Theme.inkMuted
                }

                Text {
                    id: nameText

                    x: row.isVolume ? 48 : 40
                    y: row.isVolume ? 3 : 0
                    height: row.isVolume ? 20 : 28
                    width: Math.max(0, detailText.x - x - 6)
                    verticalAlignment: Text.AlignVCenter
                    text: row.builtInName ? root.rowLabel(row.name) : row.name
                    color: row.lit ? Theme.ink : Theme.inkMuted
                    font.pixelSize: Theme.typeBody
                    elide: Text.ElideRight
                    textFormat: Text.PlainText
                }

                Text {
                    id: detailText

                    anchors.right: parent.right
                    anchors.rightMargin: 18
                    y: row.isVolume ? 3 : 0
                    height: row.isVolume ? 20 : 28
                    width: Math.min(96, implicitWidth)
                    verticalAlignment: Text.AlignVCenter
                    horizontalAlignment: Text.AlignRight
                    // A narrow column only has room for the percentage; the
                    // full pair is what the tooltip carries instead.
                    text: row.isVolume && root.narrow
                        ? row.percent + "%" : root.detailLabel(row.detail)
                    color: row.isVolume && row.fill > 0.9
                        ? Theme.warn : Theme.inkFaint
                    font.pixelSize: Theme.typeMicro
                    font.features: { "tnum": 1 }
                    elide: Text.ElideRight
                }

                // Files dragged in from the listing or from Explorer. The row
                // owns the area rather than the list, because the target is the
                // row: every destination in a sidebar is a folder.
                DropArea {
                    anchors.fill: parent
                    keys: ["text/uri-list"]
                    enabled: row.isDir && !row.isAction

                    onEntered: function (drag) { root.armDrop(row.path, drag) }
                    onPositionChanged: function (drag) { root.armDrop(row.path, drag) }
                    onExited: {
                        if (root.dropTargetPath === row.path)
                            root.dropTargetPath = ""
                    }

                    onDropped: function (drop) {
                        const paths = Workspace.localDropPaths(drop.urls)
                        const target = row.path
                        root.dropTargetPath = ""
                        if (paths.length === 0 || !row.isDir || row.isAction) {
                            drop.accepted = false
                            return
                        }
                        drop.accept(Qt.MoveAction)
                        Workspace.transferEntries(paths, target, false)
                    }
                }

                MouseArea {
                    id: rowMouse

                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onClicked: function (event) {
                        // The keyboard cursor follows the pointer, so tabbing
                        // back into the column resumes where the mouse left it.
                        list.currentIndex = row.index
                        if (event.button === Qt.RightButton) {
                            if (row.isAction)
                                return
                            const point = row.mapToItem(null, event.x, event.y)
                            root.contextRequested(row.kind, row.path, point.x, point.y)
                            return
                        }
                        row.activate()
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
                y: row.rowHeight
                height: active && status === Loader.Ready && item !== null
                    ? item.implicitHeight : 0
                active: row.treeOpen
                visible: active

                sourceComponent: Component {
                    SidebarTree {
                        width: treeLoader.width
                        dark: root.dark
                        material: root.material
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

    /// The bin is a fixed destination at the bottom, not a path the model
    /// resolves, and not a header item next to the overview.
    Item {
        id: trashArea

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 42

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            height: 1
            color: Theme.hairline
        }

        Rectangle {
            id: trashRow

            x: 8
            y: 7
            width: parent.width - 16
            height: 28
            radius: Theme.radiusSm
            color: root.trashActive
                ? Theme.fill
                : (trashMouse.containsMouse ? Theme.hover : "transparent")

            Accessible.role: Accessible.Button
            Accessible.name: root.trashLabel
            Accessible.onPressAction: root.trashRequested()

            Rectangle {
                visible: root.trashActive
                // Like a list row, the track sits on the sidebar's edge, not
                // on the plate's 8 px inset.
                x: -8
                y: 6
                width: 2
                height: 16
                radius: 1
                color: Theme.signal
            }

            Glyph {
                x: 10
                y: 7
                width: 14
                height: 14
                kind: "trash"
                tint: root.trashActive || trashMouse.containsMouse
                    ? Theme.ink : Theme.inkMuted
            }

            Text {
                x: 32
                anchors.verticalCenter: parent.verticalCenter
                text: root.trashLabel
                color: root.trashActive || trashMouse.containsMouse
                    ? Theme.ink : Theme.inkMuted
                font.pixelSize: Theme.typeBody
            }

            Text {
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                visible: root.trashCount > 0
                text: qsTr("%1 项").arg(root.trashCount)
                color: Theme.inkFaint
                font.pixelSize: Theme.typeMicro
                font.features: { "tnum": 1 }
            }

            MouseArea {
                id: trashMouse
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.trashRequested()
            }
        }
    }
}
