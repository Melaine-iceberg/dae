# Terminal shell integration

dae's terminal panel follows the shell's working directory by listening to
what the shell *reports* — OSC 7 (`file://` URI), OSC 9;9 (ConEmu CurrentDir)
or OSC 1337 (iTerm2 CurrentDir) — not by injecting `cd` into a running
shell, which would pollute the shell's own command history. dae sets the
`DAE_TERM=1` environment variable for every shell it spawns; the snippets
here use it to activate only inside dae.

Without integration, a new session still starts in the folder the active
tab is showing; the panel's directory column then stays on that spawn
folder instead of following `cd`.

| Shell | Snippet | Install |
|---|---|---|
| bash | `bash-zsh.sh` | `echo 'source <repo>/Assets/terminal-integration/bash-zsh.sh' >> ~/.bashrc` |
| zsh | `bash-zsh.sh` | `echo 'source <repo>/Assets/terminal-integration/bash-zsh.sh' >> ~/.zshrc` |
| PowerShell | `powershell.ps1` | add `. <repo>\Assets\terminal-integration\powershell.ps1` to `$PROFILE` |

This mirrors Zed's approach (`ZED_TERM` + shell snippets the user opts
into); forcing a shell to load integration via `--rcfile` / `-Command`
would override the user's own shell configuration.
