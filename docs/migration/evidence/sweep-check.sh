#!/opt/homebrew/bin/bash

# Deterministic, presence-only migration evidence checker.
#
# Usage:
#   sweep-check.sh [semantic-contract] [ownership] [closure-map] [expected-id-set]
#
# The expected-id-set is one SC id per line.  When omitted, the closure map's
# canonical mapping ids are used as the expected set.  The checker deliberately
# does not try to judge the quality of evidence, only the presence and shape of
# the fields it can identify.

set -u

script_path=${BASH_SOURCE[0]}
script_dir=${script_path%/*}
if [ "$script_dir" = "$script_path" ]; then
  script_dir=.
fi
repo_root=$script_dir/../../..

semantic_file=${1:-$repo_root/docs/migration/semantic-contract.md}
ownership_file=${2:-$repo_root/docs/migration/ownership.md}
closure_file=${3:-$repo_root/docs/migration/evidence/closure-map.md}
expected_file=${4:-$closure_file}

if [ "$#" -gt 4 ]; then
  echo "usage: $0 [semantic-contract] [ownership] [closure-map] [expected-id-set]" >&2
  exit 2
fi

for input_file in "$semantic_file" "$ownership_file" "$closure_file" "$expected_file"; do
  if [ ! -r "$input_file" ]; then
    echo "ERROR: unreadable input: $input_file" >&2
    exit 2
  fi
done

# awk is the only parser: this keeps row/block boundaries deterministic and
# avoids depending on non-portable sort, grep, or temporary-file behavior.
awk \
  -v semantic_file="$semantic_file" \
  -v ownership_file="$ownership_file" \
  -v closure_file="$closure_file" \
  -v expected_file="$expected_file" \
  '
function row_head(line, t) {
  if (line ~ /^[[:space:]]*-[[:space:]]+\*\*SC-/) {
    t = line
    sub(/^[[:space:]]*-[[:space:]]+\*\*SC-/, "SC-", t)
    sub(/\*\*.*/, "", t)
    sub(/[[:space:]]+—.*/, "", t)
    sub(/[[:space:]].*/, "", t)
    return t
  }
  if (line ~ /^[[:space:]]*\*\*SC-/) {
    t = line
    sub(/^[[:space:]]*\*\*SC-/, "SC-", t)
    sub(/\*\*.*/, "", t)
    sub(/[[:space:]]+—.*/, "", t)
    sub(/[[:space:]].*/, "", t)
    return t
  }
  return ""
}

function looks_like_row_head(line) {
  return line ~ /^[[:space:]]*\*\*SC-/ || \
         line ~ /^[[:space:]]*-[[:space:]]+\*\*SC-/
}

function process_semantic_line(line,    rest, next_part, prefix, segment, candidate, next_head) {
  # Most rows occupy a paragraph, but S15 deliberately packs several bold row
  # heads onto one physical line.  Split at every bold **SC- head so none are
  # silently lost.  Unbolded SC- references remain prose, not row heads.
  rest = line
  while (match(rest, /\*\*SC-/)) {
    prefix = substr(rest, 1, RSTART - 1)
    if (row_id != "")
      row_text = row_text " " prefix

    rest = substr(rest, RSTART)
    next_part = substr(rest, 3)
    next_head = match(next_part, /\*\*SC-/)
    if (next_head) {
      segment = substr(rest, 1, next_head + 1)
      rest = substr(rest, next_head + 2)
    } else {
      segment = rest
      rest = ""
    }

    candidate = row_head(segment)
    if (!valid_sc_id(candidate)) {
      print "MALFORMED-ROW-HEAD: " FNR " " segment
      malformed_count++
      finish_row()
      row_id = ""
      row_text = ""
      continue
    }

    finish_row()
    row_id = candidate
    row_text = segment
    row_family = current_family
    if (seen_ids[row_id]) {
      duplicate_count++
      print "DUPLICATE-ID: " row_id " (occurrence " (seen_ids[row_id] + 1) ")"
    }
    seen_ids[row_id]++
    if (row_id ~ /[\/,]|\.\./) {
      grain_count++
      print "GRAIN-VIOLATION: " row_id " combined/shorthand id in row head"
    }
  }

  if (row_id != "" && rest != "")
    row_text = row_text " " rest
}

function add_surface_item(family, item, key) {
  sub(/[[:space:]].*$/, "", item)
  if (item !~ /^[A-Za-z_][A-Za-z0-9_-]*$/)
    return
  key = family SUBSEP item
  if (!surface_seen[key]) {
    surface_seen[key] = 1
    surface_count++
    surface_families[surface_count] = family
    surface_items[surface_count] = item
  }
}

