.pragma library

// Keeps Tab inside a modal dialog. The shell is one focus chain, so without
// this a dialog would hand focus to the page behind its scrim. Walking
// nextItemInFocusChain rather than keeping a list of ids means fields added
// later become part of the cycle automatically.

function isInside(scope, item) {
    var node = item
    for (var i = 0; i < 64 && node; ++i) {
        if (node === scope)
            return true
        node = node.parent
    }
    return false
}

function firstInside(scope, forward) {
    var item = scope.nextItemInFocusChain(forward)
    for (var i = 0; i < 64 && item; ++i) {
        if (item !== scope && isInside(scope, item))
            return item
        item = item.nextItemInFocusChain(forward)
    }
    return null
}

function cycle(scope, forward) {
    var from = scope.activeFocusItem || scope
    var item = from.nextItemInFocusChain(forward)
    for (var i = 0; i < 64 && item; ++i) {
        if (item !== scope && isInside(scope, item)) {
            item.forceActiveFocus(Qt.TabFocusReason)
            return true
        }
        item = item.nextItemInFocusChain(forward)
    }
    var wrapped = firstInside(scope, forward)
    if (wrapped) {
        wrapped.forceActiveFocus(Qt.TabFocusReason)
        return true
    }
    scope.forceActiveFocus(Qt.TabFocusReason)
    return true
}
