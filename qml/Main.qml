import QtQuick

// The first shipping window. Everything the spikes measured is in ShellWindow:
// the frameless native frame and backdrop (gates 3/4), the tab strip with
// tear-off, the streaming model (gate 1) and a real TextInput for the IME
// focus guard (gate 2).
ShellWindow {
    token: "dae-main"
    firstWindow: true
}
