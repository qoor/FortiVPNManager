# /remote/logincheck rejects the password ("password") or the OTP ("otp"), then exit 1.
# usage: /bin/sh auth-failed.sh <record-file> [password|otp]
record="$1"
. "${0%/*}/lib.sh"

password_prompt
connected
if [ "${2:-password}" = otp ]; then
    otp_prompt tokeninfo
    read_secret otp
    printf '\n'
fi
echo 'ERROR:  Could not authenticate to gateway. Please check the password, client certificate, etc.'
echo 'INFO:   Closed connection to gateway.'
echo 'INFO:   Logged out.'
exit 1
