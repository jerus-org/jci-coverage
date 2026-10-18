set -- jci-coverage complexity
[[ "${GCO_FORMAT:-false}" = "true" ]] && set -- "$@" --format
[[ ! "${GCO_FILE:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --file "${GCO_FILE}"
case "${GCO_LOG_LEVEL:-default}" in
  quiet) set -- "$@" --quiet ;;
  v) set -- "$@" --verbose ;;
  vv) set -- "$@" --verbose --verbose ;;
  vvv) set -- "$@" --verbose --verbose --verbose ;;
  vvvv) set -- "$@" --verbose --verbose --verbose --verbose ;;
esac
[[ ! "${GCO_OUTPUT:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --output "${GCO_OUTPUT}"
[[ ! "${GCO_SOURCE_ROOT:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --source-root "${GCO_SOURCE_ROOT}"
[[ ! "${GCO_DUPLICATES:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --duplicates "${GCO_DUPLICATES}"
[[ ! "${GCO_RULES:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --rules "${GCO_RULES}"
[[ ! "${GCO_UNMATCHED:-}" =~ ^[[:space:]]*$ ]] && set -- "$@" --unmatched "${GCO_UNMATCHED}"
[[ "${GCO_NO_ROLLUP:-false}" = "true" ]] && set -- "$@" --no-rollup
[[ "${GCO_EXCLUDE_TESTS:-false}" = "true" ]] && set -- "$@" --exclude-tests
[[ "${GCO_DRY_RUN:-false}" = "true" ]] && set -- "$@" --dry-run
"$@"
