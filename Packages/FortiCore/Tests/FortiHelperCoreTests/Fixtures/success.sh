# Successful run (tokeninfo or HTML-form OTP prompt); on SIGTERM tears down like openfortivpn's
# io_loop handler (routes restored) and exits 0.
# usage: /bin/sh success.sh <record-file> [tokeninfo|htmlform]
record="$1"
. "${0%/*}/lib.sh"

password_prompt
connected
otp_prompt "${2:-tokeninfo}"
read_secret otp
note_stdin_state
authenticated
tunnel_setup

on_term() {
    note 'term=yes'
    teardown 'INFO:   '
    [ -n "$sleeper" ] && kill "$sleeper" 2>/dev/null
    exit 0
}
# openfortivpn installs its SIGTERM handler before the tunnel comes up (io.c:661-673).
trap on_term TERM
tunnel_up
hang
note 'hang_returned=yes'
exit 3
