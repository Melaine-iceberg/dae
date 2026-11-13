pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The navigation column: the five places people reach for, then extra places
// and favourites, volumes, network storage and cloud accounts. The
// trash is parked at the bottom, where a bin belongs, instead of sharing a
// header with the overview. `SidebarModel` decides the order and resolves the
// rows; this file decides what a row looks like and where a click goes.
Item {
    id: root

    property bool dark: true
    property string material: "none"
    property string activePath: ""
    property bool trashActive: false
    property int trashCount: 0

    /// The volume tree expansion lives on the sidebar, not in the delegate:
    /// ListView recycles delegates, and a collapsed-then-restored volume must
    /// come back expanded. `volumeRevision` exists only to make the delegate's
    /// binding re-evaluate when the map is replaced.
    property var expandedVolumes: ({})
    /// Volumes whose tree has been opened at least once. The loader stays
    /// alive after a collapse, so the model keeps the folders the user had
    /// expanded and reopening a volume is instant instead of re-reading its
    /// root and flashing the loading line.
    property var openedVolumes: ({})
    property int volumeRevision: 0

    signal navigateRequested(string path)
    signal trashRequested()
    /// A tag row was clicked. The shell turns this into a `tag:` search.
    signal tagActivated(string name)
    /// A saved search was clicked: the shell restores its query and filters.
    signal savedSearchActivated(string id)
    /// The remove mark on a saved search was clicked.
    signal savedSearchRemoveRequested(string id)
    signal addRequested(string kind)
    signal restorePlacesRequested()
    signal contextRequested(string kind, string path, bool removable, real x, real y)
    /// A discovered server was clicked. The shell pre-fills the connect dialog
    /// with protocol, host and port; credentials stay the user's to type.
    signal discoveredActivated(string protocol, string host, string port)
    /// A folder row could not be expanded. The window owns the toast.
    signal errorRequested(string message)

    /// A folder drag is hovering the favorites group, which is the add target.
    /// Set by the section header and the empty hint row, so both light while
    /// the pointer is over them.
    property bool favoriteDropActive: false

    /// Narrow columns drop the "used / total" pair: at 184 px it is the first
    /// thing that would collide with the filesystem. The percentage on the
    /// name line still says how full the drive is.
    readonly property bool narrow: width < 200
    readonly property string trashLabel: Qt.platform.os === "osx"
        ? qsTr("废纸篓") : qsTr("回收站")

    readonly property color panelColor: Theme.backdropTint(material, dark, true)

    /// mDNS answers, as the bridge publishes them. Empty until the first scan
    /// finishes; a malformed string is treated as "nothing found" rather than
    /// breaking the column.
    readonly property var discovered: {
        const raw = NetworkDiscovery.servicesJson
        if (!raw || raw === "[]")
            return []
        try {
            return JSON.parse(raw)
        } catch (error) {
            return []
        }
    }

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

    function volumeOpened(path) {
        return volumeRevision >= 0 && openedVolumes[path] === true
    }

    function toggleVolume(path) {
        if (path === "")
            return
        const next = ({})
        for (const key in expandedVolumes)
            next[key] = expandedVolumes[key]
        const opening = next[path] !== true
        if (opening)
            next[path] = true
        else
            delete next[path]
        expandedVolumes = next
        if (opening) {
            const opened = ({})
            for (const key in openedVolumes)
                opened[key] = openedVolumes[key]
            opened[path] = true
            openedVolumes = opened
        }
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
        case "Disks": return Qt.platform.os === "osx" ? qsTr("宗卷") : qsTr("磁盘")
        case "Network Storage": return qsTr("网络存储")
        case "Cloud Storage": return qsTr("云存储")
        default: return value
        }
    }

    /// `name` comes from `SidebarModel`; translating only the built-in places
    /// and action rows keeps a favourite entry named "文档" intact.
    function rowLabel(name) {
        switch (name) {
        case "Home": return qsTr("主目录")
        case "Desktop": return qsTr("桌面")
        case "Documents": return qsTr("文档")
        case "Downloads": return qsTr("下载")
        case "Pictures": return qsTr("图片")
        case "Music": return qsTr("音乐")
        case "Videos": return qsTr("视频")
        case "Connect network storage…": return qsTr("连接网络存储…")
        case "Add cloud account…": return qsTr("添加网盘账户…")
        case "Drop folders here to favorite them": return qsTr("将文件夹拖到这里加入收藏")
        case "Restore hidden places": return qsTr("恢复隐藏的位置")
        default: return name
        }
    }

    function detailLabel(value) {
        const count = /^(\d+) items?$/.exec(value)
        if (count)
            return qsTr("%1 项").arg(count[1])
        const hidden = /^(\d+) hidden$/.exec(value)
        if (hidden)
            return qsTr("已隐藏 %1 项").arg(hidden[1])
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
        case "server": return "network"
        case "cloud": return "cloud"
        case "add": return "plus"
        case "eye": return "eye"
        default: return "file"
        }
    }

    /// A folder drag lands here from the listing or Explorer. Only local
    /// folders are pinned; `addFavoritePaths` drops the rest.
    function dropFavorites(drag) {
        const paths = Workspace.localDropPaths(drag.urls)
        if (paths.length === 0) {
            drag.accepted = false
            return
        }
        drag.accept(Qt.CopyAction)
        Workspace.addFavoritePaths(paths)
        root.favoriteDropActive = false
    }

    /// Accepts a file drag for one destination and lights that row.
    function armDrop(path, drag) {
        root.dropTargetPath = path
        drag.accept(Qt.MoveAction)
    }

    Component.onCompleted: {
        sidebarModel.refresh()
        // One probe per process: a second window reuses the answer and lets
        // the header's refresh affordance ask again.
        if (!NetworkDiscovery.scanned)
            NetworkDiscovery.refresh()
    }

    Connections {
        target: Workspace

        function onFavoritesChanged() {
            sidebarModel.refresh()
        }

        function onPlacesChanged() {
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

    /// The column's masthead. drift opens its sidebar with a wordmark; the
    /// window's title bar carries a tab strip, so this is the only place the
    /// product's name is printed rather than resolved from a tab.
    Item {
        id: masthead
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 62

        Text {
            id: wordmark
            x: 18
            y: 10
            text: "dae"
            color: Theme.ink
            font.family: Theme.serifFamily
            font.italic: true
            font.pixelSize: 38
            font.letterSpacing: -1
            /// Decorative: the window's title and its accessible name already
            /// say which application this is, and a reader that lands in the
            /// sidebar wants the destinations, not the brand.
            Accessible.ignored: true
        }

        Text {
            anchors.left: wordmark.right
            anchors.leftMargin: 1
            anchors.baseline: wordmark.baseline
            text: "."
            color: Theme.signal
            font.family: Theme.serifFamily
            font.italic: true
            font.pixelSize: 38
            Accessible.ignored: true
        }
    }

    ListView {
        id: list

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: masthead.bottom
        anchors.bottom: discoveryArea.top
        anchors.topMargin: 4
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
                // The delegate's `focused` is declared on its anonymous Item
                // type, which qmllint cannot see through `itemAtIndex`.
                // qmllint disable missing-property
                if (item !== null && item.focused === true) {
                    currentIndex = index
                    return
                }
            }
            currentIndex = 0
        }

        section.delegate: Item {
            id: sectionRow

            required property string section

            readonly property bool favoriteSection: section === "Favorites"
            readonly property bool lit: favoriteSection && root.favoriteDropActive

            width: ListView.view.width
            height: Theme.controlXs

            Text {
                x: 18
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 4
                text: root.sectionLabel(sectionRow.section)
                color: parent.lit ? Theme.signal : Theme.inkFaint
                font.family: Theme.monoFamily
                font.pixelSize: Theme.typeMicro
                font.weight: Font.Medium
                font.letterSpacing: 0.88
            }

            Text {
                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 4
                visible: parent.lit && parent.favoriteSection
                text: qsTr("松开以加入收藏")
                color: Theme.signal
                font.pixelSize: Theme.typeMicro
            }

            /// The group header is the favorites drop target while the group
            /// has rows; the empty hint row carries one when it does not.
            DropArea {
                anchors.fill: parent
                keys: ["text/uri-list"]
                enabled: parent.favoriteSection

                onEntered: function (drag) {
                    root.favoriteDropActive = true
                    drag.accept(Qt.CopyAction)
                }
                onPositionChanged: function (drag) {
                    root.favoriteDropActive = true
                    drag.accept(Qt.CopyAction)
                }
                onExited: {
                    if (parent.favoriteSection)
                        root.favoriteDropActive = false
                }
                onDropped: function (drop) {
                    root.dropFavorites(drop)
                }
            }
        }

        delegate: Item {
            id: row

            /// The click handler and the keyboard cursor both address the row
            /// by index. Qt Quick only injects `index` into a delegate that
            /// declares it, so without this the assignment below threw before
            /// `activate()` ran and every row in the column was unclickable.
            required property int index
            required property string name
            required property string path
            required property string kind
            required property string detail
            required property string icon
            required property bool isDir
            /// NTFS, APFS, ext4… Empty for everything that is not a volume.
            required property string fileSystem
            /// 0..1 for a volume, 0 for everything else.
            required property real fill

            readonly property bool isVolume: row.kind === "volume"
            readonly property bool treeOpen: row.isVolume && root.volumeExpanded(row.path)
            readonly property bool current: !root.trashActive
                && root.pathEquals(row.path, root.activePath)
            readonly property bool isAction: row.kind === "addConnection"
                || row.kind === "addCloud"
                || row.kind === "restorePlaces"
            /// A place is a named destination in the first group: it carries a
            /// number instead of a glyph, the way drift indexes the column.
            readonly property bool place: !row.isVolume && !row.isAction
                && row.kind !== "favoriteHint"
            readonly property bool builtInName: row.isAction
                || row.kind === "favoriteHint"
                || row.icon === "home" || row.icon === "desktop"
                || row.icon === "documents" || row.icon === "downloads"
                || row.icon === "pictures" || row.icon === "music"
                || row.icon === "videos"
            /// `!== ""` matters: the add rows have an empty path, and an empty
            /// target would light every one of them at once.
            readonly property bool armed: root.dropTargetPath !== ""
                && root.dropTargetPath === row.path
            readonly property bool focused: row.current
            /// The keyboard cursor, drawn only while the column holds focus:
            /// an always-visible ring would claim focus the sidebar does not have.
            readonly property bool cursor: list.activeFocus
                && list.currentIndex === row.index
            readonly property bool lit: row.focused || row.armed
                || rowMouse.containsMouse
            /// A volume is three bands — name and percentage, the capacity
            /// gauge, then the filesystem and the used/total pair — so the
            /// gauge can be as wide as the row without crowding either line.
            readonly property int rowHeight: row.isVolume ? 58 : 28
            readonly property int percent: Math.round(
                Math.max(0, Math.min(1, row.fill)) * 100)
            /// The gauge's cell count, the same 24 drift prints: fine enough
            /// that one cell is ~4 %, coarse enough that it reads as a printed
            /// gauge rather than a smooth meter.
            readonly property int gaugeSegments: 24
            readonly property int litSegments: Math.round(
                Math.max(0, Math.min(1, row.fill)) * row.gaugeSegments)

            width: ListView.view.width
            height: rowHeight + (row.treeOpen ? treeLoader.height : 0)

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
                if (row.kind === "restorePlaces") {
                    root.restorePlacesRequested()
                    return
                }
                if (row.kind === "favoriteHint")
                    return
                root.navigateRequested(row.path)
            }

            /// The visible row is only the 28 / 46 px band; an expanded volume's
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
                    /// The current destination is a solid ink band with the
                    /// paper showing through the type. A drop target is the
                    /// only row that gets a signal wash and a dashed frame,
                    /// which keeps it apart from the solid keyboard ring.
                    color: row.armed ? Theme.signalSoft
                        : row.focused ? Theme.ink
                        : (rowMouse.containsMouse ? Theme.hover : "transparent")
                    border.width: row.cursor && !row.armed ? Theme.edgeWidth : 0
                    border.color: Theme.signal
                }

                /// The plate's edge for a drop target: dashed, where the
                /// keyboard cursor's ring is solid.
                DashFrame {
                    anchors.fill: plate
                    visible: row.armed
                }

                /// The index of a place, in mono. It replaces the glyph: a
                /// number says "the third destination" and survives being read
                /// at a glance, where six outline icons at 14 dp only say
                /// "somewhere".
                Text {
                    id: placeIndex
                    visible: row.place
                    x: 18
                    y: 0
                    height: rowBody.height
                    width: 20
                    verticalAlignment: Text.AlignVCenter
                    text: String(row.index + 1).padStart(2, "0")
                    color: row.focused ? Theme.signal : Theme.inkFaint
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeMicro
                }

                // A volume's chevron is a separate hit target from the row
                // itself: the label still navigates to the drive, only the
                // glyph toggles the subtree.
                Item {
                    id: chevron

                    visible: row.isVolume
                    x: 8
                    y: 3
                    width: 16
                    height: Theme.controlXs
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

                    visible: !row.place
                    x: row.isVolume ? 26 : 18
                    y: row.isVolume ? 6 : 7
                    width: 14
                    height: 14
                    kind: root.glyphKind(row.icon)
                    tint: row.lit ? Theme.ink : Theme.inkMuted
                }

                Text {
                    id: nameText

                    x: row.isVolume ? 46 : 40
                    y: row.isVolume ? 3 : 0
                    height: row.isVolume ? 20 : 28
                    // A volume's percentage owns the right edge of the name
                    // line, the used/total pair the last line, so the drive name
                    // elides only against the smaller of the two.
                    width: Math.max(0, (row.isVolume
                        ? percentText.x
                        : detailText.x) - x - 6)
                    verticalAlignment: Text.AlignVCenter
                    text: row.builtInName ? root.rowLabel(row.name) : row.name
                    color: row.kind === "favoriteHint" && root.favoriteDropActive
                        ? Theme.signal
                        : row.focused && !row.armed ? Theme.canvas
                        : (row.lit ? Theme.ink : Theme.inkMuted)
                    font.pixelSize: Theme.typeBody
                    elide: Text.ElideRight
                    textFormat: Text.PlainText
                }

                Text {
                    id: percentText

                    visible: row.isVolume
                    anchors.right: parent.right
                    anchors.rightMargin: 18
                    y: 3
                    height: 20
                    width: implicitWidth
                    verticalAlignment: Text.AlignVCenter
                    horizontalAlignment: Text.AlignRight
                    text: row.percent + "%"
                    color: row.focused && !row.armed ? Theme.canvas
                        : row.fill > 0.9 ? Theme.warn : Theme.inkFaint
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.typeMicro
                    font.features: { "tnum": 1 }
                }

                /// The capacity gauge, in drift's segmented form. It sits
                /// between the name and the filesystem so the bar can use the
                /// row's width instead of a 112 px strip on a text line.
                Row {
                    id: gauge

                    visible: row.isVolume
                    x: 46
                    y: 26
                    width: Math.max(0, rowBody.width - 46 - 18)
                    height: 10
                    spacing: 2

                    Repeater {
                        model: row.isVolume ? row.gaugeSegments : 0

                        Rectangle {
                            required property int index
                            /// The gaps come off the width first; a cell sized
                            /// from the bar alone would overflow the right
                            /// margin by the last gap.
                            width: Math.max(1, (gauge.width
                                - (row.gaugeSegments - 1) * 2) / row.gaugeSegments)
                            height: gauge.height
                            color: index < row.litSegments
                                ? (row.focused && !row.armed ? Theme.canvas
                                    : row.fill > 0.9 ? Theme.warn : Theme.ink)
                                : Theme.hairline
                        }
                    }
                }

                Text {
                    id: detailText

                    visible: !row.isVolume || !root.narrow
                    anchors.right: parent.right
                    anchors.rightMargin: 18
                    y: row.isVolume ? 40 : 0
                    height: row.isVolume ? 16 : 28
                    // A volume keeps the pair readable beside the filesystem;
                    // anything else is capped to a strip on the name line. The
                    // filesystem's own width is what is reserved, so a longer
                    // label ("fuseblk") eats the pair rather than disappearing.
                    width: Math.min(implicitWidth, row.isVolume
                        ? Math.max(0, rowBody.width - 46 - 18
                            - (row.fileSystem !== ""
                                ? fileSystemText.implicitWidth + 6 : 0))
                        : 112)
                    verticalAlignment: Text.AlignVCenter
                    horizontalAlignment: Text.AlignRight
                    // A narrow column drops the pair; the percentage on the
                    // name line still answers how full the drive is.
                    text: root.detailLabel(row.detail)
                    color: row.focused && !row.armed ? Theme.canvas
                        : row.isVolume && row.fill > 0.9
                            ? Theme.warn : Theme.inkFaint
                    font.pixelSize: Theme.typeMicro
                    font.features: { "tnum": 1 }
                    elide: Text.ElideRight
                }

                // The filesystem is the one thing a drive row could not say in
                // one line; it shares that line with the used/total pair.
                Text {
                    id: fileSystemText

                    visible: row.isVolume && row.fileSystem !== ""
                    x: 46
                    y: 40
                    height: 16
                    width: Math.max(0, (root.narrow
                        ? rowBody.width - 18 : detailText.x) - x - 6)
                    verticalAlignment: Text.AlignVCenter
                    text: row.fileSystem
                    color: row.focused && !row.armed ? Theme.canvas : Theme.inkFaint
                    font.pixelSize: Theme.typeMicro
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

                /// The empty favorites group pins whatever folder is dropped
                /// on it; the header carries the same target once it has rows.
                DropArea {
                    anchors.fill: parent
                    keys: ["text/uri-list"]
                    enabled: row.kind === "favoriteHint"

                    onEntered: function (drag) {
                        root.favoriteDropActive = true
                        drag.accept(Qt.CopyAction)
                    }
                    onPositionChanged: function (drag) {
                        root.favoriteDropActive = true
                        drag.accept(Qt.CopyAction)
                    }
                    onExited: root.favoriteDropActive = false
                    onDropped: function (drop) { root.dropFavorites(drop) }
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
                            if (row.isAction || row.kind === "favoriteHint")
                                return
                            const point = row.mapToItem(null, event.x, event.y)
                            root.contextRequested(
                                row.kind === "place" ? "place:" + row.icon : row.kind,
                                row.path, row.removable === true, point.x, point.y)
                            return
                        }
                        row.activate()
                    }
                }
            }

            // One lazily loaded tree per volume. The loader stays active after
            // the first open so a collapse does not throw away the model's
            // expansion set or re-read the root; it is only hidden and zero
            // high. `openedVolumes` is what keeps it from loading a volume the
            // user has never opened.
            Loader {
                id: treeLoader

                anchors.left: parent.left
                anchors.right: parent.right
                y: row.rowHeight
                height: row.treeOpen && active && status === Loader.Ready && item !== null
                    ? (item as SidebarTree).implicitHeight : 0
                active: row.treeOpen || (row.isVolume && root.volumeOpened(row.path))
                visible: row.treeOpen

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
                            root.contextRequested("tree", path, false, x, y)
                        }
                        onErrorRequested: function (message) {
                            root.errorRequested(message)
                        }
                    }
                }

                onLoaded: {
                    if (item !== null)
                        (item as SidebarTree).rootPath = row.path
                }
            }
        }
    }

    /// mDNS advertisers, above the tags. These are not saved connections: a
    /// click only pre-fills the connect dialog with protocol, host and port,
    /// because credentials — and whether this is the server the user means —
    /// are still theirs to confirm. The list is capped like the tag list so a
    /// noisy network cannot push the bin off the column.
    Item {
        id: discoveryArea

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: savedArea.top
        visible: root.discovered.length > 0
        height: visible ? 26 + serverList.height + 10 : 0

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            anchors.bottom: parent.bottom
            height: 1
            color: Theme.hairline
        }

        Text {
            x: 18
            y: 8
            text: qsTr("网络")
            color: Theme.inkFaint
            font.pixelSize: Theme.typeMicro
            font.weight: Font.Medium
            font.letterSpacing: 0.88
        }

        Glyph {
            id: refreshMark
            anchors.right: parent.right
            anchors.rightMargin: 14
            y: 8
            width: 12
            height: 12
            visible: !NetworkDiscovery.busy
            kind: "refresh"
            tint: refreshMouse.containsMouse ? Theme.inkMuted : Theme.inkFaint

            MouseArea {
                id: refreshMouse
                Accessible.role: Accessible.Button
                Accessible.name: qsTr("刷新网络设备")
                anchors.fill: parent
                anchors.margins: -4
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: NetworkDiscovery.refresh()
            }
        }

        Text {
            anchors.right: parent.right
            anchors.rightMargin: 14
            y: 8
            visible: NetworkDiscovery.busy
            text: qsTr("搜索中…")
            color: Theme.inkFaint
            font.pixelSize: Theme.typeMicro
        }

        ListView {
            id: serverList
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.topMargin: 24
            height: Math.min(contentHeight, 3 * 24)
            clip: true
            interactive: contentHeight > height
            boundsBehavior: Flickable.StopAtBounds
            model: root.discovered

            delegate: Item {
                id: serverRow
                required property var modelData

                width: ListView.view.width
                height: 24

                readonly property bool hovered: serverMouse.containsMouse
                readonly property string label: modelData.name !== ""
                    ? modelData.name : modelData.host

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    radius: Theme.radiusSm
                    color: serverRow.hovered ? Theme.hover : "transparent"
                }

                Glyph {
                    x: 18
                    width: 12
                    height: 12
                    anchors.verticalCenter: parent.verticalCenter
                    kind: "network"
                    tint: Theme.inkFaint
                }

                Text {
                    x: 36
                    anchors.right: protocolText.left
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    text: serverRow.label
                    color: Theme.inkMuted
                    font.pixelSize: Theme.typeBody
                    elide: Text.ElideRight
                }

                Text {
                    id: protocolText
                    anchors.right: parent.right
                    anchors.rightMargin: 18
                    anchors.verticalCenter: parent.verticalCenter
                    text: String(serverRow.modelData.protocol).toUpperCase()
                    color: Theme.inkFaint
                    font.pixelSize: Theme.typeMicro
                    font.features: { "tnum": 1 }
                }

                MouseArea {
                    id: serverMouse
                    Accessible.role: Accessible.ListItem
                    Accessible.name: qsTr("连接到 %1").arg(serverRow.modelData.host)
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.discoveredActivated(serverRow.modelData.protocol,
                                                         serverRow.modelData.host,
                                                         String(serverRow.modelData.port))
                }
            }
        }
    }

    /// Saved searches are not paths either: a row re-runs the query with the
    /// mode, scope and filters it was stored with. The list is capped and
    /// scrolls, like the tag list, so a long list cannot push the bin off the
    /// column.
    Item {
        id: savedArea

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: tagsArea.top
        visible: savedSearches.length > 0
        height: visible ? 26 + savedList.height + 10 : 0

        /// A malformed document reads as "no saved searches" rather than
        /// breaking the column; the preference setter already drops entries
        /// that could never run.
        readonly property var savedSearches: {
            try {
                const parsed = JSON.parse(ExplorerPrefs.savedSearches)
                return Array.isArray(parsed) ? parsed : []
            } catch (error) {
                return []
            }
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            anchors.bottom: parent.bottom
            height: 1
            color: Theme.hairline
        }

        Text {
            x: 18
            y: 8
            text: qsTr("保存的搜索")
            color: Theme.inkFaint
            font.pixelSize: Theme.typeMicro
            font.weight: Font.Medium
            font.letterSpacing: 0.88
        }

        ListView {
            id: savedList
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.topMargin: 24
            height: Math.min(contentHeight, 4 * 24)
            clip: true
            interactive: contentHeight > height
            boundsBehavior: Flickable.StopAtBounds
            model: savedArea.savedSearches

            delegate: Item {
                id: savedRow
                required property var modelData

                width: ListView.view.width
                height: 24

                readonly property bool hovered: savedMouse.containsMouse

                Accessible.role: Accessible.Button
                Accessible.name: savedRow.modelData.name
                Accessible.description: savedRow.modelData.query

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    radius: Theme.radiusSm
                    color: savedRow.hovered ? Theme.hover : "transparent"
                }

                Glyph {
                    x: 20
                    anchors.verticalCenter: parent.verticalCenter
                    width: 12
                    height: 12
                    kind: "search"
                    tint: Theme.inkFaint
                }

                Text {
                    id: savedName

                    x: 38
                    anchors.right: parent.right
                    anchors.rightMargin: savedRow.hovered ? 32 : 18
                    anchors.verticalCenter: parent.verticalCenter
                    text: savedRow.modelData.name
                    color: Theme.inkMuted
                    font.pixelSize: Theme.typeBody
                    elide: Text.ElideRight
                    textFormat: Text.PlainText
                }

                MouseArea {
                    id: savedMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.savedSearchActivated(savedRow.modelData.id)
                }

                /// Only while the row is hovered: a permanent cross on every
                /// row would read as "delete" before it read as "this search".
                Item {
                    id: savedRemove

                    anchors.right: parent.right
                    anchors.rightMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    width: 16
                    height: 16
                    visible: savedRow.hovered

                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("删除保存的搜索“%1”").arg(savedRow.modelData.name)
                    Accessible.onPressAction: root.savedSearchRemoveRequested(
                        savedRow.modelData.id)

                    Glyph {
                        anchors.centerIn: parent
                        width: 9
                        height: 9
                        kind: "close"
                        tint: Theme.inkFaint
                    }

                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: root.savedSearchRemoveRequested(savedRow.modelData.id)
                    }
                }
            }
        }
    }

    /// File tags are not paths, so they are not rows of `SidebarModel`: a
    /// click runs a `tag:` search and a drop writes the tag onto the files.
    /// The list is capped and scrolls, so a user with forty tags does not push
    /// the bin off the column.
    Item {
        id: tagsArea

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: trashArea.top
        visible: tags.length > 0
        height: visible ? 26 + tagList.height + 10 : 0

        readonly property var tags: {
            try {
                return JSON.parse(TagStore.tagsJson)
            } catch (error) {
                return []
            }
        }

        function dropTag(drag, tagId) {
            const paths = Workspace.localDropPaths(drag.urls)
            if (paths.length === 0) {
                drag.accepted = false
                return
            }
            drag.accept(Qt.CopyAction)
            TagStore.setTags(paths, [tagId])
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            anchors.bottom: parent.bottom
            height: 1
            color: Theme.hairline
        }

        Text {
            x: 18
            y: 8
            text: qsTr("标签")
            color: Theme.inkFaint
            font.pixelSize: Theme.typeMicro
            font.weight: Font.Medium
            font.letterSpacing: 0.88
        }

        ListView {
            id: tagList
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.topMargin: 24
            height: Math.min(contentHeight, 6 * 24)
            clip: true
            interactive: contentHeight > height
            boundsBehavior: Flickable.StopAtBounds
            model: tagsArea.tags

            delegate: Item {
                id: tagRow
                required property var modelData

                width: ListView.view.width
                height: 24

                readonly property bool hovered: tagMouse.containsMouse
                readonly property bool armed: tagDrop.containsDrag

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    radius: Theme.radiusSm
                    color: tagRow.armed ? Theme.signalSoft
                        : (tagRow.hovered ? Theme.hover : "transparent")
                }

                Rectangle {
                    x: 20
                    width: 8
                    height: 8
                    radius: 4
                    anchors.verticalCenter: parent.verticalCenter
                    color: tagRow.modelData.color
                }

                Text {
                    x: 36
                    anchors.right: countText.left
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    text: tagRow.modelData.name
                    color: Theme.inkMuted
                    font.pixelSize: Theme.typeBody
                    elide: Text.ElideRight
                }

                Text {
                    id: countText
                    anchors.right: parent.right
                    anchors.rightMargin: 18
                    anchors.verticalCenter: parent.verticalCenter
                    visible: tagRow.modelData.count > 0
                    text: tagRow.modelData.count
                    color: Theme.inkFaint
                    font.pixelSize: Theme.typeMicro
                    font.features: { "tnum": 1 }
                }

                DropArea {
                    id: tagDrop
                    anchors.fill: parent
                    keys: ["text/uri-list"]
                    onDropped: function (drag) {
                        tagsArea.dropTag(drag, tagRow.modelData.id)
                    }
                }

                MouseArea {
                    id: tagMouse
                    Accessible.role: Accessible.ListItem
                    Accessible.name: tagRow.modelData.name
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.tagActivated(tagRow.modelData.name)
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
            height: Theme.controlMd
            radius: Theme.radiusSm
            color: root.trashActive
                ? Theme.signalSoft
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
                width: 3
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
