# dae terminal integration - PowerShell 7+ and Windows PowerShell 5.1.
#
# Reports the shell's working directory with OSC 9;9 (ConEmu CurrentDir,
# which dae reads the same as OSC 7) after every prompt, so dae's terminal
# panel can follow the directory the shell is actually in. dae never
# injects `cd`; the shell reports, the panel listens.
#
# Install: add this line to your $PROFILE
#     . /path/to/Assets/terminal-integration/powershell.ps1
#
# Only active inside dae's terminal ($env:DAE_TERM), so sourcing it from a
# shared profile is harmless elsewhere.

if ($env:DAE_TERM) {
    $global:__daeOriginalPrompt = $function:global:prompt

    function global:prompt {
        [Console]::Write("$([char]27)]9;9;`"$($executionContext.SessionState.Path.CurrentLocation)`"$([char]27)\")
        if ($global:__daeOriginalPrompt) {
            & $global:__daeOriginalPrompt
        } else {
            "PS $($executionContext.SessionState.Path.CurrentLocation)> "
        }
    }
}
