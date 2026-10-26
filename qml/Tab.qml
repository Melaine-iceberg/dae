import QtQuick
import dae 1.0

// One browsing session: a path, the streamed model behind it, and the
// selection that belongs to *this* tab rather than to the window. A tab that
// moves to another window keeps its selection because the object moves with
// it, the same way its rows do.
QtObject {
    id: tab

    property string title: listing.path !== "" ? listing.path : "dae"
    property DirectoryListModel listing: DirectoryListModel {}
    property SearchModel nameSearch: SearchModel {}
    property ContentSearchModel contentSearch: ContentSearchModel {}

    /// path -> true. Replaced wholesale so QML bindings re-evaluate; mutating
    /// an object in place would not notify the delegates.
    property var selected: ({})
    property int selectionVersion: 0
    property string selectionAnchor: ""

    function navigate(path) {
        if (path && path.length > 0) {
            cancelSearch()
            listing.load(path)
            clearSelection()
        }
    }

    function cancelSearch() {
        nameSearch.cancel()
        contentSearch.cancel()
    }

    function clearSelection() {
        selected = ({})
        selectionAnchor = ""
        selectionVersion += 1
    }

    function selectOnly(path) {
        const next = ({})
        if (path)
            next[path] = true
        selected = next
        selectionAnchor = path || ""
        selectionVersion += 1
    }

    function toggle(path) {
        const next = ({})
        for (const key in selected) {
            if (selected[key])
                next[key] = true
        }
        if (next[path])
            delete next[path]
        else
            next[path] = true
        selected = next
        selectionAnchor = path
        selectionVersion += 1
    }

    function selectMany(paths) {
        const next = ({})
        for (let index = 0; index < paths.length; index += 1)
            next[paths[index]] = true
        selected = next
        selectionVersion += 1
    }

    function isSelected(path) {
        // The read is what makes delegate bindings depend on the version.
        return selectionVersion >= 0 && selected[path] === true
    }

    function dispose() {
        cancelSearch()
        listing.dispose()
        nameSearch.dispose()
        contentSearch.dispose()
    }
}
