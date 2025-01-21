#!/bin/sh
if [ ! -d .git ]; then
    echo "No `.git` dir found. Please run this script from repository root."
    exit 1
fi
name=$(basename `pwd`)
echo "Use 'git clone git://<host>/$name'"
exec git daemon --base-path=.. --export-all