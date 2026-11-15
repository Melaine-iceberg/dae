import QtQuick
import QtQuick.Layouts

/// One list row (§12.1). Every list-like page — overview, trash, spaces and
/// the column browser — mounts this instead of restating the row grammar:
/// 40 dp tall by default, a glyph, one or two lines of text, optional trailing
/// content, a 4% hover wash, a 10% accent selection and a keyboard cursor.
///
/// It is `ListRow`, not `Row`: QtQuick already defines `Row`, and a file named
/// `Row.qml` in this directory shadows it for every document here.
///
/// The row owns its interaction states and leaves the cells to the page. A
/// page adds middle columns through `default` content (they land in a
/// `RowLayout` and can use `Layout.*`) and right-aligned content through
/// `trailing`; `meta` is a plain right-aligned figure for the common case.
Item {
    id: row

    property bool isSelected: false
    property bool previousSelected: false
    property bool nextSelected: false
    /// The ListView's current index, used for `Accessible.selected`.
    property bool isCurrent: false
    /// Paint the §7.1.8 cursor. Owners only turn this on for keyboard moves.
    property bool keyboardCursor: false
    property bool dropTarget: false
    property bool flash: false
    property bool interactive: true
    property bool focusable: false
    property bool showFocusRail: true
    /// A non-selected row that should still read as inhabited (the folder a
    /// column has open). The wash is the hover wash.
    property bool highlighted: false
    /// Set by an owner that handles a drag: the release that follows must not
    /// also count as a click.
    property bool clickSuppressed: false

    property int rowHeight: Theme.rowSpacious
    property int contentInset: 8
    property int spacing: 10
    /// Middle cells are fixed-width columns by default so a page can line them
    /// up with its header; the title column absorbs the leftover width.
    property bool middleFill: false

    property string title
    property string subtitle
    /// Read `subtitle` on the title's line instead of under it: the overview's
    /// recents row is name + relative path + time in one line (§11.1).
    property bool subtitleInline: false
    property string meta
    property int metaWidth: 96
    property string iconCategory: "file"
    property int iconEdge: 18
    property real iconOpacity: 1
    property bool showIcon: true
    /// Set by pages that supply a replacement glyph through `leading`; an
    /// invisible child still counts in `childrenRect`, so the slot cannot
    /// decide this from its contents alone.
    property bool showLeading: false
    property color titleColor: Theme.ink
    property int titleSize: Theme.typeBody
    property int titleWeight: Font.Normal
    property bool titleMonospace: false

    property string accessibleName: row.title
    property string accessibleDescription: row.subtitle

    /// Middle cells. Use `Layout.preferredWidth` / `Layout.fillWidth`.
    default property alias content: middle.data
    /// A replacement glyph (a thumbnail, a drive bar). Hides `showIcon`.
    property alias leading: leadingSlot.data
    /// Right-aligned content (a capacity bar, a count, an action).
    property alias trailing: trailingSlot.data

    signal clicked(var mouse)
    signal doubleClicked(var mouse)
    signal rightClicked(var mouse)
    signal pressed(var mouse)
    signal moved(var mouse)
    signal released(var mouse)
    signal canceled()
    signal activated()

    readonly property bool hovered: hoverArea.containsMouse
    readonly property bool focusedCursor: row.keyboardCursor
        && (row.activeFocus || row.isCurrent)

    implicitHeight: rowHeight
    height: rowHeight

    Accessible.role: Accessible.ListItem
    Accessible.name: row.accessibleName
    Accessible.description: row.accessibleDescription
    Accessible.selectable: row.interactive
    Accessible.selected: row.isSelected
    Accessible.onPressAction: row.activated()

    activeFocusOnTab: row.focusable

    Keys.onReturnPressed: row.activated()
    Keys.onEnterPressed: row.activated()
    Keys.onSpacePressed: row.activated()

    Rectangle {
        id: stateMask

        anchors.fill: parent
        color: row.isSelected ? Theme.selection
            : row.dropTarget ? Theme.accentSoftStrong
            : (row.hovered || row.highlighted ? Theme.hover : "transparent")
        topLeftRadius: row.isSelected && row.previousSelected ? 0 : Theme.radiusSm
        topRightRadius: row.isSelected && row.previousSelected ? 0 : Theme.radiusSm
        bottomLeftRadius: row.isSelected && row.nextSelected ? 0 : Theme.radiusSm
        bottomRightRadius: row.isSelected && row.nextSelected ? 0 : Theme.radiusSm
        Accessible.ignored: true
    }

    Rectangle {
        anchors.fill: parent
        visible: row.flash
        radius: Theme.radiusSm
        color: Theme.accentSoftStrong
        Accessible.ignored: true

        Behavior on opacity {
            enabled: Theme.motion
            NumberAnimation { duration: 160; easing.type: Easing.OutCubic }
        }
    }

    Rectangle {
        anchors.fill: parent
        visible: row.dropTarget
        radius: Theme.radiusSm
        color: "transparent"
        border.width: 1.5
        border.color: Theme.accent
        Accessible.ignored: true
    }

    Rectangle {
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: 2
        visible: row.showFocusRail && row.focusedCursor && row.isSelected
        color: Theme.accent
        Accessible.ignored: true
    }

    Rectangle {
        anchors.fill: parent
        visible: row.focusedCursor && !row.isSelected
        radius: Theme.radiusSm
        color: "transparent"
        border.width: 1.5
        border.color: Theme.accent
        Accessible.ignored: true
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: row.contentInset
        anchors.rightMargin: row.contentInset
        spacing: row.spacing

        FileIcon {
            Layout.alignment: Qt.AlignVCenter
            visible: row.showIcon && !leadingSlot.visible
            opacity: row.iconOpacity
            category: row.iconCategory
            edge: row.iconEdge
            outline: row.iconCategory === "folder" ? Theme.inkSecondary
                                                  : Theme.inkTertiary
        }

        Item {
            id: leadingSlot

            Layout.alignment: Qt.AlignVCenter
            Layout.preferredWidth: childrenRect.width
            Layout.preferredHeight: childrenRect.height
            visible: row.showLeading
            implicitWidth: childrenRect.width
            implicitHeight: childrenRect.height
        }

        ColumnLayout {
            visible: row.title !== ""
            Layout.fillWidth: true
            Layout.minimumWidth: 40
            Layout.alignment: Qt.AlignVCenter
            spacing: 1

            /// The one-line shape: the name keeps its natural width up to
            /// 60% of the row, the path takes the rest and elides in the
            /// middle, so the last segment stays readable.
            RowLayout {
                Layout.fillWidth: true
                visible: row.subtitleInline
                spacing: 10

                Text {
                    Layout.maximumWidth: Math.max(64, Math.round(row.width * 0.6))
                    visible: row.title !== ""
                    text: row.title
                    color: row.titleColor
                    font.pixelSize: row.titleSize
                    font.weight: row.titleWeight
                    font.family: row.titleMonospace ? Theme.monoFamily : Theme.uiFamily
                    elide: Text.ElideRight
                    maximumLineCount: 1
                }

                Text {
                    Layout.fillWidth: true
                    visible: row.subtitle !== ""
                    text: row.subtitle
                    color: Theme.inkTertiary
                    font.pixelSize: Theme.typeCaption
                    elide: Text.ElideMiddle
                    maximumLineCount: 1
                }
            }

            Text {
                Layout.fillWidth: true
                visible: !row.subtitleInline && row.title !== ""
                text: row.title
                color: row.titleColor
                font.pixelSize: row.titleSize
                font.weight: row.titleWeight
                font.family: row.titleMonospace ? Theme.monoFamily : Theme.uiFamily
                elide: Text.ElideRight
                maximumLineCount: 1
            }

            Text {
                Layout.fillWidth: true
                visible: !row.subtitleInline && row.subtitle !== ""
                text: row.subtitle
                color: Theme.inkTertiary
                font.pixelSize: Theme.typeCaption
                elide: Text.ElideMiddle
                maximumLineCount: 1
            }
        }

        RowLayout {
            id: middle

            Layout.fillWidth: row.middleFill
            Layout.alignment: Qt.AlignVCenter
            visible: children.length > 0
            spacing: row.spacing
        }

        Text {
            Layout.alignment: Qt.AlignVCenter
            Layout.preferredWidth: row.meta !== "" ? row.metaWidth : 0
            visible: row.meta !== ""
            text: row.meta
            color: Theme.inkSecondary
            font.pixelSize: Theme.typeCaption
            font.features: { "tnum": 1 }
            horizontalAlignment: Text.AlignRight
            elide: Text.ElideRight
        }

        Item {
            id: trailingSlot

            Layout.alignment: Qt.AlignVCenter
            Layout.preferredWidth: childrenRect.width
            Layout.preferredHeight: childrenRect.height
            visible: children.length > 0
            implicitWidth: childrenRect.width
            implicitHeight: childrenRect.height
        }
    }

    MouseArea {
        id: hoverArea

        anchors.fill: parent
        /// Below the cells: a checkbox or a per-row action still gets the
        /// click first; the row only receives what the cells leave behind.
        z: -1
        hoverEnabled: row.interactive
        enabled: row.interactive
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        cursorShape: Qt.PointingHandCursor
        onPressed: function(mouse) {
            row.pressed(mouse)
        }
        onPositionChanged: function(mouse) {
            row.moved(mouse)
        }
        onReleased: function(mouse) {
            row.released(mouse)
        }
        onCanceled: row.canceled()
        onClicked: function(mouse) {
            if (row.clickSuppressed) {
                row.clickSuppressed = false
                return
            }
            if (mouse.button === Qt.RightButton)
                row.rightClicked(mouse)
            else
                row.clicked(mouse)
        }
        onDoubleClicked: function(mouse) {
            if (mouse.button === Qt.LeftButton)
                row.doubleClicked(mouse)
        }
    }
}
