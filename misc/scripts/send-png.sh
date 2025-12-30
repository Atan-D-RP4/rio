#!/bin/sh

send_chunked() {
    first="y"
    while IFS= read -r chunk; do
        metadata=""; [ "$first" = "y" ] && { metadata="a=T,f=100,"; first="n"; }
        printf "\033_G%sm=1;%s\033\\" "${metadata}" "${chunk}"
    done
    [ "$first" = "n" ] && { printf "\033_Gm=0;\033\\"; return 0; }
    return 1
}

transmit_png() {
    # Different systems have different or missing base64 executables.
    # The sed command below adds a trailing newline which openssl
    # base64 does not produce and is needed for reading via read -r
    { command base64 -w 4096 "$1" 2>/dev/null | send_chunked; } || \
    { command base64 -b 4096 "$1" 2>/dev/null | send_chunked; } || \
    { command openssl base64 -e -A -in "$1" | command sed '$a\' | command fold -b -w 4096 | send_chunked; }
}

transmit_png "$1"
