import QtQuick
import dae 1.0

// One browsing session: a path, the streamed model behind it, and the
// selection that belongs to *this* tab rather than to the window. A tab that
// moves to another window keeps its selection because the object moves with
// it, the same way its rows do.
QtObject {
    id: tab

    /// What this tab is showing. A folder tab owns `listing`; the trash,
    /// overview and space surfaces reuse the same tab so the tab strip keeps
    /// one stable identity while the body swaps.
    property string surface: "folder"
    property string spaceId: ""
    readonly property bool folder: surface === "folder"
    readonly property bool trash: surface === "trash"
    readonly property bool overview: surface === "overview"
    readonly property bool space: surface === "space"
    readonly property string spaceTitle: {
        // Reading `revision` is what makes a rename in another window update
        // this tab's title.
        if (surface !== "space" || spaceId === "" || Spaces.revision < 0)
            return ""
        return Spaces.nameOfId(spaceId)
    }
    property string title: {
        if (surface === "trash")
            return "回收站"
        if (surface === "overview")
            return "概览"
        if (surface === "space")
            return spaceTitle.length > 0 ? spaceTitle : "空间"
        return listing.path !== "" ? listing.path : "dae"
    }
    property DirectoryListModel listing: DirectoryListModel {}
    property SearchModel nameSearch: SearchModel {}
    property ContentSearchModel contentSearch: ContentSearchModel {}

    /// path -> true. Replaced wholesale so QML bindings re-evaluate; mutating
    /// an object in place would not notify the delegates.
    property var selected: ({})
    property int selectionVersion: 0
    property string selectionAnchor: ""

    /// path -> { y, current } for the listing viewport. Kept on the tab so a
    /// refresh — auto or F5 — and a return to a directory restore the position
    /// the model reset would otherwise drop. Replaced wholesale like
    /// `selected`: the view reads it in a function call, not in a binding.
    property var scrollState: ({})

    function navigate(path) {
        if (path && path.length > 0) {
            surface = "folder"
            spaceId = ""
            cancelSearch()
            listing.load(path)
            clearSelection()
        }
    }

    /// Shows the system recycle bin in this tab. The window owns the model and
    /// refreshes it when the surface becomes active.
    function openTrash() {
        cancelSearch()
        clearSelection()
        surface = "trash"
        spaceId = ""
    }

    /// Shows the landing page. Its data lives in the window's OverviewModel.
    function openOverview() {
        cancelSearch()
        clearSelection()
        surface = "overview"
        spaceId = ""
    }

    /// Shows a space by id. The space view owns its item list; a rename keeps
    /// the id stable, and deleting the space closes this surface.
    function openSpace(id) {
        if (!id)
            return
        cancelSearch()
        clearSelection()
        surface = "space"
        spaceId = id
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
