#!/bin/sh
# Prints the Team ID (certificate OU) of the first "Apple Development" signing identity.
set -eu
cert=$(security find-certificate -c "Apple Development" -p 2>/dev/null) || {
    echo "No Apple Development certificate found. Create one in Xcode > Settings > Accounts > Manage Certificates." >&2
    exit 1
}
printf '%s\n' "$cert" | openssl x509 -noout -subject -nameopt multiline | awk -F' = ' '/organizationalUnitName/ { print $2; exit }'