function parse_surface_header(family, line, rest, token, count, i) {
  if (family == "S3") {
    rest = line
    while (match(rest, /`[^`]+`/)) {
      token = substr(rest, RSTART + 1, RLENGTH - 2)
      gsub(/\//, " ", token)
      if (token !~ /\*/) {
        count = split(token, surface_parts, /[[:space:],]+/)
        for (i = 1; i <= count; i++)
          add_surface_item(family, surface_parts[i])
      }
      rest = substr(rest, RSTART + RLENGTH)
    }
  } else if (family == "S15") {
    rest = line
    while (match(rest, /AE_[A-Z0-9_]*\*?/)) {
      token = substr(rest, RSTART, RLENGTH)
      if (token !~ /\*/)
        add_surface_item(family, token)
      rest = substr(rest, RSTART + RLENGTH)
    }
  } else if (family == "S1") {
    rest = line
    while (match(rest, /cmd_[A-Za-z0-9_]*\*?/)) {
      token = substr(rest, RSTART, RLENGTH)
      if (token !~ /\*/)
        add_surface_item(family, token)
      rest = substr(rest, RSTART + RLENGTH)
    }
  }
}

function d_head(line, t) {
  if (line ~ /^[[:space:]]*###[[:space:]]+D[0-9]/) {
    t = line
    sub(/^[[:space:]]*###[[:space:]]+/, "", t)
    sub(/[[:space:]]+—.*/, "", t)
    sub(/[[:space:]].*/, "", t)
    return t
  }
  if (line ~ /^[[:space:]]*\*\*D[0-9]/) {
    t = line
    sub(/^[[:space:]]*\*\*/, "", t)
    sub(/\*\*.*/, "", t)
    sub(/[[:space:]]+—.*/, "", t)
    sub(/[[:space:]].*/, "", t)
    return t
  }
  return ""
}

function valid_sc_id(id) {
  # Slash/comma/range forms are intentionally accepted as one token so that a
  # combined or shorthand row can be reported as a grain violation, never expanded.
  return id ~ /^SC-[0-9][0-9]*[A-Za-z0-9\/,\.]*$/
}

function exact_sc_token(line, id, escaped) {
  escaped = id
  gsub(/[][\\.^$*+?(){}|]/, "\\\\&", escaped)
  return line ~ ("(^|[^A-Za-z0-9])" escaped "([^A-Za-z0-9]|$)")
}

function has_field(text, field) {
  if (field == "bucket")
    return text ~ /[Bb]ucket[[:space:]]+[1-4]([^0-9]|$)/ || \
           text ~ /(^|[^A-Za-z0-9])b[1-4]([^A-Za-z0-9]|$)/ || \
           text ~ /[Bb]ucket-[1-4]-[Dd][Rr]([^A-Za-z0-9]|$)/
  if (field == "authority")
    return text ~ /[Aa]uthority[[:space:]]*[:=]/
  if (field == "empirical")
    return text ~ /[Ee]mpirical[[:space:]]*[:=]/
  if (field == "conflict")
    return text ~ /[Cc]onflict[[:space:]]*[:=]/
  return 0
}

function finish_row(    field) {
  if (row_id == "")
    return
  row_count++
  row_ids[row_count] = row_id
  row_blocks[row_count] = row_text
  row_families[row_count] = row_family

  if (!has_field(row_text, "bucket")) {
    print "MISSING-FIELD: " row_id " bucket"
    missing_count++
  }
  if (!has_field(row_text, "authority")) {
    print "MISSING-FIELD: " row_id " authority"
    missing_count++
  }
  if (!has_field(row_text, "empirical")) {
    print "MISSING-FIELD: " row_id " empirical"
    missing_count++
  }
  if (!has_field(row_text, "conflict")) {
    print "MISSING-FIELD: " row_id " conflict"
    missing_count++
  }
}

function finish_d(    field) {
  if (d_id == "")
    return
  d_count++
  if (d_text !~ /[Ee]ffects([^:]*):/) {
    print "MISSING-D-FIELD: " d_id " effects"
    missing_d_count++
  }
  if (d_text !~ /[Cc]urrent[[:space:]]+writer[[:space:]]*\/[[:space:]]*call[[:space:]]+path[[:space:]]*:/) {
    print "MISSING-D-FIELD: " d_id " current writer/call path"
    missing_d_count++
  }
  if (d_text !~ /[Ll]ocks([^:]*):/) {
    print "MISSING-D-FIELD: " d_id " locks"
    missing_d_count++
  }
  if (d_text !~ /[Aa]tomicity([^:]*):/) {
    print "MISSING-D-FIELD: " d_id " atomicity"
    missing_d_count++
  }
  if (d_text !~ /[Cc]urrent[[:space:]]+owner[[:space:]]*:/) {
    print "MISSING-D-FIELD: " d_id " current owner"
    missing_d_count++
  }
  if (d_text !~ /[Pp]lanned[[:space:]]+owner\/[[:space:]]*fate[[:space:]]*:/) {
    print "MISSING-D-FIELD: " d_id " planned owner/fate"
    missing_d_count++
  }
}

function classified_for(id,    i, line, rest, token, start, finish, n, suffix) {
  n = id
  sub(/^SC-/, "", n)
  sub(/[A-Za-z\/].*$/, "", n)
  n += 0
  suffix = id
  sub(/^SC-[0-9][0-9]*/, "", suffix)

  for (i = 1; i <= classified_count; i++) {
    line = classified_lines[i]
    # An explicit exception applies to the exact id named by the statement.
    if (line ~ /[Ee][Xx][Cc][Ee][Pp][Tt]/) {
      rest = line
      sub(/^.*[Ee][Xx][Cc][Ee][Pp][Tt]/, "", rest)
      if (exact_sc_token(rest, id))
        continue
    }

    rest = line
    while (match(rest, /SC-[0-9][0-9]*\.\.[0-9][0-9]*/)) {
      token = substr(rest, RSTART, RLENGTH)
      start = token
      sub(/^SC-/, "", start)
      sub(/\.\..*$/, "", start)
      finish = token
      sub(/^.*\.\./, "", finish)
      if (n >= (start + 0) && n <= (finish + 0)) {
        if (suffix == "" || line ~ /[Ll]etter-splits|[Aa]ll splits|[Aa]ll their letter-splits/)
          return 1
      }
      rest = substr(rest, RSTART + RLENGTH)
    }

    # Exact ids are also used for one-off statements and for range endpoints.
    rest = line
    while (match(rest, /SC-[0-9][0-9]*[A-Za-z0-9\/,\.]*/)) {
      token = substr(rest, RSTART, RLENGTH)
      if (token == id)
        return 1
      rest = substr(rest, RSTART + RLENGTH)
    }
  }
  return 0
}

function id_number(id, n) {
  n = id
  sub(/^SC-/, "", n)
  sub(/[A-Za-z\/].*$/, "", n)
  return n + 0
}

function id_less(a, b, na, nb) {
  na = id_number(a)
  nb = id_number(b)
  if (na != nb)
    return na < nb
  return a < b
}

function add_expected_token(token) {
  sub(/[[:space:](].*$/, "", token)
  if (valid_sc_id(token))
    expected_ids[token] = 1
}

function add_expected_line(line, token) {
  while (match(line, /SC-[0-9][0-9]*[A-Za-z0-9\/,\.]*/)) {
    token = substr(line, RSTART, RLENGTH)
    add_expected_token(token)
    line = substr(line, RSTART + RLENGTH)
  }
}

function surface_present(family, item, i) {
  for (i = 1; i <= row_count; i++) {
    if (row_families[i] == family && row_blocks[i] ~ ("(^|[^A-Za-z0-9_-])" item "([^A-Za-z0-9_-]|$)"))
      return 1
  }
  return 0
}

FILENAME == semantic_file {
  if ($0 ~ /^### S1[[:space:]]/) {
    current_family = "S1"
    surface_header_active = 1
  } else if ($0 ~ /^### S3[[:space:]]/) {
    current_family = "S3"
    surface_header_active = 1
  } else if ($0 ~ /^### S15[[:space:]]/) {
    current_family = "S15"
    surface_header_active = 1
  } else if ($0 ~ /^### /) {
    current_family = ""
    surface_header_active = 0
  }
  if (surface_header_active && $0 !~ /<!-- rows:/)
    parse_surface_header(current_family, $0)
  if ($0 ~ /<!-- rows:/)
    surface_header_active = 0

  if ($0 ~ /^[[:space:]]*(\*\*)?[Cc]lassified_by[[:space:]]*:/) {
    classified_count++
    classified_lines[classified_count] = $0
  }

  process_semantic_line($0)
  next
}

FILENAME == ownership_file {
  candidate = d_head($0)
  if (candidate != "") {
    finish_d()
    d_id = candidate
    d_text = $0
    next
  }
  if (d_id != "")
    d_text = d_text " " $0
  next
}

FILENAME == closure_file {
  # Mapping entries begin with their canonical id.  Body prose is not map data.
  if ($0 ~ /^SC-[0-9]/) {
    candidate = $1
    sub(/\(.*/, "", candidate)
    if (valid_sc_id(candidate)) {
      closure_ids[candidate] = 1
      if (expected_file == closure_file)
        expected_ids[candidate] = 1
    }
  }
  next
}

FILENAME == expected_file {
  # Explicit expected sets may contain one id per line or whitespace-separated
  # ids.  The closure-map mapping-entry parser above remains stricter.
  if (expected_file != closure_file)
    add_expected_line($0)
  next
}

END {
  finish_row()
  finish_d()

  for (i = 1; i <= row_count; i++) {
    id = row_ids[i]
    if (!classified_for(id)) {
      print "MISSING-FIELD: " id " classified_by"
      missing_count++
    }
  }

  for (i = 1; i <= surface_count; i++) {
    if (!surface_present(surface_families[i], surface_items[i])) {
      print "MISSING-SURFACE: " surface_families[i] " " surface_items[i] " (no row mentions enumerated item)"
      missing_surface_count++
    }
  }

  # Build one natural-sorted universe first so every set-difference report is
  # stable across awk hash-table iteration orders.
  for (id in seen_ids)
    universe_ids[id] = 1
  for (id in closure_ids)
    universe_ids[id] = 1
  for (id in expected_ids)
    universe_ids[id] = 1
  universe_count = 0
  for (id in universe_ids) {
    universe_count++
    universe_sorted[universe_count] = id
  }
  for (i = 1; i <= universe_count; i++) {
    for (j = i + 1; j <= universe_count; j++) {
      if (id_less(universe_sorted[j], universe_sorted[i])) {
        id = universe_sorted[i]
        universe_sorted[i] = universe_sorted[j]
        universe_sorted[j] = id
      }
    }
  }
  for (i = 1; i <= universe_count; i++) {
    id = universe_sorted[i]
    if ((id in closure_ids) && !(id in seen_ids)) {
      print "CLOSURE-MAP-ORPHAN: " id
      closure_orphan_count++
    }
    if ((id in seen_ids) && !(id in closure_ids)) {
      print "CLOSURE-MAP-MISSING: " id
      closure_missing_count++
    }
    if ((id in seen_ids) && !(id in expected_ids)) {
      print "SET-DIFF extra-in-contract: " id
      expected_extra_count++
    }
    if ((id in expected_ids) && !(id in seen_ids)) {
      print "SET-DIFF missing-from-contract: " id
      expected_missing_count++
    }
  }

  sorted_count = 0
  for (id in seen_ids) {
    sorted_count++
    sorted_ids[sorted_count] = id
  }
  for (i = 1; i <= sorted_count; i++) {
    for (j = i + 1; j <= sorted_count; j++) {
      if (id_less(sorted_ids[j], sorted_ids[i])) {
        id = sorted_ids[i]
        sorted_ids[i] = sorted_ids[j]
        sorted_ids[j] = id
      }
    }
  }

  print "NOTE: presence checks cannot certify evidence FIDELITY; the separate pin audit must verify evidence fidelity."
  print "NOTE: line/block parsing may misparse prose-form or wrapped fields; implicit family authority and complex classified_by prose are not inferred."
  print "NOTE: surface coverage only sees list-shaped S1/S3/S15 headers and token mentions; it cannot certify omitted, renamed, or prose-only surfaces."
  printf "SUMMARY: SC_ROWS=%d D_RECORDS=%d MISSING_FIELDS=%d MISSING_D_FIELDS=%d MISSING_SURFACES=%d GRAIN_VIOLATIONS=%d DUPLICATE_IDS=%d MALFORMED_ROW_HEADS=%d CLOSURE_ORPHANS=%d CLOSURE_MISSING=%d SET_EXTRA=%d SET_MISSING=%d\n", \
    row_count, d_count, missing_count, missing_d_count, missing_surface_count, grain_count, duplicate_count, malformed_count, \
    closure_orphan_count, closure_missing_count, expected_extra_count, expected_missing_count
  printf "SUMMARY SC_IDS:"
  if (sorted_count == 0)
    printf " (none)"
  for (i = 1; i <= sorted_count; i++)
    printf " %s", sorted_ids[i]
  printf "\n"

  if (missing_count || missing_d_count || missing_surface_count || grain_count || duplicate_count || malformed_count || \
      closure_orphan_count || closure_missing_count || expected_extra_count || expected_missing_count)
    exit 1
  exit 0
}
' "$semantic_file" "$ownership_file" "$closure_file" "$expected_file"
