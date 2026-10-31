import QtQuick

// A surface that is built the first time it is needed.
//
// Every window used to build all of its dialogs and hidden views before the
// first frame, even though a session opens a handful of them. It costs no
// startup time — measured inside the noise — but it is resident memory: the
// working set went from 240–242 MB to 218–229 MB once every on-demand surface
// in `ShellWindow.qml` was deferred, and a second window (torn off, or Ctrl+N)
// pays the same again.
//
// `visible` is deliberately *not* overridden here: an `Item`'s `visible` is the
// effective one, so mirroring the child's would make the two definitions chase
// each other and settle on false (the surface would never appear). Callers read
// `shown` instead. Openers call `instantiate()` and the surface exists from then
// on; nothing is ever destroyed, so a reply that arrives after a dialog was
// closed still reaches a live object.
Item {
    id: root

    /// The surface, as `Component { SomeDialog { ... } }`.
    property Component surface: null

    /// For surfaces shown by state rather than opened by a call: true creates
    /// it, and it stays created after the first time.
    property bool loadWhenShown: false

    /// Whether the surface has been created.
    property bool everLoaded: false

    readonly property var content: loader.item
    /// Whether the wrapped surface is showing.
    // qmllint disable missing-property
    readonly property bool shown: loader.item !== null && loader.item.visible

    /// Creates the surface if needed and returns it. Synchronous: `loader.item`
    /// is ready by the time this returns.
    function instantiate() {
        everLoaded = true;
        return loader.item;
    }

    Loader {
        id: loader
        anchors.fill: parent
        active: root.loadWhenShown || root.everLoaded
        sourceComponent: root.surface
        onLoaded: root.everLoaded = true
    }
}
