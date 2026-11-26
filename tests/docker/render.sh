#!/bin/sh
# Render a deterministic set of attributes for the rendering smoke test.
# Usage: render.sh DIR TAG [SECONDS]

dir=$1
tag=$2
delay=${3:-4}

echo "$WINDOWID" > "$dir/wid_$tag"
stty size > "$dir/size_$tag"

# Steady block cursor: the blink phase must not influence screenshots.
printf '\033[2 q'

# Normalise the palette so the screenshot shows terminal behaviour rather
# than the configured colour scheme.
colors='383a42 e45649 50a14f c18401 0184bc a626a4 0997b3 a0a1a7
4f525e e06c75 98c379 e5c07b 61afef c678dd 56b6c2 ffffff'
i=0
for color in $colors; do
	printf '\033]4;%d;#%s\007' "$i" "$color"
	i=$((i + 1))
done
printf '\033]10;#383a42\007\033]11;#ffffff\007\033]12;#383a42\007'
printf '\033]4;256;#383a42\007\033]4;257;#d0d0d0\007'

printf 'normal text\r\n'
printf '\033[1mBOLD\033[0m \033[3mITALIC\033[0m \033[4mUNDER\033[0m \033[9mSTRUCK\033[0m \033[7mREVERSE\033[0m \033[8mINVISIBLE\033[0m\r\n'
printf '\033[41;37mwhite on red\033[0m \033[44;93myellow on blue\033[0m\r\n'
printf '256: \033[38;5;196mred\033[0m \033[48;5;21mblue-bg\033[0m \033[38;5;46mgreen\033[0m\r\n'
printf 'true: \033[38;2;255;128;0morange\033[0m \033[48;2;20;40;60mdark-bg\033[0m\r\n'
printf 'box: \342\224\214\342\224\200\342\224\220 \342\224\202 wide: \344\270\226\347\225\214\r\n'
printf 'ul: \033[4:2mdouble\033[0m \033[4:3;58:2::228:86:73mcurly\033[0m \033[4:4mdotted\033[0m \033[4:5;58;5;4mdashed\033[0m\r\n'
printf 'frame: \342\225\224\342\225\220\342\225\246\342\225\220\342\225\227 \342\226\221\342\226\222\342\226\223\342\226\210 \342\224\217\342\224\201\342\224\223\r\n'
printf 'link: \033]8;;https://example.invalid/\033\\anchor\033]8;;\033\\ \033[>4;1m\033[?2026h\033[?2026l\r\n'
printf '\033[?5hREVERSE VIDEO MODE\r\n'
printf '\033[10;1H'

sleep "$delay"
