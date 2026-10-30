#!/bin/bash
# helpers: S.sh eval 'js'  |  S.sh shot NAME  |  S.sh ext enable|disable UUID  | S.sh info UUID
H=$(cd "$(dirname "$0")" && pwd)
R="$H/isolated-session.sh run"
case "$1" in
eval) $R python3 "$H/client/shell_eval.py" "$2" "${3:-}" ;;
shot) $R gdbus call --session -d org.gnome.Shell.Screenshot -o /org/gnome/Shell/Screenshot -m org.gnome.Shell.Screenshot.Screenshot false false "/tmp/omk-gs-spike/$2.png" ;;
ext) $R gdbus call --session -d org.gnome.Shell.Extensions -o /org/gnome/Shell/Extensions -m org.gnome.Shell.Extensions.${2^}Extension "$3" ;;
info) $R gdbus call --session -d org.gnome.Shell.Extensions -o /org/gnome/Shell/Extensions -m org.gnome.Shell.Extensions.GetExtensionInfo "$2" ;;
esac
