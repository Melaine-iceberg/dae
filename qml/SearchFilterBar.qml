import QtQuick

// The search field's filter row: which mode is searching, how strictly, and
// which kinds of file may answer.
//
// Before this file the shell carried two different bars. Content search had
// one — a regex toggle, a case toggle and a free-text glob box — and name
// search had none, so the cheaper and far more common search could not say how
// many rows it had found or narrow them at all. The row below is the one bar
// both modes share: the mode chips say who is answering, and the controls to
// their right only exist while a content query is running, because that is the
// only backend that accepts a case flag and a file filter. A toggle that
// silently does nothing is worse than no toggle.
//
// The type menu itself is not built here. The row knows the categories and
// their glob lists, but a popup has to be anchored in window coordinates, so
// the shell opens its own menu when the chip asks for one.
Item {
    id: root

    /// "name" or "content".
    property string mode: "name"
    /// Content search is a local ripgrep walk. Name search is local-only too,
    /// so the shell hides this row on network paths; keeping the content chip
    /// inert here is a second guard rather than the only one.
    property bool contentEnabled: true
    property bool caseSensitive: false
    property bool regex: false
    /// One of `typeOptions`' ids.
    property string typeId: "all"
    property bool busy: false
    /// Rows (name mode) or files (content mode) the running query matched.
    property int matchCount: 0
    property string query: ""

    signal modeRequested(string mode)
    signal caseToggled()
    signal regexToggled()
    /// The chip's own top-left corner, in window coordinates.
    signal typeMenuRequested(real x, real y)

    /// The categories a content query may be narrowed to. Each glob list is
    /// what the backend's file filter already accepts (`"*.ts, .rs"`), so this
    /// introduces no new filter dimension — only a shorter way to type one.
    readonly property var typeOptions: [
        { id: "all", label: qsTr("全部类型"), glob: "" },
        { id: "doc", label: qsTr("文档"),
          glob: "*.txt, *.md, *.pdf, *.doc, *.docx, *.rtf, *.csv" },
        { id: "image", label: qsTr("图片"),
          glob: "*.png, *.jpg, *.jpeg, *.gif, *.webp, *.svg, *.bmp, *.heic, *.avif" },
        { id: "code", label: qsTr("代码"),
          glob: "*.rs, *.ts, *.tsx, *.js, *.jsx, *.py, *.go, *.c, *.h, *.cc, *.cpp, *.hpp, *.java, *.kt, *.rb, *.php, *.swift, *.sh, *.lua, *.json, *.toml, *.yaml, *.yml, *.xml, *.html, *.css, *.scss, *.sql, *.qml" },
        { id: "archive", label: qsTr("压缩包"),
          glob: "*.zip, *.tar, *.gz, *.tgz, *.bz2, *.xz, *.7z, *.rar" },
        { id: "media", label: qsTr("媒体"),
          glob: "*.mp3, *.flac, *.wav, *.m4a, *.ogg, *.mp4, *.mkv, *.mov, *.avi, *.webm" }
    ]

    /// True while the content-only controls have something to act on.
    readonly property bool contentMode: root.mode === "content"

    function optionAt(id) {
        for (let index = 0; index < root.typeOptions.length; index += 1) {
            if (root.typeOptions[index].id === id)
                return root.typeOptions[index]
        }
        return root.typeOptions[0]
    }

    function typeLabel(id) {
        return root.optionAt(id).label
    }

    function typeGlob(id) {
        return root.optionAt(id).glob
    }

    implicitWidth: row.implicitWidth + 140
    implicitHeight: 32

    Rectangle {
        anchors.fill: parent
        color: Theme.canvas
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 1
        color: Theme.hairline
    }

    /// One chip: 22 high, 11 px, corner 2. A chip is a switch, not a button —
    /// its pressed state is the fill the shell uses everywhere for "this is the
    /// current one", never the signal colour.
    component Chip: Rectangle {
        id: chip

        property string label: ""
        property bool active: false
        property bool available: true
        /// A fixed square chip for marks like ".*" and "Aa".
        property bool square: false

        signal clicked()

        height: 22
        width: chip.square ? 28 : Math.ceil(chipLabel.implicitWidth) + 14
        radius: Theme.radiusXs
        color: chip.active ? Theme.fill : "transparent"
        border.width: 1
        border.color: chip.active ? Theme.hairlineStrong : Theme.hairline
        opacity: chip.available ? 1 : 0.45

        Accessible.role: Accessible.Button
        Accessible.name: chip.label

        Text {
            id: chipLabel
            anchors.centerIn: parent
            text: chip.label
            color: chip.active ? Theme.ink : Theme.inkMuted
            font.pixelSize: Theme.typeMicro
        }

        MouseArea {
            anchors.fill: parent
            enabled: chip.available
            hoverEnabled: true
            cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
            onClicked: chip.clicked()
        }
    }

    Row {
        id: row

        anchors.left: parent.left
        anchors.leftMargin: 12
        anchors.verticalCenter: parent.verticalCenter
        height: 22
        spacing: 6

        Chip {
            label: qsTr("文件名")
            active: root.mode === "name"
            onClicked: root.modeRequested("name")
        }

        Chip {
            label: qsTr("内容")
            active: root.contentMode
            available: root.contentEnabled
            onClicked: root.modeRequested("content")
        }

        Chip {
            square: true
            visible: root.contentMode
            label: ".*"
            active: root.regex
            onClicked: root.regexToggled()
        }

        Chip {
            square: true
            visible: root.contentMode
            label: "Aa"
            active: root.caseSensitive
            onClicked: root.caseToggled()
        }

        Rectangle {
            id: typeChip

            visible: root.contentMode
            width: visible ? typeRow.implicitWidth + 28 : 0
            height: 22
            radius: Theme.radiusXs
            color: typeMouse.containsMouse ? Theme.hover : "transparent"
            border.width: 1
            border.color: Theme.hairline

            Accessible.role: Accessible.Button
            Accessible.name: qsTr("按类型筛选：%1").arg(root.typeLabel(root.typeId))

            Row {
                id: typeRow
                anchors.left: parent.left
                anchors.leftMargin: 7
                anchors.verticalCenter: parent.verticalCenter
                spacing: 5

                Glyph {
                    width: 14
                    height: 14
                    kind: "file"
                    tint: Theme.inkMuted
                }

                Text {
                    text: root.typeLabel(root.typeId)
                    color: Theme.ink
                    font.pixelSize: Theme.typeMicro
                }
            }

            Glyph {
                anchors.right: parent.right
                anchors.rightMargin: 5
                anchors.verticalCenter: parent.verticalCenter
                width: 10
                height: 10
                kind: "chevron"
                rotation: 90
                tint: Theme.inkFaint
            }

            MouseArea {
                id: typeMouse
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                    const point = typeChip.mapToItem(null, 0, 0)
                    root.typeMenuRequested(point.x, point.y + typeChip.height + 4)
                }
            }
        }
    }

    Text {
        id: countText

        anchors.right: parent.right
        anchors.rightMargin: 12
        anchors.verticalCenter: parent.verticalCenter
        text: {
            if (root.busy)
                return qsTr("搜索中…")
            if (root.query.length === 0 && root.typeId === "all")
                return root.contentMode ? qsTr("输入以搜索文件内容")
                                       : qsTr("输入以筛选名称")
            return qsTr("%1 项匹配").arg(root.matchCount)
        }
        color: Theme.inkFaint
        font.pixelSize: Theme.typeMicro
        Accessible.role: Accessible.StaticText
        Accessible.name: text
    }
}
