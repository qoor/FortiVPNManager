# Closes stdin, prints the password prompt and exits shortly after without reading: the helper's
# password write fails with EPIPE.
# usage: /bin/sh early-exit.sh <record-file>
record="$1"
. "${0%/*}/lib.sh"

exec 0<&-
printf 'VPN account password: '
/bin/sleep 0.3
note 'exited=yes'
exit 1
