set -- jci-coverage upload
set -- "$@" --file "${GCO_FILE}"
case "${GCO_LOG_LEVEL:-default}" in
  quiet) set -- "$@" --quiet ;;
  v) set -- "$@" --verbose ;;
  vv) set -- "$@" --verbose --verbose ;;
  vvv) set -- "$@" --verbose --verbose --verbose ;;
  vvvv) set -- "$@" --verbose --verbose --verbose --verbose ;;
esac
[[ ! "${GCO_REPO_TOKEN:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --repo-token "${GCO_REPO_TOKEN}"
[[ ! "${GCO_ENDPOINT:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --endpoint "${GCO_ENDPOINT}"
"$@"
