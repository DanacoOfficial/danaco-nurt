#!/bin/sh
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           packaging/tests/container-install.sh
# Responsibility:   Installs the packages of a directory in the running system and checks the result.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

# Installs the packages of a directory in the running system and checks the
# result. The script runs inside a clean container of the distribution.
#
# Usage: container-install.sh deb|rpm PACKAGE_DIR

set -eu

[ $# -eq 2 ] || {
    echo "usage: container-install.sh deb|rpm PACKAGE_DIR" >&2
    exit 2
}
kind=$1
dir=$2

case $kind in
    deb)
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq
        apt-get install -y -qq pkg-config
        apt-get install -y -qq "$dir"/libnurt1_*.deb "$dir"/libnurt-dev_*.deb "$dir"/nurt_*.deb
        ;;
    rpm)
        dnf install -y -q pkgconf-pkg-config
        dnf install -y -q "$dir"/libnurt-[0-9]*.rpm "$dir"/libnurt-devel-*.rpm "$dir"/nurt-[0-9]*.rpm
        ;;
    *)
        echo "unknown package kind: $kind" >&2
        exit 2
        ;;
esac

first_line=$(nurt version | head -n 1)
case $first_line in
    "nurt "[0-9]*) ;;
    *)
        echo "unexpected first line of 'nurt version': $first_line" >&2
        exit 1
        ;;
esac

ldconfig -p | grep -q 'libnurt\.so\.1' || {
    echo "libnurt.so.1 is not known to the dynamic linker" >&2
    exit 1
}
pkg-config --exists nurt || {
    echo "pkg-config does not find nurt" >&2
    exit 1
}
test -f /usr/include/nurt.h

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
