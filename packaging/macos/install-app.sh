#!/bin/sh
# Replaces the installed app with a new build, relaunches it and reports whether the tunnel came back.
# Usage: install-app.sh <built app> <installed app> <bundle id>
# Quitting the app drops the VPN, which may carry the very session that ran `make install`; when that
# session goes, its shell and process group can be killed. So the swap runs detached and always
# finishes, and this process only waits for it and reports. When the VPN was up before the swap and
# the new build brings no tunnel back, the swap puts the previous app back, so the VPN returns.
set -u
built=$1 installed=$2 app_id=$3
name=${installed##*/}
name=${name%.app}
root=$(cd "$(dirname "$0")/../.." && pwd)
log=$root/.build/install.log
done_file=$root/.build/install.done
previous=$root/.build/previous.app
tunnel_wait=90

vpn_connected() {
    { ifconfig; echo '#routes'; netstat -rn -f inet; } 2>/dev/null | awk '
        $0 == "#routes" { routes = 1; next }
        !routes && /^[^[:space:]]/ { name = $1; sub(/:$/, "", name); next }
        !routes && $1 == "inet" {
            if (!(($2, name) in held)) { held[$2, name] = 1; holders[$2]++ }
            if (name ~ /^utun[0-9]+$/) address[name] = $2
            next
        }
        routes && $4 ~ /^utun[0-9]+$/ {
            if (($4 in address) && $1 == address[$4]) next
            if (!(($1, $4) in via)) { via[$1, $4] = 1; carried[$4]++; carriers[$1]++ }
        }
        END {
            for (tunnel in address) {
                if (holders[address[tunnel]] != 1 || !carried[tunnel]) continue
                alone = 1
                for (key in via) {
                    split(key, part, SUBSEP)
                    if (part[2] == tunnel && carriers[part[1]] > 1) alone = 0
                }
                if (alone) found = 1
            }
            exit !found
        }'
}

wait_for_tunnel() {
    for i in $(seq 1 "$tunnel_wait"); do
        vpn_connected && return 0
        sleep 1
    done
    return 1
}

quit_app() {
    osascript -e "tell application id \"$app_id\" to quit"
    for i in $(seq 1 15); do pgrep -x "$name" >/dev/null || break; sleep 1; done
}

# Writes the outcome to the done file: installed, no-tunnel, rolled-back or failed.
swap() {
    set -x
    vpn_was_up=no
    vpn_connected && vpn_was_up=yes
    quit_app
    rm -rf "$previous"
    if [ -e "$installed" ]; then
        mv "$installed" "$previous" || { open "$installed"; echo failed >"$done_file"; return; }
    fi
    # The old helper exits by itself once its binary is replaced (components/helper/macos/swift/SelfUpdateWatcher.swift).
    if ! ditto "$built" "$installed" || ! open "$installed"; then
        rm -rf "$installed"
        [ -e "$previous" ] && mv "$previous" "$installed" && open "$installed"
        echo failed >"$done_file"
        return
    fi
    if wait_for_tunnel; then
        echo installed >"$done_file"
    elif [ "$vpn_was_up" = yes ] && [ -e "$previous" ]; then
        quit_app
        rm -rf "$installed" && mv "$previous" "$installed" && open "$installed"
        echo rolled-back >"$done_file"
    else
        echo no-tunnel >"$done_file"
    fi
}

if [ -e "$installed" ] && { [ ! -w "$installed" ] || [ ! -w "$(dirname "$installed")" ]; }; then
    echo "install: no permission to replace $installed" >&2
    exit 1
fi
rm -f "$done_file"
# macOS has no setsid(1). `set -m` puts the background job in its own process group, and the job
# outlives this subshell, so launchd adopts it: killing our process group or process tree misses it.
# A terminal hangup signals only the foreground group and the session leader, and the job ignores
# SIGHUP anyway. Its stdio is the log, so a revoked terminal cannot fail its writes.
pid=$(trap '' HUP; set -m; swap </dev/null >"$log" 2>&1 & echo $!)
echo "install: quitting, replacing and relaunching $installed in the background (log: $log)"
while [ ! -s "$done_file" ] && kill -0 "$pid" 2>/dev/null; do sleep 1; done
case $(cat "$done_file" 2>/dev/null) in
installed) echo "install: relaunched and the tunnel is up." ;;
no-tunnel) echo "install: relaunched, but no tunnel within $tunnel_wait s (launch auto-connect happens only inside the configured schedule)." ;;
rolled-back)
    echo "install: no tunnel within $tunnel_wait s although the VPN was up before; the previous app is back (see $log)." >&2
    exit 1
    ;;
*)
    echo "install: relaunch failed; see $log" >&2
    exit 1
    ;;
esac
