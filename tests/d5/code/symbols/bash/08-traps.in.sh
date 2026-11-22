cleanup() {
  rm -f "$tmp"
}

trap cleanup EXIT
tmp=$(mktemp)
