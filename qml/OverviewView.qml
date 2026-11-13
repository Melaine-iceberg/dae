pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// The landing page: a pinboard, not a list.
//
// The page used to be one sectioned `ListView`. A row is the denser shape when
// the job is scanning a column of names, but nothing here is a column: the
// model publishes at most 34 destinations across six groups, and every row
// spent one 32 dp band on one name plus one fact. A band of the same height on
// this board holds a whole row of destinations, which is what "information
// density" means on a page whose items are places you can continue from.
//
// Each destination is a note of sticky paper with a pin through its head. The
// material is drawn rather than photographed: a flat plate in one of six paper
// stocks, a 1 px ink edge, the shell's hard offset shadow (`HardShadow`, which
// is also what makes it read as a sheet laid on the page instead of a card
// hovering over it), and a metal pin with a cast shadow. A note is tilted by a
// hash of its row, so a given row always lands on the same tilt and the same
// stock while the board never looks ruled.
//
// The grid is a `Column` of `Flow`s, not a `GridView`: a section has to keep
// its own heading and its own last row, and a Flow does that without a
// delegate that pretends to be a header. The model is a `QAbstractListModel`
// with roles, but a `Repeater` has no roles to bind, so the board asks the
// model for its rows through the `*At` accessors (the same ones the command
// palette uses) and keeps the flattened notes in `sections`.
Item {
    id: root

    property var model: null
    property color accent: Theme.interactive
    property color textColor: Theme.ink
    property color mutedColor: Theme.inkMuted
    property color hairline: Theme.hairline
    property string commandShortcut: "Ctrl+K"

    signal openRequested(string kind, string path)
    signal commandPaletteRequested()

    readonly property color contentColor: Theme.content

    // ── Board geometry ──────────────────────────────────────────────────────
    /// The readable measure. Wider than the list's 832 because a board is read
    /// across, not down: at 1180 the same window holds five notes a row.
    readonly property int contentMax: 1180
    readonly property int sideMargin: 24
    readonly property int contentWidth: Math.max(320,
        Math.min(width - sideMargin * 2, contentMax))
    readonly property int noteGap: 14
    readonly property int noteHeight: 132
    /// Narrowest a note may get before the board drops a column. Two lines of
    /// a 14 px name plus one meta line need ~190: below that names elide to
    /// indistinguishable stamps and the note stops paying for its paper.
    readonly property int noteMin: 190
    readonly property int columnMax: 6
    readonly property int columns: {
        const fit = Math.floor((root.contentWidth + root.noteGap)
                               / (root.noteMin + root.noteGap));
        return Math.max(2, Math.min(root.columnMax, fit));
    }
    readonly property real noteWidth:
        Math.floor((root.contentWidth - root.noteGap * (root.columns - 1))
                   / root.columns)

    /// Sticky stock. Six tints, warm and low-chroma enough to sit on drift's
    /// paper canvas without turning the page into a swatch chart; the dark set
    /// is paper in a dark room — same hue, lower value — because a full-bright
    /// note beside a #131210 canvas is a lamp, not a note.
    readonly property var palette: Theme.dark
        ? ["#4a4023", "#4a2f2c", "#2f4030", "#2b3a47", "#3b3347", "#463a26"]
        : ["#f7e58a", "#f6cfc4", "#cfe3c2", "#c8dcec", "#ded1e6", "#f2ddb8"]

    // ── Board contents ──────────────────────────────────────────────────────
    /// `[{ name, label, notes: [{ row, index, section, tint, tilt, kind, … }] }]`
    /// rebuilt from the model whenever it republishes. Notes are plain objects
    /// on purpose: a delegate can then bind `modelData.name` once instead of
    /// calling a model accessor per property per frame.
    property var sections: []
    property int cursorRow: -1
    readonly property int firstRow: root.sections.length > 0
        && root.sections[0].notes.length > 0 ? root.sections[0].notes[0].row : -1

    /// Live delegates by model row, for the keyboard: the cursor moves focus
    /// between notes, and only the delegate knows how to take it.
    property var noteItems: ({})

    function takeFocus() {
        if (root.firstRow < 0) {
            board.forceActiveFocus();
            return;
        }
        root.focusRow(root.cursorRow >= 0 ? root.cursorRow : root.firstRow);
    }

    function focusRow(row) {
        root.cursorRow = row;
        const item = root.noteItems[row];
        if (item !== undefined)
            item.forceActiveFocus();
    }

    function registerNote(row, item) {
        root.noteItems[row] = item;
    }

    function unregisterNote(row) {
        delete root.noteItems[row];
    }

    /// A focus move must bring the note on screen; the board scrolls with the
    /// pointer otherwise, which leaves a keyboard user driving blind.
    ///
    /// Only the keyboard calls this. Doing it from `activeFocusChanged` also
    /// caught focus restored by the window, which lands mid-resize when the
    /// viewport is a few dozen dp tall: the board then scrolled past the first
    /// row and stayed there.
    function reveal(item) {
        if (board.height <= 0 || item.height <= 0)
            return;
        const point = item.mapToItem(stack, 0, 0);
        const top = stack.y + point.y - 10;
        const bottom = top + item.height + 26;
        if (top < board.contentY)
            board.contentY = Math.max(0, top);
        else if (bottom > board.contentY + board.height)
            board.contentY = Math.min(bottom - board.height,
                                      Math.max(0, board.contentHeight - board.height));
    }

    function focusAndReveal(row) {
        root.focusRow(row);
        const item = root.noteItems[row];
        if (item !== undefined)
            root.reveal(item);
    }

    /// One arrow press, from the note that has focus. Left/right cross into the
    /// neighbouring section at a row's edge because the board is one surface;
    /// up/down stay inside the section, where the rows actually are.
    function moveCursor(entry, dx, dy) {
        const section = root.sections[entry.section];
        if (section === undefined)
            return;
        const count = section.notes.length;
        const step = entry.index + dx;
        let target = step;

        if (dy !== 0) {
            const rows = Math.ceil(count / root.columns);
            const atRow = Math.floor(entry.index / root.columns);
            const atColumn = entry.index % root.columns;
            const nextRow = atRow + dy;
            if (nextRow < 0 || nextRow >= rows)
                return;
            target = nextRow * root.columns + atColumn;
            if (target >= count)
                return;
        }

        if (target < 0) {
            for (let index = entry.section - 1; index >= 0; --index) {
                const before = root.sections[index].notes;
                if (before.length > 0) {
                    root.focusAndReveal(before[before.length - 1].row);
                    return;
                }
            }
            return;
        }
        if (target >= count) {
            for (let index = entry.section + 1; index < root.sections.length; ++index) {
                const after = root.sections[index].notes;
                if (after.length > 0) {
                    root.focusAndReveal(after[0].row);
                    return;
                }
            }
            return;
        }
        root.focusAndReveal(section.notes[target].row);
    }

    function rebuild() {
        if (root.model === null) {
            root.sections = [];
            return;
        }
        const order = ["Projects", "Favorites", "Recent", "Spaces", "Tags", "Drives"];
        const buckets = ({});
        const total = root.model.count;
        for (let row = 0; row < total; ++row) {
            const section = root.model.sectionAt(row);
            if (buckets[section] === undefined)
                buckets[section] = [];
            buckets[section].push(row);
        }

        const sections = [];
        const names = [];
        for (let index = 0; index < order.length; ++index) {
            if (buckets[order[index]] !== undefined)
                names.push(order[index]);
        }
        // A model that grows a seventh section still renders: unknown names
        // keep their first-seen order after the known ones instead of
        // vanishing, which is what a stale `order` may not know about.
        for (const name in buckets) {
            if (order.indexOf(name) < 0)
                names.push(name);
        }

        let placement = 0;
        for (let index = 0; index < names.length; ++index) {
            const name = names[index];
            const rows = buckets[name];
            if (rows === undefined || rows.length === 0)
                continue;
            const notes = [];
            for (let at = 0; at < rows.length; ++at) {
                const row = rows[at];
                const title = root.model.nameAt(row);
                notes.push({
                    row: row,
                    index: at,
                    section: sections.length,
                    // Stock follows the note's place on the board, so a row
                    // never repeats a colour while the board is at full width;
                    // tilt comes from the hash, so a destination keeps its
                    // angle when a refresh reorders the sections.
                    tint: placement % root.palette.length,
                    tilt: root.tiltFor(row, title),
                    kind: root.model.kindAt(row),
                    name: title,
                    path: root.model.pathAt(row),
                    detail: root.model.detailAt(row),
                    icon: root.model.iconAt(row),
                    isDir: root.model.isDirAt(row),
                    branch: root.model.branchAt(row),
                    changeText: root.model.changeTextAt(row),
                    timeText: root.model.timeTextAt(row)
                });
                placement += 1;
            }
            sections.push({ name: name, label: root.sectionLabel(name), notes: notes });
        }

        root.sections = sections;
        if (root.cursorRow >= 0 && root.noteForRow(root.cursorRow) === null)
            root.cursorRow = -1;
    }

    function noteForRow(row) {
        for (let index = 0; index < root.sections.length; ++index) {
            const notes = root.sections[index].notes;
            for (let at = 0; at < notes.length; ++at) {
                if (notes[at].row === row)
                    return notes[at];
            }
        }
        return null;
    }

    /// −1.5°…1.5° in half-degree steps, from a hash of the row and its name.
    /// FNV-1a because it is four lines and its only job is to be stable.
    function tiltFor(row, name) {
        const text = row + ":" + name;
        let hash = 2166136261;
        for (let index = 0; index < text.length; ++index) {
            hash ^= text.charCodeAt(index);
            hash = Math.imul(hash, 16777619);
        }
        return (((hash >>> 0) % 7) - 3) * 0.5;
    }

    /// The overview model names destinations by the same keys the sidebar uses,
    /// so the landing page draws its icons from `Glyph` too. The old text
    /// glyphs were a second icon set: the project row's "⌥" was the macOS
    /// Option key, not a branch, and it sat beside a real folder glyph.
    function glyphKind(icon) {
        switch (icon) {
        case "home": return "home"
        case "desktop": return "desktop"
        case "documents": return "documents"
        case "downloads": return "downloads"
        case "pictures": return "pictures"
        case "music": return "music"
        case "videos": return "videos"
        case "star":
        case "favorite": return "star"
        case "space": return "diamond"
        case "project": return "folder"
        case "tag": return "tag"
        case "drive": return "drive"
        case "file": return "file"
        default: return "folder"
        }
    }

    /// The one place a note turns into a destination, so the pointer, a screen
    /// reader's press and the keyboard cursor cannot disagree.
    function openRow(kind, path, isDir) {
        if (kind === "space")
            root.openRequested("space", path);
        else if (kind === "tag")
            root.openRequested("tag", path);
        else if (kind === "recent" && !isDir)
            root.openRequested("file", path);
        else
            root.openRequested("navigate", path);
    }

    function openNote(entry) {
        root.openRow(entry.kind, entry.path, entry.isDir);
    }

    function sectionLabel(value) {
        switch (value) {
        case "Projects": return qsTr("项目")
        case "Favorites": return qsTr("收藏")
        case "Recent": return qsTr("最近")
        case "Spaces": return qsTr("空间")
        case "Tags": return qsTr("标签")
        case "Drives": return qsTr("驱动器")
        default: return value
        }
    }

    function placeLabel(icon) {
        switch (icon) {
        case "home": return qsTr("主目录")
        case "desktop": return qsTr("桌面")
        case "documents": return qsTr("文档")
        case "downloads": return qsTr("下载")
        case "pictures": return qsTr("图片")
        case "music": return qsTr("音乐")
        case "videos": return qsTr("视频")
        default: return ""
        }
    }

    function detailLabel(value) {
        if (value === "Empty space")
            return qsTr("空空间")
        const count = /^(\d+) items?$/.exec(value)
        if (count)
            return qsTr("%1 项").arg(count[1])
        return value
    }

    function changeLabel(value) {
        if (value === "Clean")
            return qsTr("干净")
        if (value === "Untracked")
            return qsTr("未跟踪")
        const count = /^(\d+) changes?$/.exec(value)
        if (count)
            return qsTr("%1 项变更").arg(count[1])
        return value
    }

    function timeLabel(value) {
        if (value === "Today")
            return qsTr("今天")
        if (value === "Yesterday")
            return qsTr("昨天")
        const days = /^(\d+) days ago$/.exec(value)
        if (days)
            return qsTr("%1 天前").arg(days[1])
        return value
    }

    onModelChanged: root.rebuild()
    Component.onCompleted: root.rebuild()

    Connections {
        target: root.model
        function onModelReset() { root.rebuild() }
        function onCountChanged() { root.rebuild() }
        function onLoadedChanged() { root.rebuild() }
    }

    Rectangle {
        anchors.fill: parent
        color: root.contentColor
    }

    // ── Title band ──────────────────────────────────────────────────────────
    // Fixed, not part of the scroll: the search entry is the page's one
    // always-reachable control, and a board that scrolls its own affordance
    // away is a board you have to hunt for.
    Item {
        id: header

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 110

        Item {
            id: headerInner

            x: Math.round((header.width - root.contentWidth) / 2)
            width: root.contentWidth
            height: header.height

            Text {
                id: titleText

                anchors.left: parent.left
                anchors.top: parent.top
                anchors.topMargin: 16
                text: qsTr("概览")
                color: root.textColor
                font.pixelSize: Theme.typeTitle
                font.weight: Font.Medium
            }

            Text {
                id: subtitleText

                anchors.left: parent.left
                anchors.right: busyText.left
                anchors.rightMargin: 12
                anchors.top: titleText.bottom
                anchors.topMargin: 2
                text: qsTr("从收藏、最近访问与空间继续。")
                color: root.mutedColor
                font.pixelSize: Theme.typeCaption
                elide: Text.ElideRight
            }

            Text {
                id: busyText

                anchors.right: parent.right
                anchors.top: titleText.bottom
                anchors.topMargin: 2
                visible: root.model !== null && root.model.projectBusy
                text: qsTr("正在查找最近的项目…")
                color: root.mutedColor
                font.pixelSize: Theme.typeMicro
            }

            Rectangle {
                id: launcher

                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: subtitleText.bottom
                anchors.topMargin: 10
                height: Theme.rowSpacious
                radius: Theme.radiusMd
                color: Theme.canvasAlt
                border.width: 1
                border.color: launcherMouse.containsMouse ? root.accent : root.hairline

                Glyph {
                    anchors.left: parent.left
                    anchors.leftMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    width: 14
                    height: 14
                    kind: "search"
                    tint: root.mutedColor
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 34
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("搜索文件、空间或命令")
                    color: root.mutedColor
                    font.pixelSize: Theme.typeBody
                }

                Rectangle {
                    id: shortcutChip

                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    width: shortcutText.width + 12
                    height: Theme.controlXs
                    radius: Theme.radiusXs
                    color: Theme.hover

                    Text {
                        id: shortcutText
                        anchors.centerIn: parent
                        text: root.commandShortcut
                        color: root.mutedColor
                        font.pixelSize: Theme.typeMicro
                    }
                }

                MouseArea {
                    id: launcherMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("搜索文件、空间或命令")
                    Accessible.description: root.commandShortcut
                    onClicked: root.commandPaletteRequested()
                }
            }
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: root.hairline
        }
    }

    // ── The board ───────────────────────────────────────────────────────────
    Flickable {
        id: board

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: header.bottom
        anchors.bottom: parent.bottom
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        contentWidth: width
        contentHeight: stack.y + stack.height + 24

        /// Startup split: the model marks its publish point in Rust (a QML
        /// handler runs after the view's own model connection, so it would
        /// report layout as data); this is the layout half, and the Rust side
        /// keeps only the first report of each stage. See
        /// `crates/dae-qt/src/startup.rs`.
        onContentHeightChanged: if (root.visible && root.sections.length > 0
                                    && contentHeight > 0)
            Startup.mark("qml-first-list")

        Column {
            id: stack

            x: Math.round((board.width - root.contentWidth) / 2)
            y: 16
            width: root.contentWidth
            spacing: 22

            Repeater {
                model: root.sections

                delegate: Column {
                    id: sectionBlock

                    required property var modelData

                    width: stack.width
                    spacing: 12

                    Item {
                        id: sectionHead

                        width: sectionBlock.width
                        height: 18

                        Text {
                            id: sectionText

                            anchors.left: parent.left
                            anchors.verticalCenter: parent.verticalCenter
                            text: sectionBlock.modelData.label
                            color: Theme.inkFaint
                            font.pixelSize: Theme.typeMicro
                            font.weight: Font.Medium
                            font.letterSpacing: 0.88
                        }

                        Rectangle {
                            anchors.left: sectionText.right
                            anchors.leftMargin: 10
                            anchors.right: sectionCount.left
                            anchors.rightMargin: 10
                            anchors.verticalCenter: parent.verticalCenter
                            height: 1
                            color: Theme.rowRule
                        }

                        Text {
                            id: sectionCount

                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("%1 项").arg(sectionBlock.modelData.notes.length)
                            color: Theme.inkFaint
                            font.pixelSize: Theme.typeMicro
                            font.features: { "tnum": 1 }
                        }
                    }

                    Flow {
                        width: sectionBlock.width
                        spacing: root.noteGap

                        Repeater {
                            model: sectionBlock.modelData.notes

                            delegate: Item {
                                id: note

                                required property var modelData

                                /// The note's own fact set, published by
                                /// `rebuild()`. Not called `data`: `Item.data`
                                /// is the read-only default-property alias for
                                /// children, and a property with that name
                                /// fails to assign at load time.
                                readonly property var entry: note.modelData
                                readonly property string displayName: {
                                    const place = root.placeLabel(note.entry.icon);
                                    return place !== "" ? place : note.entry.name;
                                }
                                readonly property color paper:
                                    root.palette[note.entry.tint % root.palette.length]
                                readonly property bool current: note.activeFocus

                                width: root.noteWidth
                                height: root.noteHeight
                                rotation: note.lifted ? note.entry.tilt * 0.4 : note.entry.tilt
                                scale: note.lifted ? 1.02 : 1.0
                                // A lifted note paints over its neighbours the
                                // way a real one would when you pull it up.
                                z: note.current ? 2 : (noteMouse.containsMouse ? 1 : 0)

                                /// Pointing at a note, or reaching it with the
                                /// keyboard, picks it up: it straightens and
                                /// grows a little instead of changing colour,
                                /// which would read as a state rather than a
                                /// lift.
                                readonly property bool lifted: noteMouse.containsMouse
                                    || note.current

                                Behavior on rotation {
                                    enabled: Theme.motion
                                    NumberAnimation { duration: Theme.motionFast }
                                }
                                Behavior on scale {
                                    enabled: Theme.motion
                                    NumberAnimation { duration: Theme.motionFast }
                                }

                                Accessible.role: Accessible.Button
                                Accessible.name: note.displayName
                                Accessible.description: note.entry.kind === "project"
                                    ? note.entry.branch + " · "
                                      + root.changeLabel(note.entry.changeText)
                                    : root.detailLabel(note.entry.detail)
                                Accessible.onPressAction: root.openNote(note.entry)

                                activeFocusOnTab: note.entry.row === root.firstRow
                                Keys.onLeftPressed: root.moveCursor(note.entry, -1, 0)
                                Keys.onRightPressed: root.moveCursor(note.entry, 1, 0)
                                Keys.onUpPressed: root.moveCursor(note.entry, 0, -1)
                                Keys.onDownPressed: root.moveCursor(note.entry, 0, 1)
                                Keys.onReturnPressed: root.openNote(note.entry)
                                Keys.onSpacePressed: root.openNote(note.entry)
                                Component.onCompleted: root.registerNote(note.entry.row, note)
                                Component.onDestruction: root.unregisterNote(note.entry.row)

                                HardShadow {
                                    target: paperPlate
                                    offset: note.current || noteMouse.containsMouse ? 6 : 4
                                    color: Theme.dark ? "#73000000" : Theme.ink
                                }

                                Rectangle {
                                    id: paperPlate

                                    anchors.fill: parent
                                    y: noteMouse.pressed ? 1 : 0
                                    color: note.paper
                                    border.width: 1
                                    border.color: Theme.hairline
                                }

                                /// The keyboard cursor, drawn outside the paper
                                /// so it never competes with the paper's own
                                /// edge, and only while this note holds focus.
                                Rectangle {
                                    anchors.fill: paperPlate
                                    anchors.margins: -3
                                    visible: note.current
                                    color: "transparent"
                                    border.width: 2
                                    border.color: Theme.signal
                                }

                                // ── Pin ─────────────────────────────────
                                // A thumbtack, not a dot: a warm metal head
                                // over a tight shadow, plus a specular chip at
                                // the top left. The shadow follows the same
                                // down-right direction as `HardShadow`, which
                                // is what makes the head look pushed through
                                // the sheet rather than drawn on it.
                                Item {
                                    id: pin

                                    anchors.horizontalCenter: parent.horizontalCenter
                                    y: -7
                                    width: 16
                                    height: 16

                                    Rectangle {
                                        x: 4
                                        y: 11
                                        width: 11
                                        height: 3
                                        radius: height / 2
                                        color: Qt.rgba(0, 0, 0, Theme.dark ? 0.55 : 0.30)
                                    }

                                    Rectangle {
                                        anchors.fill: parent
                                        radius: width / 2
                                        border.width: 1
                                        border.color: Theme.dark ? "#7a6c58" : "#5d4d38"
                                        gradient: Gradient {
                                            GradientStop {
                                                position: 0.0
                                                color: Theme.dark ? "#f6eedd" : "#fffdf7"
                                            }
                                            GradientStop {
                                                position: 0.48
                                                color: Theme.dark ? "#d3c2a6" : "#e6d9c2"
                                            }
                                            GradientStop {
                                                position: 1.0
                                                color: Theme.dark ? "#83735c" : "#9b8b72"
                                            }
                                        }
                                    }

                                    /// The dark bottom of the head, so the
                                    /// gradient reads as a sphere and not as
                                    /// a flat disc with a gradient on it.
                                    Rectangle {
                                        anchors.fill: parent
                                        anchors.margins: 2
                                        radius: width / 2
                                        color: "transparent"
                                        border.width: 2
                                        border.color: Qt.rgba(0, 0, 0,
                                                              Theme.dark ? 0.18 : 0.10)
                                    }

                                    Rectangle {
                                        x: 4
                                        y: 3
                                        width: 5
                                        height: 3
                                        radius: 1.5
                                        color: "#ffffff"
                                        opacity: 0.9
                                    }
                                }

                                // ── Note face ───────────────────────────
                                Glyph {
                                    anchors.left: parent.left
                                    anchors.leftMargin: 14
                                    anchors.top: parent.top
                                    anchors.topMargin: 20
                                    width: 14
                                    height: 14
                                    kind: root.glyphKind(note.entry.icon)
                                    tint: Theme.inkMuted
                                }

                                Text {
                                    id: noteName

                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.leftMargin: 14
                                    anchors.rightMargin: 14
                                    anchors.top: parent.top
                                    anchors.topMargin: 40
                                    height: 38
                                    text: note.displayName
                                    color: Theme.ink
                                    font.pixelSize: Theme.typeName
                                    font.weight: Font.Medium
                                    wrapMode: Text.Wrap
                                    maximumLineCount: 2
                                    elide: Text.ElideRight
                                    textFormat: Text.PlainText
                                    lineHeight: Theme.leadingTight
                                }

                                /// One reading line at the foot of the note —
                                /// the fact you would otherwise have to open it
                                /// to learn. A project leads with its branch,
                                /// everything else with its count or parent.
                                Item {
                                    id: noteFoot

                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.leftMargin: 14
                                    anchors.rightMargin: 14
                                    anchors.bottom: parent.bottom
                                    anchors.bottomMargin: 11
                                    height: 16

                                    Rectangle {
                                        id: branchChip

                                        anchors.left: parent.left
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: note.entry.kind === "project" && note.entry.branch !== ""
                                        height: 16
                                        width: branchLabel.implicitWidth + 10
                                        color: Qt.rgba(Theme.ink.r, Theme.ink.g,
                                                       Theme.ink.b, 0.10)

                                        Text {
                                            id: branchLabel
                                            anchors.centerIn: parent
                                            text: note.entry.branch
                                            color: Theme.ink
                                            font.pixelSize: Theme.typeMicro
                                            font.family: Theme.monoFamily
                                        }
                                    }

                                    Text {
                                        id: footLeft

                                        anchors.left: branchChip.visible
                                            ? branchChip.right : parent.left
                                        anchors.leftMargin: branchChip.visible ? 6 : 0
                                        anchors.right: footRight.left
                                        anchors.rightMargin: 8
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: note.entry.kind === "project"
                                            ? note.entry.changeText !== ""
                                            : note.entry.detail !== ""
                                        text: note.entry.kind === "project"
                                            ? root.changeLabel(note.entry.changeText)
                                            : root.detailLabel(note.entry.detail)
                                        color: note.entry.kind === "project"
                                            && note.entry.changeText !== "Clean"
                                            && note.entry.changeText !== "Untracked"
                                            ? Theme.warn
                                            : (note.entry.changeText === "Untracked"
                                               ? Theme.link : Theme.inkMuted)
                                        font.pixelSize: Theme.typeCaption
                                        elide: Text.ElideMiddle
                                        textFormat: Text.PlainText
                                    }

                                    Text {
                                        id: footRight

                                        anchors.right: parent.right
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: note.entry.timeText !== ""
                                        width: visible ? implicitWidth : 0
                                        text: root.timeLabel(note.entry.timeText)
                                        color: Theme.inkMuted
                                        font.pixelSize: Theme.typeCaption
                                        font.features: { "tnum": 1 }
                                    }
                                }

                                MouseArea {
                                    id: noteMouse
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    Accessible.role: Accessible.Button
                                    Accessible.name: note.displayName
                                    onClicked: {
                                        root.cursorRow = note.entry.row;
                                        root.openNote(note.entry);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// With every section hidden and nothing favourited yet, the board is an
    /// empty page. This is the landing page's own empty state, using the shared
    /// grammar rather than a second mute sentence.
    EmptyState {
        anchors.fill: board
        visible: root.model !== null && root.model.loaded
            && root.model.count === 0
        glyph: "folder"
        title: qsTr("还没有可显示的位置")
        hint: qsTr("在地址栏打开一个文件夹，它会出现在「最近」里。")
    }
}
