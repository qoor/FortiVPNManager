#!/bin/sh
# Replaces the installed app with a new build, relaunches it and reports whether the tunnel came back.
# Usage: install-app.sh <built app> <installed app> <bundle id>
# Quitting the app drops the VPN, which may carry the very session that ran `make install`; when that
# session goes, its shell and process group can be killed. So the swap runs detached and always
# finishes, and this process only waits for it and reports.
set -u
built=$1 installed=$2 app_id=$3
name=${installed##*/}
name=${name%.app}
root=$(cd "$(dirname "$0")/.." && pwd)
log=$root/.build/install.log
done_file=$root/.build/install.done
vpn_log=$HOME/Library/Logs/FortiVPNManager/openfortivpn.log

swap() {
    set -x
    osascript -e "tell application id \"$app_id\" to quit"
    for i in $(seq 1 15); do pgrep -x "$name" >/dev/null || break; sleep 1; done
    # The old helper exits by itself once its binary is replaced (components/os-integration-helper/macos/SelfUpdateWatcher.swift).
    rm -rf "$installed" && ditto "$built" "$installed" && open "$installed"
    echo $? >"$done_file"
}

rm -f "$done_file"
start=$(date +%Y-%m-%dT%H:%M:%S)
# macOS has no setsid(1). `set -m` puts the background job in its own process group, and the job
# outlives this subshell, so launchd adopts it: killing our process group or process tree misses it.
# A terminal hangup signals only the foreground group and the session leader, and the job ignores
# SIGHUP anyway. Its stdio is the log, so a revoked terminal cannot fail its writes.
pid=$(trap '' HUP; set -m; swap </dev/null >"$log" 2>&1 & echo $!)
echo "install: quitting, replacing and relaunching $installed in the background (log: $log)"
while [ ! -s "$done_file" ] && kill -0 "$pid" 2>/dev/null; do sleep 1; done
if [ "$(cat "$done_file" 2>/dev/null)" != 0 ]; then
    echo "install: relaunch failed; see $log" >&2
    exit 1
fi

# openfortivpn.log lines start with ISO-8601 local timestamps, so the first 19 characters compare as strings.
for i in $(seq 1 90); do
    if awk -v start="$start" 'substr($0, 1, 19) > start && /Tunnel is up and running\./ { up = 1; exit }
        END { exit !up }' "$vpn_log" 2>/dev/null; then
        echo "install: relaunched and the tunnel is up."
        exit 0
    fi
    sleep 1
done
echo "install: relaunched, but no tunnel within 90 s (launch auto-connect happens only inside the configured schedule)."
