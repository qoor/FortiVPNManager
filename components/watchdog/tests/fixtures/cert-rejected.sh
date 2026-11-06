# Gateway certificate not trusted: the block is printed for the connection and again for the
# logout attempt, then openfortivpn exits 1.
# usage: /bin/sh cert-rejected.sh <record-file>
record="$1"
. "${0%/*}/lib.sh"

password_prompt
printf '\n'
cert_block
echo 'INFO:   Closed connection to gateway.'
cert_block
echo 'INFO:   Could not log out.'
exit 1
