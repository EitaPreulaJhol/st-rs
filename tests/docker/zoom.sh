#!/bin/sh
# Keep reporting the pty size while the harness exercises font zoom.
# Usage: zoom.sh DIR TAG

dir=$1
tag=$2

echo "$WINDOWID" > "$dir/wid_zoom_$tag"
stty raw -echo

i=0
while [ "$i" -lt 300 ]; do
	stty size >> "$dir/zoom_$tag"
	sleep 0.1
	i=$((i + 1))
done
