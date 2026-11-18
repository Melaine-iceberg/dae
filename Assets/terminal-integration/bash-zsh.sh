# dae terminal integration — bash and zsh.
#
# Reports the shell's working directory with OSC 7 after every prompt, so
# dae's terminal panel can follow the directory the shell is actually in.
# dae never injects `cd`; the shell reports, the panel listens.
#
# Install (bash):  echo 'source /path/to/bash-zsh.sh' >> ~/.bashrc
# Install (zsh):   echo 'source /path/to/bash-zsh.sh' >> ~/.zshrc
#
# Only active inside dae's terminal ($DAE_TERM), so sourcing this from a
# shared rc file is harmless elsewhere.

if [ -n "$DAE_TERM" ]; then
    __dae_report_pwd() {
        # percent-encode the path, then wrap it as a file URI; UTF-8 names
        # survive the round trip and the host part is empty (localhost).
        local encoded=""
        local ch
        local i
        i=0
        while i=$((i + 1)) && [ "$i" -le ${#PWD} ]; do
            ch=${PWD:i-1:1}
            case "$ch" in
                [a-zA-Z0-9/._~-]) encoded="$encoded$ch" ;;
                *) encoded="$encoded$(printf '%%%02X' "'$ch")" ;;
            esac
        done
        printf '\e]7;file://%s%s\e\\' "$(hostname)" "$encoded"
    }

    if [ -n "$ZSH_VERSION" ]; then
        autoload -Uz add-zsh-hook
        add-zsh-hook precmd __dae_report_pwd
    else
        PROMPT_COMMAND="__dae_report_pwd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
    fi
fi
