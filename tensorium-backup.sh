#!/usr/bin/env bash
set -euo pipefail

DATA_DIR="${TENSORIUM_DATA_DIR:-/root/.tensorium}"
BACKUP_DIR="${TENSORIUM_BACKUP_DIR:-/root/backups}"
KEEP_COUNT="${TENSORIUM_BACKUP_KEEP:-14}"
STAMP="$(date -u +%F-%H%M%S)"
ARCHIVE="${BACKUP_DIR}/tensorium-backup-${STAMP}.tgz"

mkdir -p "${BACKUP_DIR}"

declare -a inputs=()
for path in \
  "${DATA_DIR}/state.db" \
  "${DATA_DIR}/tensorium-mc-state.db" \
  "${DATA_DIR}/mempool.json" \
  "${DATA_DIR}/banlist.json" \
  "${DATA_DIR}/tensorium-mc-mempool.json" \
  "${DATA_DIR}/tensorium-mc-banlist.json"
do
  if [[ -e "${path}" ]]; then
    inputs+=("${path}")
  fi
done

shopt -s nullglob
for migrated in "${DATA_DIR}"/*.json.migrated; do
  inputs+=("${migrated}")
done
shopt -u nullglob

if [[ "${#inputs[@]}" -eq 0 ]]; then
  echo "No chain-state artifacts found under ${DATA_DIR}" >&2
  exit 1
fi

tar -czf "${ARCHIVE}" "${inputs[@]}"
echo "Created backup: ${ARCHIVE}"

mapfile -t existing < <(find "${BACKUP_DIR}" -maxdepth 1 -type f -name 'tensorium-backup-*.tgz' | sort)
if (( ${#existing[@]} > KEEP_COUNT )); then
  remove_count=$(( ${#existing[@]} - KEEP_COUNT ))
  for old in "${existing[@]:0:remove_count}"; do
    rm -f "${old}"
    echo "Removed old backup: ${old}"
  done
fi
