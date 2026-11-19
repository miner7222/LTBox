#!/usr/bin/env bash
# Package a native CI binary and verify installation and ELF dependencies.
set -euo pipefail

binary="$(realpath "$1")"
version="$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')"
case "$(uname -m)" in
    x86_64) arch=amd64 ;;
    aarch64) arch=arm64 ;;
    *) echo "Unsupported native architecture" >&2; exit 1 ;;
esac
mkdir -p packaging-stage test-packages
install -m 0755 "${binary}" packaging-stage/ltbox
sed 's|__LTBOX_EXEC__|/usr/bin/ltbox|g' \
    misc/desktop/io.github.miner7222.LTBox.desktop \
    > packaging-stage/io.github.miner7222.LTBox.desktop
for format in deb rpm; do
    printf '%s\n' "${format}" > packaging-stage/ltbox.install-source
    docker run --rm -v "${PWD}:/work" -w /work \
        -e NFPM_ARCH="${arch}" -e NFPM_VERSION="${version}" \
        -e NFPM_RPM_SIGNING_KEY_FILE= -e NFPM_RPM_SIGNING_KEY_ID= \
        ghcr.io/goreleaser/nfpm:v2.47.0 package \
        --config packaging/linux/nfpm.yaml --packager "${format}" \
        --target "/work/test-packages/ltbox.${format}"
done

# ldd -r checks symbol versions and relocations without starting LTBox.
for distro in ubuntu:24.04 debian:13-slim fedora:latest; do
    docker run --rm -v "${PWD}/test-packages:/packages:ro" "${distro}" sh -ceu '
        if command -v apt-get >/dev/null; then
            apt-get -o Acquire::Retries=5 update
            DEBIAN_FRONTEND=noninteractive apt-get -o Acquire::Retries=5 install -y /packages/ltbox.deb
            test "$(cat /usr/share/ltbox/ltbox.install-source)" = deb
        else
            dnf install -y /packages/ltbox.rpm
            test "$(cat /usr/share/ltbox/ltbox.install-source)" = rpm
        fi
        test -x /usr/bin/ltbox
        ldd -r /usr/bin/ltbox > /tmp/ltbox-links 2>&1
        cat /tmp/ltbox-links
        if grep -E "not found|undefined symbol" /tmp/ltbox-links; then
            exit 1
        fi
    '
done
