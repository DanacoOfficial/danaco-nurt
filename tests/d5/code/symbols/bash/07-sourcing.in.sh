source ./lib.sh
. ./other.sh

run_all() {
  for task in "$@"; do
    "$task"
  done
}
