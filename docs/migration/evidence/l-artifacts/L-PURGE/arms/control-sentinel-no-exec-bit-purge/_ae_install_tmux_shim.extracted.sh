_ae_install_tmux_shim() {
    if [[ "${AE_TMUX_SERVER_KIND:-}" == "ambiguous" ]]; then
        tmux() {
            echo "ae: refusing tmux operation — this session's server record is ambiguous; run 'ae doctor --refresh' first." >&2
            return 1
        }
        return 0
    fi
    [[ -n "${AE_TMUX_SERVER:-}" ]] || return 0
    if [[ "${AE_TMUX_SERVER_KIND:-}" == "socket" ]]; then
        tmux() { command tmux -S "$AE_TMUX_SERVER" "$@"; }
    elif [[ "${AE_TMUX_SERVER_KIND:-}" == "name" ]]; then
        tmux() { command tmux -L "$AE_TMUX_SERVER" "$@"; }
    elif [[ "$AE_TMUX_SERVER" == /* ]]; then
        tmux() { command tmux -S "$AE_TMUX_SERVER" "$@"; }
    else
        tmux() { command tmux -L "$AE_TMUX_SERVER" "$@"; }
    fi
    return 0
}
