# Shared helpers for the fake openfortivpn 1.24.1 fixtures (sourced, not run).
# Fixtures run as `/bin/sh <fixture> <record-file> [args...]` with PATH as the only environment
# variable. They report what they received on stdin to the record file as key=value lines:
# lengths and SHA-256 digests only, never the secrets themselves.
# Output strings are copied from openfortivpn 1.24.1 (log.c prefixes, main.c, tunnel.c, io.c) and
# Apple pppd; replace them with real Phase 0b captures when available.

note() {
    printf '%s\n' "$1" >> "$record"
}

# read_secret NAME: reads one line from stdin and records its length and SHA-256.
read_secret() {
    if IFS= read -r secret; then
        note "$1_eof=no"
    else
        note "$1_eof=yes"
    fi
    note "$1_len=${#secret}"
    note "$1_sha256=$(printf '%s' "$secret" | /sbin/sha256 -q)"
    secret=''
}

# note_stdin_state: records whether stdin is at EOF (the helper closes it after the OTP).
note_stdin_state() {
    if IFS= read -r extra; then
        note 'stdin_closed=no'
    else
        note 'stdin_closed=yes'
    fi
}

# hang: blocks until a signal ends the shell. A trapped TERM makes `wait` return so the trap runs.
# The sleeper does not hold the output pipes.
hang() {
    /bin/sleep 30 </dev/null >/dev/null 2>&1 &
    sleeper=$!
    wait "$sleeper"
}

# Prompts print without a newline and flush (userinput.c:344-345). The "\n" printed after reading
# (userinput.c:367) stays in stdio's buffer until the next log line, so it is prefixed to that line.
password_prompt() {
    printf 'VPN account password: '
    read_secret password
}

connected() {
    printf '\nINFO:   Connected to gateway.\n'
}

# otp_prompt [tokeninfo|htmlform]
otp_prompt() {
    if [ "$1" = htmlform ]; then
        printf 'Please enter one-time password: '
    else
        printf 'Two-factor authentication token: '
    fi
}

authenticated() {
    printf '\nINFO:   Authenticated.\n'
}

tunnel_setup() {
    echo 'INFO:   Remote gateway has allocated a VPN.'
    echo 'Thu Oct 15 09:12:04 2026 : publish_entry SCDSet() failed: Success!' >&2
    echo 'Thu Oct 15 09:12:04 2026 : Using interface ppp0' >&2
    echo 'Thu Oct 15 09:12:04 2026 : Connect: ppp0 <--> /dev/ttys004' >&2
    echo 'INFO:   Got addresses: [10.212.134.200], ns [10.0.0.53, 10.0.0.54], ns_suffix [corp.example.com]'
    echo 'INFO:   Negotiation complete.'
    echo 'INFO:   Negotiation complete.'
    echo 'Thu Oct 15 09:12:04 2026 : local  IP address 10.212.134.200' >&2
    echo 'Thu Oct 15 09:12:04 2026 : remote IP address 169.254.2.1' >&2
    echo 'INFO:   Interface ppp0 is UP.'
    echo 'INFO:   Setting new routes...'
    echo 'add host 203.0.113.10: gateway 192.168.1.1'
    echo 'delete net 0.0.0.0: gateway 192.168.1.1'
    echo 'add net 0.0.0.0: gateway ppp0'
    echo 'INFO:   Adding VPN nameservers...'
}

tunnel_up() {
    echo 'INFO:   Tunnel is up and running.'
}

# teardown PPPD_LEVEL_PREFIX: what openfortivpn prints after the tunnel goes down
# (tunnel.c:150-165, 483-572). The pppd line is INFO after SIGTERM and ERROR otherwise.
teardown() {
    echo 'Thu Oct 15 12:12:05 2026 : Modem hangup' >&2
    echo 'Thu Oct 15 12:12:05 2026 : Connection terminated.' >&2
    echo 'INFO:   Cancelling threads...'
    echo 'INFO:   Cleanup, joining threads...'
    echo 'INFO:   Setting ppp0 interface down.'
    echo 'INFO:   Restoring routes...'
    echo 'delete host 203.0.113.10: gateway 192.168.1.1'
    echo 'delete net 0.0.0.0: gateway ppp0'
    echo 'add net 0.0.0.0: gateway 192.168.1.1'
    echo 'INFO:   Removing VPN nameservers...'
    echo "$1pppd: The link was terminated by the modem hanging up."
    echo 'INFO:   Terminated pppd.'
    echo 'INFO:   Closed connection to gateway.'
    echo 'INFO:   Logged out.'
}

# The block tunnel.c:900-935 prints when the gateway certificate is rejected.
cert_block() {
    /bin/cat <<'BLOCK'
ERROR:  Gateway certificate validation failed, and the certificate digest is not in the local whitelist. If you trust it, rerun with:
ERROR:      --trusted-cert b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11
ERROR:  or add this line to your configuration file:
ERROR:      trusted-cert = b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11
ERROR:  Gateway certificate:
ERROR:      subject:
ERROR:          C=US
ERROR:          ST=California
ERROR:          L=Sunnyvale
ERROR:          O=Fortinet
ERROR:          OU=FortiGate
ERROR:          CN=FGT60FTK00000000
ERROR:          emailAddress=support@fortinet.com
ERROR:      issuer:
ERROR:          C=US
ERROR:          ST=California
ERROR:          L=Sunnyvale
ERROR:          O=Fortinet
ERROR:          OU=Certificate Authority
ERROR:          CN=fortinet-subca2001
ERROR:          emailAddress=support@fortinet.com
ERROR:      sha256 digest:
ERROR:          b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11
BLOCK
}
