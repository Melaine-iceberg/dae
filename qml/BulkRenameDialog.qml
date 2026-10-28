pragma ComponentBehavior: Bound
import QtQuick
import dae 1.0

// Renames a whole selection from one rule: a numbered sequence, a find and
// replace, or a case change.
//
// The plan is computed here, in QML, because it is a pure preview: nothing is
// written until "重命名" is pressed, and the backend re-validates every name
// anyway. The one rule the backend owns that this cannot know is a collision
// with an entry outside the batch, which comes back as a toast.
FocusScope {
    id: root

    property bool dark: true
    property color accent: Theme.interactive

    /// `[{ path, name }]` — the selection the menu was opened on.
    property var entries: []
    /// `sequence`, `replace` or `case`.
    property string mode: "sequence"

    property string baseName: ""
    property int start: 1
    property int step: 1
    property int padding: 2

    property string find: ""
    property string replacement: ""
    property bool caseSensitive: false
    property bool useRegex: false
    /// `lower`, `upper`, `title` or `sentence`. Named `caseTransform` because
    /// `transform` is a FINAL member of `Item` and cannot be overridden.
    property string caseTransform: "lower"

    property string globalError: ""
    property var planItems: []
    property int renameCount: 0
    property int errorCount: 0

    readonly property color textColor: Theme.ink
    readonly property color mutedColor: Theme.inkMuted
    readonly property color fieldColor: Theme.field
    readonly property color hairline: Theme.hairline
    readonly property color dangerColor: Theme.danger
    readonly property bool canApply: root.renameCount > 0 && root.errorCount === 0
                                      && root.globalError === ""

    signal closed()

    visible: false
    z: 1006
    focus: visible

    function open(paths) {
        const next = []
        for (let i = 0; i < paths.length; i += 1)
            next.push({ path: paths[i], name: baseNameOf(paths[i]) })
        root.entries = next
        root.mode = "sequence"
        root.baseName = ""
        root.start = 1
        root.step = 1
        root.padding = 2
        root.find = ""
        root.replacement = ""
        root.caseSensitive = false
        root.useRegex = false
        root.caseTransform = "lower"
        // The fields keep their text between openings; a new selection starts
        // from the defaults, not from the previous entry's names.
        baseField.reset("")
        findField.reset("")
        replacementField.reset("")
        startField.reset(1)
        stepField.reset(1)
        paddingField.reset(2)
        root.recompute()
        root.visible = true
        root.forceActiveFocus()
        Qt.callLater(function () {
            if (root.visible)
                baseField.focusInput()
        })
    }

    function close() {
        root.visible = false
        root.closed()
    }

    function apply() {
        if (!root.canApply)
            return
        const paths = []
        const names = []
        for (let i = 0; i < root.planItems.length; i += 1) {
            const item = root.planItems[i]
            if (item.status !== "rename")
                continue
            paths.push(item.path)
            names.push(item.newName)
        }
        if (paths.length > 0)
            Workspace.renameEntriesBatch(paths, names)
        root.close()
    }

    // ---- Planning ---------------------------------------------------------

    function baseNameOf(path) {
        const parts = path.split(/[\\/]/)
        return parts.length > 0 ? parts[parts.length - 1] : path
    }

    function splitName(name) {
        const dot = name.lastIndexOf(".")
        if (dot <= 0)
            return { base: name, ext: "" }
        return { base: name.slice(0, dot), ext: name.slice(dot) }
    }

    function escapeRegExp(text) {
        return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
    }

    function padNumber(value, width) {
        const sign = value < 0 ? "-" : ""
        let digits = String(Math.abs(value))
        while (digits.length < Math.max(0, width))
            digits = "0" + digits
        return sign + digits
    }

    function transformCase(text, how) {
        if (how === "lower")
            return text.toLocaleLowerCase()
        if (how === "upper")
            return text.toLocaleUpperCase()
        if (how === "title")
            return text.toLocaleLowerCase().replace(/(^|\s)(\S)/g,
                function (all, lead, letter) { return lead + letter.toLocaleUpperCase() })
        // sentence
        return text.replace(/^(\s*)(\S)/, function (all, lead, letter) {
            return lead + letter.toLocaleUpperCase()
        })
    }

    /// Windows' rules, applied on every platform so the preview never promises
    /// a name the backend will refuse.
    function isInvalidName(name) {
        if (/[<>:"/\\|?*]/.test(name))
            return true
        for (let i = 0; i < name.length; i += 1) {
            const code = name.charCodeAt(i)
            if (code < 0x20 || code === 0x7f)
                return true
        }
        return /[ .]$/.test(name)
    }

    function newNameFor(entry, index) {
        const parts = root.splitName(entry.name)
        if (root.mode === "sequence") {
            const number = root.start + index * root.step
            return root.baseName + root.padNumber(number, root.padding) + parts.ext
        }
        if (root.mode === "replace") {
            if (root.find === "")
                return entry.name
            let pattern = null
            try {
                pattern = new RegExp(root.useRegex ? root.find : root.escapeRegExp(root.find),
                                     root.caseSensitive ? "g" : "gi")
            } catch (error) {
                return null
            }
            return parts.base.replace(pattern, root.replacement) + parts.ext
        }
        return root.transformCase(parts.base, root.caseTransform) + parts.ext
    }

    function recompute() {
        const items = []
        let renameCount = 0
        let errorCount = 0
        let globalError = ""

        const seen = ({})
        for (let i = 0; i < root.entries.length; i += 1) {
            const entry = root.entries[i]
            const newName = root.newNameFor(entry, i)
            let status = "rename"
            let errorKey = ""

            if (newName === null) {
                globalError = qsTr("正则表达式无效")
                status = "error"
                errorKey = "invalid"
            } else if (newName === "") {
                status = "error"
                errorKey = "empty"
            } else if (root.isInvalidName(newName)) {
                status = "error"
                errorKey = "invalid"
            } else {
                const key = newName.toLocaleLowerCase()
                if (seen[key] === true) {
                    status = "error"
                    errorKey = "duplicate"
                } else {
                    seen[key] = true
                }
            }

            if (status === "error")
                errorCount += 1
            else if (newName === entry.name)
                status = "unchanged"
            else
                renameCount += 1

            items.push({
                path: entry.path,
                name: entry.name,
                newName: newName === null ? entry.name : newName,
                status: status,
                errorKey: errorKey
            })
        }

        root.planItems = items
        root.renameCount = renameCount
        root.errorCount = errorCount
        root.globalError = globalError
    }

    function errorTextFor(item) {
        if (item.errorKey === "empty")
            return qsTr("名称不能为空")
        if (item.errorKey === "invalid")
            return qsTr("名称包含无效字符")
        if (item.errorKey === "duplicate")
            return qsTr("与本次的另一项重名")
        return ""
    }

    Keys.onEscapePressed: root.close()

    Rectangle {
        anchors.fill: parent
        color: Theme.scrim

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: root.close()
        }
    }

    Rectangle {
        anchors.centerIn: parent
        width: Math.min(620, root.width - 40)
        height: Math.min(600, root.height - 48)
        radius: 16
        color: Theme.raised
        border.width: 1
        border.color: Theme.hairlineStrong

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onWheel: function (wheel) { wheel.accepted = true }
        }

        Column {
            id: layout
            anchors.fill: parent
            anchors.margins: 18
            spacing: 0

            Text {
                width: parent.width
                text: qsTr("批量重命名")
                color: root.textColor
                font.pixelSize: 14
                font.weight: Font.DemiBold
            }

            Item { width: 1; height: 5 }

            Text {
                width: parent.width
                text: qsTr("为选中的 ") + root.entries.length + qsTr(" 项生成新名称。")
                color: root.mutedColor
                font.pixelSize: 12
            }

            Item { width: 1; height: 12 }

            Row {
                id: modeTabs
                width: parent.width
                height: 30
                spacing: 18

                Repeater {
                    model: [
                        { key: "sequence", label: qsTr("按序号") },
                        { key: "replace", label: qsTr("替换") },
                        { key: "case", label: qsTr("大小写") }
                    ]

                    delegate: Item {
                        id: modeItem
                        required property var modelData
                        width: modeLabel.width
                        height: modeTabs.height

                        Text {
                            id: modeLabel
                            anchors.centerIn: parent
                            text: modeItem.modelData.label
                            color: root.mode === modeItem.modelData.key
                                ? root.textColor : root.mutedColor
                            font.pixelSize: 12
                            font.weight: root.mode === modeItem.modelData.key
                                ? Font.DemiBold : Font.Normal
                        }

                        Rectangle {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            height: 2
                            radius: 4
                            color: root.accent
                            visible: root.mode === modeItem.modelData.key
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                root.mode = modeItem.modelData.key
                                root.recompute()
                            }
                        }
                    }
                }
            }

            Rectangle {
                width: parent.width
                height: 1
                color: root.hairline
            }

            Item { width: 1; height: 12 }

            // ---- Mode-specific inputs ---------------------------------------
            Item {
                width: parent.width
                height: root.mode === "replace" ? 84 : 34

                Row {
                    id: sequenceFields
                    width: parent.width
                    height: 30
                    spacing: 10
                    visible: root.mode === "sequence"

                    Field {
                        id: baseField
                        width: 190
                        placeholder: qsTr("基础名称")
                        onEdited: function (value) {
                            root.baseName = value
                            root.recompute()
                        }
                    }

                    NumberField {
                        id: startField
                        width: 64
                        label: qsTr("起始")
                        value: root.start
                        onEdited: function (value) {
                            root.start = value
                            root.recompute()
                        }
                    }

                    NumberField {
                        id: stepField
                        width: 64
                        label: qsTr("步长")
                        value: Math.max(1, root.step)
                        onEdited: function (value) {
                            root.step = Math.max(1, value)
                            root.recompute()
                        }
                    }

                    NumberField {
                        id: paddingField
                        width: 74
                        label: qsTr("位数")
                        value: root.padding
                        onEdited: function (value) {
                            root.padding = Math.max(0, Math.min(8, value))
                            root.recompute()
                        }
                    }
                }

                Column {
                    width: parent.width
                    spacing: 8
                    visible: root.mode === "replace"

                    Row {
                        width: parent.width
                        height: 30
                        spacing: 10

                        Field {
                            id: findField
                            width: 190
                            placeholder: qsTr("查找")
                            onEdited: function (value) {
                                root.find = value
                                root.recompute()
                            }
                        }

                        Field {
                            id: replacementField
                            width: 190
                            placeholder: qsTr("替换为")
                            onEdited: function (value) {
                                root.replacement = value
                                root.recompute()
                            }
                        }
                    }

                    Row {
                        width: parent.width
                        height: 22
                        spacing: 18

                        Toggle {
                            label: qsTr("区分大小写")
                            checked: root.caseSensitive
                            onToggled: {
                                root.caseSensitive = !root.caseSensitive
                                root.recompute()
                            }
                        }

                        Toggle {
                            label: qsTr("正则表达式")
                            checked: root.useRegex
                            onToggled: {
                                root.useRegex = !root.useRegex
                                root.recompute()
                            }
                        }
                    }
                }

                Row {
                    width: parent.width
                    height: 30
                    spacing: 8
                    visible: root.mode === "case"

                    Repeater {
                        model: [
                            { key: "lower", label: qsTr("全部小写") },
                            { key: "upper", label: qsTr("全部大写") },
                            { key: "title", label: qsTr("每词首字母大写") },
                            { key: "sentence", label: qsTr("句首大写") }
                        ]

                        delegate: Rectangle {
                            id: caseButton
                            required property var modelData
                            width: caseLabel.width + 20
                            height: 26
                            radius: 6
                            color: root.caseTransform === caseButton.modelData.key
                                ? Theme.selection
                                : (caseMouse.containsMouse
                                    ? Theme.hover
                                    : "transparent")
                            border.width: 1
                            border.color: root.caseTransform === caseButton.modelData.key
                                ? root.accent : root.hairline

                            Text {
                                id: caseLabel
                                anchors.centerIn: parent
                                text: caseButton.modelData.label
                                color: root.textColor
                                font.pixelSize: 11
                            }

                            MouseArea {
                                id: caseMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                onClicked: {
                                    root.caseTransform = caseButton.modelData.key
                                    root.recompute()
                                }
                            }
                        }
                    }
                }
            }

            Item { width: 1; height: 8 }

            // ---- Preview ----------------------------------------------------
            Text {
                width: parent.width
                height: 16
                text: root.globalError !== ""
                    ? root.globalError
                    : (root.errorCount > 0
                        ? qsTr("%1 项将被重命名，%2 项有问题")
                            .arg(root.renameCount).arg(root.errorCount)
                        : qsTr("%1 项将被重命名").arg(root.renameCount))
                color: root.globalError !== "" || root.errorCount > 0
                    ? root.dangerColor : root.mutedColor
                font.pixelSize: 11
                elide: Text.ElideRight
            }

            Item { width: 1; height: 6 }

            Rectangle {
                width: parent.width
                height: Math.max(120, layout.height - y - 46)
                radius: 6
                color: root.fieldColor
                border.width: 1
                border.color: root.hairline
                clip: true

                ListView {
                    id: previewList
                    anchors.fill: parent
                    anchors.margins: 4
                    model: root.planItems
                    clip: true
                    spacing: 1
                    boundsBehavior: Flickable.StopAtBounds

                    delegate: Item {
                        id: planRow
                        required property var modelData

                        width: previewList.width
                        height: 24

                        Text {
                            width: parent.width * 0.44
                            anchors.verticalCenter: parent.verticalCenter
                            text: planRow.modelData.name
                            color: root.mutedColor
                            font.pixelSize: 11
                            elide: Text.ElideMiddle
                        }

                        Text {
                            x: parent.width * 0.44
                            width: 16
                            anchors.verticalCenter: parent.verticalCenter
                            text: "\u2192"
                            color: root.mutedColor
                            font.pixelSize: 11
                            horizontalAlignment: Text.AlignHCenter
                        }

                        Text {
                            x: parent.width * 0.44 + 20
                            width: parent.width * 0.56 - 20
                            anchors.verticalCenter: parent.verticalCenter
                            text: planRow.modelData.status === "unchanged"
                                ? planRow.modelData.name + qsTr("（不变）")
                                : (planRow.modelData.status === "error"
                                    ? root.errorTextFor(planRow.modelData)
                                    : planRow.modelData.newName)
                            color: planRow.modelData.status === "error"
                                ? root.dangerColor
                                : (planRow.modelData.status === "unchanged"
                                    ? root.mutedColor : root.textColor)
                            font.pixelSize: 11
                            elide: Text.ElideMiddle
                        }
                    }
                }
            }

            Item { width: 1; height: 10 }

            Row {
                width: parent.width
                height: 30
                spacing: 8

                Item {
                    width: Math.max(0, parent.width - 76 - 96 - 16)
                    height: 1
                }

                Rectangle {
                    width: 76
                    height: 28
                    anchors.verticalCenter: parent.verticalCenter
                    radius: 6
                    color: cancelMouse.containsMouse
                        ? Theme.fill : "transparent"
                    border.width: 1
                    border.color: root.hairline

                    Text {
                        anchors.centerIn: parent
                        text: qsTr("取消")
                        color: Theme.inkMuted
                        font.pixelSize: 12
                    }

                    MouseArea {
                        id: cancelMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.close()
                    }
                }

                Rectangle {
                    width: 96
                    height: 28
                    anchors.verticalCenter: parent.verticalCenter
                    radius: 6
                    color: !root.canApply ? Theme.canvasAlt
                        : (applyMouse.containsMouse ? Theme.interactiveHover : Theme.interactive)

                    Text {
                        anchors.centerIn: parent
                        text: qsTr("重命名")
                        color: root.canApply ? Theme.onInteractive : root.mutedColor
                        font.pixelSize: 12
                    }

                    MouseArea {
                        id: applyMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: root.canApply
                        onClicked: root.apply()
                    }
                }
            }
        }
    }

    // ---- Inline components ------------------------------------------------

    component Field: Rectangle {
        id: field

        property string placeholder: ""

        signal edited(string value)

        /// Replaces the live text. User typing breaks a `text:` binding, so a
        /// reopened dialog goes through this instead.
        function reset(value) {
            fieldInput.text = value
        }

        height: 28
        radius: 6
        color: root.fieldColor
        border.width: 1
        border.color: fieldInput.activeFocus
            ? Theme.focusRing
            : root.hairline

        function focusInput() {
            fieldInput.forceActiveFocus()
        }

        TextInput {
            id: fieldInput
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            verticalAlignment: TextInput.AlignVCenter
            color: root.textColor
            selectionColor: root.accent
            selectedTextColor: Theme.onInteractive
            font.pixelSize: 12
            selectByMouse: true
            clip: true

            onTextEdited: field.edited(text)
        }

        Text {
            anchors.left: parent.left
            anchors.leftMargin: 9
            anchors.verticalCenter: parent.verticalCenter
            visible: fieldInput.text.length === 0
            text: field.placeholder
            color: root.mutedColor
            font.pixelSize: 12
        }
    }

    /// A labelled number input. Kept as text so an empty box does not read as
    /// zero while the user is still typing.
    component NumberField: Item {
        id: number

        property string label: ""
        property int value: 0

        signal edited(int value)

        function reset(value) {
            numberInput.text = String(value)
        }

        height: 30

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 24
            radius: 6
            color: root.fieldColor
            border.width: 1
            border.color: numberInput.activeFocus
                ? Theme.focusRing
                : root.hairline

            TextInput {
                id: numberInput
                anchors.fill: parent
                anchors.leftMargin: 7
                anchors.rightMargin: 7
                verticalAlignment: TextInput.AlignVCenter
                color: root.textColor
                selectionColor: root.accent
                selectedTextColor: Theme.onInteractive
                font.pixelSize: 12
                selectByMouse: true
                clip: true
                text: String(number.value)
                validator: IntValidator { bottom: 0; top: 99999 }

                onTextEdited: number.edited(parseInt(text, 10) || 0)
            }
        }

        Text {
            x: 2
            anchors.bottom: parent.top
            anchors.bottomMargin: 1
            text: number.label
            color: root.mutedColor
            font.pixelSize: 11
        }
    }

    component Toggle: Item {
        id: toggle

        property string label: ""
        property bool checked: false

        signal toggled()

        width: box.width + 8 + toggleLabel.width
        height: 22

        Rectangle {
            id: box
            width: 15
            height: 15
            anchors.verticalCenter: parent.verticalCenter
            radius: 4
            color: toggle.checked ? root.accent : "transparent"
            border.width: 1
            border.color: toggle.checked ? root.accent : Theme.scrim

            Text {
                anchors.centerIn: parent
                visible: toggle.checked
                text: "\u2713"
                color: Theme.onInteractive
                font.pixelSize: 11
            }
        }

        Text {
            id: toggleLabel
            anchors.left: box.right
            anchors.leftMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            text: toggle.label
            color: root.textColor
            font.pixelSize: 11
        }

        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: toggle.toggled()
        }
    }
}
