# Gets as far as STAGE and then hangs with no output, for the watchdog tests.
# STAGE: spawned | gateway | otp | submitted | authenticated | tunnel
# With "ignore-term" the shell ignores SIGTERM, so only SIGKILL ends it.
# usage: /bin/sh hang.sh <record-file> <stage> [ignore-term]
record="$1"
stage="$2"
. "${0%/*}/lib.sh"

if [ "$3" = ignore-term ]; then
    trap '' TERM
fi
note 'started=yes'
if [ "$stage" = spawned ]; then hang; exit 0; fi
password_prompt
connected
if [ "$stage" = gateway ]; then hang; exit 0; fi
otp_prompt tokeninfo
if [ "$stage" = otp ]; then hang; exit 0; fi
read_secret otp
printf '\n'
if [ "$stage" = submitted ]; then hang; exit 0; fi
echo 'INFO:   Authenticated.'
if [ "$stage" = authenticated ]; then hang; exit 0; fi
tunnel_setup
tunnel_up
hang
exit 0
