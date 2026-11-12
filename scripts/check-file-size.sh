#!/usr/bin/env bash
# Fails when a source file is longer than its limit. Clippy has no file-length lint.
set -euo pipefail
cd "$(dirname "$0")/.."

limit_for() {
  case "$1" in
    */tests/*|*_test.rs|*.spec.ts) echo 600 ;;
    *.rs) echo 400 ;;
    *.ts) echo 300 ;;
    *.html|*.scss|*.css) echo 200 ;;
    *.sql) echo 300 ;;
  esac
}

failed=0
while IFS= read -r file; do
  limit=$(limit_for "$file")
  lines=$(wc -l <"$file")
  if [ "$lines" -gt "$limit" ]; then
    echo "$file: $lines lines, limit $limit"
    failed=1
  fi
done < <(find crates web/src migrations -type f \
  \( -name '*.rs' -o -name '*.ts' -o -name '*.html' -o -name '*.scss' -o -name '*.css' -o -name '*.sql' \) \
  -not -path '*/node_modules/*' -not -path '*/target/*' -not -path '*/fixtures/*' 2>/dev/null)

exit "$failed"
