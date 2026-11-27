#!/usr/bin/env bash
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Prepares the file-backed volumes that the ignored volume tests of the
# repository mount (tests/d2/volumes, tests/d2/retention and the volume tests
# of crates/nurt-fs) and writes a file of "export" lines naming them.
#
# Volumes, one image file each unless stated otherwise:
#   NURT_TEST_EXT4_DIR           ext4, 1 GiB
#   NURT_TEST_EXT4_SMALL_DIR     ext4, 64 MiB
#   NURT_TEST_XFS_DIR            XFS, 512 MiB
#   NURT_TEST_BTRFS_DIR          btrfs, 512 MiB
#   NURT_TEST_REFLINK_DIR        the btrfs volume (it clones blocks)
#   NURT_TEST_QUOTA_DIR          ext4, 128 MiB, mounted with user quota
#   NURT_TEST_OVERLAY_DIR        overlay, upper layer on ext4 of 256 MiB, directory
#                                "lower" of 8 MiB from the lower layer
#   NURT_TEST_OVERLAY_SMALL_DIR  overlay, upper layer on ext4 of 48 MiB (about 38 MiB
#                                free), directory "lower" of 48 MiB from the lower
#                                layer, so that a copy of "lower" does not fit
#   NURT_TEST_TMPFS_DIR          tmpfs, 64 MiB
#
# Usage: tools/ci/prepare-test-volumes.sh up <work-dir>
#        tools/ci/prepare-test-volumes.sh down <work-dir>
#
# "up" creates <work-dir>/images and <work-dir>/mnt, mounts the volumes, gives
# them to the invoking user and writes <work-dir>/volumes.env; load it with
# ". <work-dir>/volumes.env". "down" unmounts the volumes, detaches the loop
# devices and removes <work-dir>/images, <work-dir>/mnt and <work-dir>/volumes.env.
# The work directory must be on a disk with about 2.5 GiB free; it must not be
# on tmpfs because the images are sparse files that grow while tests run.
#
# Requirements: Linux, sudo without a password, mkfs.ext4, mkfs.xfs,
# mkfs.btrfs, losetup, mount with overlayfs, setquota (package "quota").
#
# Exit status: 0 success, 1 failure, 2 usage error.

set -eu

readonly MIB=1048576

usage() {
  echo "Usage: prepare-test-volumes.sh up|down <work-dir>" >&2
  exit 2
}

[ $# -eq 2 ] || usage
action=$1
mkdir -p "$2"
work=$(cd "$2" && pwd)
images=$work/images
mnt=$work/mnt
envfile=$work/volumes.env
owner=$(id -un):$(id -gn)

# Creates a sparse image of $2 MiB at $images/$1.img and prints its loop device.
attach() {
  truncate -s "$(($2 * MIB))" "$images/$1.img"
  sudo losetup --find --show "$images/$1.img"
}

# Formats the loop device $2 with filesystem $3 (extra mkfs arguments follow
# the mount options $4) and mounts it at $mnt/$1.
format_mount() {
  local name=$1 device=$2 fstype=$3 options=$4
  shift 4
  sudo "mkfs.$fstype" -q "$@" "$device" >/dev/null
  mkdir -p "$mnt/$name"
  sudo mount -t "$fstype" -o "$options" "$device" "$mnt/$name"
  sudo chown "$owner" "$mnt/$name"
}

# Mounts an overlay at $mnt/$1 whose upper layer lives on the volume $2 and
# whose lower layer holds a directory "lower" of $3 MiB made of 1 MiB files.
overlay() {
  local name=$1 upper_volume=$2 lower_mib=$3
  mkdir -p "$mnt/$name-lower/lower" "$mnt/$upper_volume/upper" "$mnt/$upper_volume/work" "$mnt/$name"
  local index
  for index in $(seq 1 "$lower_mib"); do
    head -c "$MIB" /dev/urandom >"$mnt/$name-lower/lower/file-$index.bin"
  done
  sudo mount -t overlay overlay \
    -o "lowerdir=$mnt/$name-lower,upperdir=$mnt/$upper_volume/upper,workdir=$mnt/$upper_volume/work" \
    "$mnt/$name"
}

up() {
  if [ -e "$envfile" ]; then
    echo "volumes are already prepared: run 'down' first" >&2
    exit 1
  fi
  mkdir -p "$images" "$mnt"
  local device

  device=$(attach ext4 1024)
  format_mount ext4 "$device" ext4 defaults -F
  device=$(attach ext4-small 64)
  format_mount ext4-small "$device" ext4 defaults -F
  device=$(attach xfs 512)
  format_mount xfs "$device" xfs defaults -f
  device=$(attach btrfs 512)
  format_mount btrfs "$device" btrfs defaults -f
  device=$(attach quota 128)
  format_mount quota "$device" ext4 usrquota -F -O quota -E quotatype=usrquota
  sudo setquota -u "$(id -un)" 0 "$((100 * 1024))" 0 0 "$mnt/quota"

  device=$(attach overlay-upper 256)
  format_mount overlay-upper "$device" ext4 defaults -F
  overlay overlay overlay-upper 8
  device=$(attach overlay-small-upper 48)
  format_mount overlay-small-upper "$device" ext4 defaults -F
  overlay overlay-small overlay-small-upper 48

  mkdir -p "$mnt/tmpfs"
  sudo mount -t tmpfs -o size=64m tmpfs "$mnt/tmpfs"
  sudo chown "$owner" "$mnt/tmpfs"

  {
    echo "export NURT_TEST_EXT4_DIR=$mnt/ext4"
    echo "export NURT_TEST_EXT4_SMALL_DIR=$mnt/ext4-small"
    echo "export NURT_TEST_XFS_DIR=$mnt/xfs"
    echo "export NURT_TEST_BTRFS_DIR=$mnt/btrfs"
    echo "export NURT_TEST_REFLINK_DIR=$mnt/btrfs"
    echo "export NURT_TEST_QUOTA_DIR=$mnt/quota"
    echo "export NURT_TEST_OVERLAY_DIR=$mnt/overlay"
    echo "export NURT_TEST_OVERLAY_SMALL_DIR=$mnt/overlay-small"
    echo "export NURT_TEST_TMPFS_DIR=$mnt/tmpfs"
  } >"$envfile"
  echo "volumes are mounted; load them with: . $envfile"
}

down() {
  local name
  for name in overlay-small overlay tmpfs quota btrfs xfs ext4-small ext4 overlay-small-upper overlay-upper; do
    if mountpoint -q "$mnt/$name"; then
      sudo umount "$mnt/$name"
    fi
  done
  local image
  for image in "$images"/*.img; do
    [ -e "$image" ] || continue
    sudo losetup --associated "$image" | cut -d: -f1 | while read -r device; do
      sudo losetup --detach "$device"
    done
  done
  sudo rm -rf "$images" "$mnt" "$envfile"
}

case $action in
  up) up ;;
  down) down ;;
  *) usage ;;
esac
