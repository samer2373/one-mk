#!/bin/bash
set -euo pipefail

if [ "${1:-}" != --namespace ]; then
    repository=$(cd "$(dirname "$0")/.." && pwd)
    audit="$repository/.audit-wave6"
    mkdir -p "$audit/roots" "$audit/target"
    root=$(mktemp -d "$audit/roots/root.XXXXXX")
    trap 'rm -rf -- "$root"' EXIT
    unshare -U --map-current-user --keep-caps -mnpf bash "$0" --namespace "$root" "$repository" "$HOME" "$@"
    exit
fi

shift
root=$1
repository=$2
toolhome=$3
shift 3
mount --make-rprivate /
for directory in usr etc; do
    mkdir -p "$root/$directory"
    mount --bind "/$directory" "$root/$directory"
    mount -o remount,bind,ro "$root/$directory"
done
for directory in bin sbin lib lib64; do
    if [ -L "/$directory" ]; then ln -s "$(readlink "/$directory")" "$root/$directory"; fi
done
for directory in "$toolhome/.cargo" "$toolhome/.rustup" "$repository"; do
    mkdir -p "$root$directory"
    mount --bind "$directory" "$root$directory"
done
mkdir -p "$root/proc" "$root/dev" "$root/tmp" "$root/h" "$root/r" "$root/audit-bin"
chmod 700 "$root/tmp" "$root/h" "$root/r"
mount -t proc proc "$root/proc"
for device in null zero random urandom full; do
    touch "$root/dev/$device"
    mount --bind "/dev/$device" "$root/dev/$device"
done
mkdir -p "$root/dev/shm" "$root/dev/pts"
chmod 1777 "$root/dev/shm"
mount -t devpts devpts "$root/dev/pts" -o newinstance,ptmxmode=0666,mode=0620
ln -s pts/ptmx "$root/dev/ptmx"
ln -s /proc/self/fd "$root/dev/fd"
for descriptor in stdin:0 stdout:1 stderr:2; do
    ln -s "/proc/self/fd/${descriptor#*:}" "$root/dev/${descriptor%:*}"
done
cat > "$root/audit-bin/Xvfb" <<'WRAPPER'
#!/bin/sh
export __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json
export LIBGL_ALWAYS_SOFTWARE=1
exec /usr/bin/Xvfb ":$((10000 + $$))" "$@"
WRAPPER
chmod 755 "$root/audit-bin/Xvfb"
ip link set lo up
ip link add audit0 type dummy
ip addr add 192.0.2.10/24 dev audit0
ip link set audit0 up
ip route add default dev audit0
exec chroot "$root" /usr/bin/setpriv --bounding-set=-all --inh-caps=-all --ambient-caps=-all /usr/bin/env -i \
    PATH="/audit-bin:$toolhome/.cargo/bin:/usr/local/bin:/usr/sbin:/usr/bin:/bin" \
    HOME=/h XDG_RUNTIME_DIR=/r ONE_MK_HOME=/h/mk TMPDIR=/tmp \
    CARGO_HOME="$toolhome/.cargo" RUSTUP_HOME="$toolhome/.rustup" \
    CARGO_TARGET_DIR="$repository/.audit-wave6/target" CARGO_BUILD_JOBS=2 ONE_MK_ISOLATED_AUDIT=1 \
    /bin/bash -c 'cd "$1"; shift; exec "$@"' audit "$repository" "$@"
