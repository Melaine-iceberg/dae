pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import dae 1.0

// Entry filters: which rows exist, as opposed to how they are arranged.
//
// The menu carries the hidden-files switch alongside the three buckets because
// all four narrow the same listing, and keeps a clear-all action at the bottom
// so an accidental combination is one click away from undo. The panel is
// `Menu`, shared with every other menu in the shell; the clear row is its
// footer, pinned so it cannot scroll away.
Menu {
    id: root

    property color accent: Theme.accent
    /// The binding from `settings.toml`; the view only prints it.
    property string hiddenShortcut: "Ctrl+H"

    readonly property bool active: ExplorerPrefs.kindFilter !== "all"
        || ExplorerPrefs.modifiedFilter !== "any"
        || ExplorerPrefs.sizeFilter !== "any"
        || ExplorerPrefs.nameFilter !== ""
        || !ExplorerPrefs.showHiddenFiles

    readonly property var items: [
        { kind: "label", label: qsTr("可见性") },
        { kind: "toggle", property: "showHiddenFiles", glyph: "eye",
          label: qsTr("显示隐藏文件"),
          shortcut: root.hiddenShortcut },
        { kind: "sep" },
        { kind: "label", label: qsTr("类型") },
        { kind: "radio", group: "kindFilter", value: "all", label: qsTr("全部") },
        { kind: "radio", group: "kindFilter", value: "folders", glyph: "folder",
          label: qsTr("仅文件夹") },
        { kind: "radio", group: "kindFilter", value: "files", glyph: "file",
          label: qsTr("仅文件") },
        { kind: "radio", group: "kindFilter", value: "images", glyph: "image",
          label: qsTr("仅图片") },
        { kind: "sep" },
        { kind: "label", label: qsTr("修改时间") },
        { kind: "radio", group: "modifiedFilter", value: "any", label: qsTr("不限") },
        { kind: "radio", group: "modifiedFilter", value: "today", label: qsTr("今天") },
        { kind: "radio", group: "modifiedFilter", value: "week", label: qsTr("最近 7 天") },
        { kind: "radio", group: "modifiedFilter", value: "month", label: qsTr("最近 30 天") },
        { kind: "sep" },
        { kind: "label", label: qsTr("大小") },
        { kind: "radio", group: "sizeFilter", value: "any", label: qsTr("不限") },
        { kind: "radio", group: "sizeFilter", value: "small", label: qsTr("小（< 1 MB）") },
        { kind: "radio", group: "sizeFilter", value: "medium", label: qsTr("中（1 – 100 MB）") },
        { kind: "radio", group: "sizeFilter", value: "large", label: qsTr("大（> 100 MB）") }
    ]

    rows: root.normalized()

    function openAbove(x, y) {
        root.openX = x
        root.openY = y
        root.above = true
        root.show()
    }

    function close() {
        root.visible = false
    }

    function checked(item) {
        if (item.kind === "toggle")
            return ExplorerPrefs[item.property] === true
        return ExplorerPrefs[item.group] === item.value
    }

    function choose(item) {
        if (item === undefined)
            return
        if (item.kind === "toggle") {
            ExplorerPrefs.setBool(item.property, !ExplorerPrefs[item.property])
            return
        }
        if (item.kind === "radio")
            ExplorerPrefs.setString(item.group, item.value)
    }

    function normalized() {
        const mapped = []
        for (let index = 0; index < root.items.length; index += 1) {
            const entry = root.items[index]
            const kind = entry.kind === "label" ? "header"
                : (entry.kind === "sep" ? "separator" : "item")
            mapped.push({
                kind: kind,
                label: entry.label,
                glyph: entry.glyph || "",
                checked: kind === "item" && root.checked(entry),
                checkable: kind === "item" && (entry.kind === "toggle"
                    || entry.kind === "radio"),
                shortcut: entry.shortcut || ""
            })
        }
        return mapped
    }

    onRowTriggered: function (index) { root.choose(root.items[index]) }

    /// The way back from a filter combination that found nothing. It is a
    /// footer rather than the last row so a menu scrolled to the top of a
    /// short window still offers it.

    /// The one filter that is typed rather than chosen. It sits above the
    /// rows the way the clear row sits below them: same listing, same
    /// vocabulary, and the box is what answers free text.
    header: Component {
        Item {
            id: band
            implicitHeight: 40

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: Theme.ruleWidth(Screen.devicePixelRatio)
                color: Theme.hairline
            }

            TextInput {
                id: nameInput
                anchors.left: parent.left
                anchors.leftMargin: 12
                anchors.right: clearBox.visible ? clearBox.left : parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                color: Theme.ink
                selectionColor: Theme.accent
                selectedTextColor: Theme.onAccent
                font.pixelSize: Theme.typeBody
                selectByMouse: true
                clip: true

                Accessible.role: Accessible.EditableText
                Accessible.name: qsTr("按名称筛选")

                onTextChanged: nameDebounce.restart()
                onAccepted: {
                    ExplorerPrefs.setString("nameFilter", nameInput.text)
                    root.close()
                }
                // A one-line box has no use for Down: hand the rows back the
                // keyboard so the menu stays reachable without a pointer.
                Keys.onDownPressed: root.focusRows()
                Keys.onEscapePressed: root.close()

                Text {
                    visible: nameInput.text === ""
                    text: qsTr("按名称筛选")
                    color: Theme.inkTertiary
                    font.pixelSize: Theme.typeBody
                }
            }

            Item {
                id: clearBox
                visible: nameInput.text !== ""
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                width: 16
                height: 16

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("清除名称筛选")

                Text {
                    anchors.centerIn: parent
                    text: "×"
                    color: clearMouse.containsMouse ? Theme.ink : Theme.inkTertiary
                    font.pixelSize: Theme.typeBody
                }

                MouseArea {
                    id: clearMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        nameInput.text = ""
                        ExplorerPrefs.setString("nameFilter", "")
                        nameInput.forceActiveFocus()
                    }
                }
            }

            /// Typing rebuilds the listing on every change; the pause keeps
            /// one word from being four reloads.
            Timer {
                id: nameDebounce
                interval: 300
                onTriggered: ExplorerPrefs.setString("nameFilter", nameInput.text)
            }

            Connections {
                target: root
                function onVisibleChanged() {
                    if (!root.visible)
                        return
                    nameInput.text = ExplorerPrefs.nameFilter
                    // `Menu.show()` queues `list.takeFocus` first; a second
                    // hop lands after it, so the box wins the keyboard.
                    Qt.callLater(function () {
                        Qt.callLater(function () { nameInput.forceActiveFocus() })
                    })
                }
                /// The clear row below also clears the box; the box follows
                /// unless the user is the one typing into it.
                function onNameFilterChanged() {
                    if (!nameInput.activeFocus)
                        nameInput.text = ExplorerPrefs.nameFilter
                }
            }
        }
    }

    footer: Component {
        Item {
            implicitHeight: root.active ? 29 : 0

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                height: Theme.ruleWidth(Screen.devicePixelRatio)
                color: Theme.hairline
            }

            Text {
                anchors.centerIn: parent
                text: qsTr("清除筛选")
                color: root.accent
                font.pixelSize: Theme.typeMicro
            }

            MouseArea {
                Accessible.role: Accessible.Button
                Accessible.name: qsTr("清除筛选")
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: ExplorerPrefs.resetFilters()
            }
        }
    }
}
