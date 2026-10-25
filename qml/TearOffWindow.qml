import QtQuick

// A second window is the same object as the first. Gate 4 measured this shape:
// the native state is keyed by HWND, so nothing here needs a second code path.
ShellWindow {
    firstWindow: false
}
