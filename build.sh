#!/bin/sh

set -e

INPUT_VIDEO="$HOME/badapple.mp4"

#OUTPUT_WIDTH=48
#OUTPUT_HEIGHT=36
#OUTPUT_FRAMERATE=30

OUTPUT_WIDTH=24
OUTPUT_HEIGHT=18
OUTPUT_FRAMERATE=10

LW_SAVES="$HOME/.local/share/Steam/steamapps/common/Logic World/saves"
TEMPLATE="$LW_SAVES/bad apple"

SAVE_NAME="bad apple ${OUTPUT_WIDTH}x${OUTPUT_HEIGHT} ${OUTPUT_FRAMERATE}fps"
SAVE_FILE="$LW_SAVES/$SAVE_NAME"

[ -e "$SAVE_FILE" ] && echo "$SAVE_FILE" && rm -rI "$SAVE_FILE"
cp -r "$TEMPLATE" "$SAVE_FILE"

cat >"$SAVE_FILE/meta.succ" <<EOF
Title: $SAVE_NAME
Description:
Tags:
EOF

rm -rf frames
mkdir frames

ffmpeg -i "$INPUT_VIDEO" \
    -vf "fps=${OUTPUT_FRAMERATE},scale=${OUTPUT_WIDTH}:${OUTPUT_HEIGHT}" \
    frames/%05d.png

cargo r -- "$SAVE_FILE/data.logicworld"
