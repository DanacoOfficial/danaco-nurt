process() {
  :
}

usage() {
  case "$1" in
    -h) echo "help" ;;
  esac
}

for file in *; do
  process "$file"
done
