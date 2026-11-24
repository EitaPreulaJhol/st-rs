#!/bin/sh
# Print all font styles and keep reporting the terminal size so the harness can
# observe runtime zoom with custom per-style fonts.
# Usage: font_test.sh DIR TAG

dir=$1
tag=$2

echo "$WINDOWID" > "$dir/wid_font_$tag"
stty raw -echo
stty size > "$dir/fontsize_$tag"

# Steady block cursor: screenshots must not depend on the blink phase.
printf '\033[2 q'

printf 'normal text\r\n'
printf '\033[1mBOLD text\033[0m\r\n'
printf '\033[3mITALIC text\033[0m\r\n'
printf '\033[1;3mBOLD ITALIC text\033[0m\r\n'
printf '\033[2mFAINT text\033[0m\r\n'
printf '\033[4mUNDERLINE text\033[0m\r\n'

i=0
while [ "$i" -lt 200 ]; do
	stty size >> "$dir/fontsize_$tag"
	sleep 0.1
	i=$((i + 1))
done
