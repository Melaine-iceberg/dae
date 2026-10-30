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
            return qsTr("回收站")
        if (surface === "overview")
            return qsTr("概览")
        if (surface === "space")
            return spaceTitle.length > 0 ? spaceTitle : qsTr("空间")
        return listing.path !== "" ? listing.path : "dae"
    }
    property DirectoryListModel listing: DirectoryListModel {}
    property SearchModel nameSearch: SearchModel {}
    property ContentSearchModel contentSearch: ContentSearchModel {}
    /// The shell shown in this tab's terminal panel. One session per tab, so
    /// hiding the panel or switching tabs keeps it running, and a torn-off tab
    /// carries its shell along. `TerminalPanel` starts it on first use.
    property TerminalSession terminal: TerminalSession {}

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

    // ── Navigation history ──────────────────────────────────────────────────
    //
    // Two stacks of `{ path, selected, anchor }`, oldest first, the newest move
    // last. On the tab and not on the window, because the tab is the browsing
    // session: tearing one off into its own window has to keep its back button
    // working. The cap is a memory guard, not a feature — a file manager with
    // 64 steps back is already past what anyone walks.
    property var backStack: []
    property var forwardStack: []
    readonly property int historyLimit: 64
    readonly property bool canGoBack: backStack.length > 0
    readonly property bool canGoForward: forwardStack.length > 0

    function historyEntry() {
        return { path: listing.path, selected: selected, anchor: selectionAnchor }
    }

    function pushHistory(stack, entry) {
        const next = stack.slice()
        next.push(entry)
        while (next.length > historyLimit)
            next.shift()
        return next
    }

    /// The one place that puts a folder on screen. `keepSelection` is how a
    /// history move restores what was selected there rather than arriving with
    /// an empty selection the way a fresh navigation does.
    function loadFolder(path, keepSelection) {
        surface = "folder"
        spaceId = ""
        cancelSearch()
        listing.load(path)
        if (!keepSelection)
            clearSelection()
    }

    function loadHistoryEntry(entry) {
        loadFolder(entry.path, true)
        // The entry was captured in another visit, so the paths in it may have
        // been renamed or deleted since. A stale key is inert: `isSelected`
        // only ever runs from a delegate over rows that exist now.
        selected = entry.selected ? entry.selected : ({})
        selectionAnchor = entry.anchor ? entry.anchor : ""
        selectionVersion += 1
    }

    function navigate(path) {
        if (!path || path.length === 0)
            return
        // Only a step away from a folder is worth remembering. Arriving here
        // from the recycle bin, a space or the overview would otherwise push a
        // `listing.path` that no longer describes what the user was looking at.
        if (surface === "folder" && listing.path !== "" && listing.path !== path)
            backStack = pushHistory(backStack, historyEntry())
        // A new branch invalidates the forward one, exactly as in a browser.
        forwardStack = []
        loadFolder(path, false)
    }

    function goBack() {
        if (backStack.length === 0)
            return
        const target = backStack[backStack.length - 1]
        backStack = backStack.slice(0, backStack.length - 1)
        forwardStack = pushHistory(forwardStack, historyEntry())
        loadHistoryEntry(target)
    }

    function goForward() {
        if (forwardStack.length === 0)
            return
        const target = forwardStack[forwardStack.length - 1]
        forwardStack = forwardStack.slice(0, forwardStack.length - 1)
        backStack = pushHistory(backStack, historyEntry())
        loadHistoryEntry(target)
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

    /// Starts this tab's shell at its current folder. The panel calls this
    /// instead of reaching through `terminal` itself: on a tab switch the
    /// panel's derived `session` binding may not have re-evaluated yet, and
    /// starting the previous tab's shell twice would leave this one dead.
    function startTerminal() {
        if (terminal.running)
            return
        terminal.start(folder ? listing.path : "", 80, 24, ExplorerPrefs.terminalShell)
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
        terminal.kill()
    }
}
