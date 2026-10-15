# Tunnel comes up, then the gateway ends the session (exit 1 with pppd's messages on stderr), or
# with "crash" the process dies by SIGKILL right after changing routes.
# usage: /bin/sh server-cut.sh <record-file> [cut|crash]
record="$1"
. "${0%/*}/lib.sh"

password_prompt
connected
otp_prompt tokeninfo
read_secret otp
authenticated
tunnel_setup
tunnel_up
if [ "${2:-cut}" = crash ]; then
    kill -KILL $$
fi
/bin/sleep 0.3
teardown 'ERROR:  '
exit 1
