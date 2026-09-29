set -- jci-coverage report
[[ ! "${GCO_PACKAGE:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --package "${GCO_PACKAGE}"
case "${GCO_LOG_LEVEL:-default}" in
  quiet) set -- "$@" --quiet ;;
  v) set -- "$@" --verbose ;;
  vv) set -- "$@" --verbose --verbose ;;
  vvv) set -- "$@" --verbose --verbose --verbose ;;
  vvvv) set -- "$@" --verbose --verbose --verbose --verbose ;;
esac
[[ ! "${GCO_RUNNER:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --runner "${GCO_RUNNER}"
[[ ! "${GCO_NEXTEST_PROFILE:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --nextest-profile "${GCO_NEXTEST_PROFILE}"
[[ "${GCO_NO_ALL_FEATURES:-false}" = "true" ]] && set -- "$@" --no-all-features
[[ "${GCO_DRY_RUN:-false}" = "true" ]] && set -- "$@" --dry-run
"$@"
