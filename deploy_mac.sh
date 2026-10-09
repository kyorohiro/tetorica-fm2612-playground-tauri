#!/bin/sh
set -eu
cd "$(dirname "$0")"

export CI=true
export APPLE_SIGNING_IDENTITY="Developer ID Application: KIYOHIRO KAWAMURA (5H7KW7PC7C)"
export APPLE_ID="kyorohiro@gmail.com"
#export APPLE_PASSWORD="<password>"
export APPLE_TEAM_ID="5H7KW7PC7C"

exec node scripts/deploy_mac.mjs "$@"
