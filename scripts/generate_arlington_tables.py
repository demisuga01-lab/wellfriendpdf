#!/usr/bin/env python3
"""Generate Oxide's compact Arlington validation tables from upstream TSVs.

This is a developer-run generator. Runtime validation consumes the generated
Rust tables and never parses Arlington TSV files on the hot parser path.
"""

from __future__ import annotations

import argparse
import csv
import json
import re
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Iterable


TYPE_MAP = {
    "any": "Any",
    "array": "Array",
    "bitmask": "Integer",
    "boolean": "Boolean",
    "date": "String",
    "dictionary": "Dictionary",
    "integer": "Integer",
    "matrix": "Array",
    "name": "Name",
    "name-tree": "Dictionary",
    "null": "Any",
    "number": "Number",
    "number-tree": "Dictionary",
    "rectangle": "Array",
    "stream": "Stream",
    "string": "String",
    "string-ascii": "String",
    "string-byte": "String",
    "string-text": "String",
}


PREDICATE_RE = re.compile(r"fn:[A-Za-z0-9_]+")
NAME_VALUE_RE = re.compile(r"@[A-Za-z0-9_.:-]+|\b[A-Z][A-Za-z0-9_.:-]*\b")


@dataclass
class Stats:
    source: str
    commit: str
    tsv_files: int = 0
    object_models: int = 0
    keys: int = 0
    required_key_rules: int = 0
    type_rules: int = 0
    version_rules: int = 0
    indirect_reference_rules: int = 0
    link_rules: int = 0
    unsupported_predicates: int = 0
    parse_warnings: int = 0
    output: str = ""


def rust_str(value: str) -> str:
    return json.dumps(value, ensure_ascii=True)


def read_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8-sig", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        rows: list[dict[str, str]] = []
        for row in reader:
            normalized: dict[str, str] = {}
            for key, value in row.items():
                if key is None:
                    continue
                normalized[key] = (value or "").strip()
            rows.append(normalized)
        return rows


def split_semicolon(value: str) -> list[str]:
    return [part.strip() for part in value.split(";") if part.strip()]


def normalize_types(type_expr: str) -> tuple[list[str], list[str]]:
    values: list[str] = []
    unsupported: list[str] = []
    for raw in split_semicolon(type_expr):
        part = raw.strip()
        if not part:
            continue
        if part.startswith("fn:"):
            unsupported.extend(sorted(set(PREDICATE_RE.findall(part))))
            # Arlington often uses fn:SinceVersion(...,dictionary); keep the
            # non-predicate type tokens that appear in the expression.
            for token in TYPE_MAP:
                if re.search(rf"\b{re.escape(token)}\b", part):
                    values.append(TYPE_MAP[token])
            continue
        mapped = TYPE_MAP.get(part)
        if mapped is None:
            values.append("Any")
            unsupported.append(f"unmapped-type:{part}")
        else:
            values.append(mapped)
    if not values:
        values.append("Any")
    return sorted(set(values), key=values.index), unsupported


def normalize_required(required: str) -> tuple[bool, list[str]]:
    if required == "TRUE":
        return True, []
    if required == "FALSE" or not required:
        return False, []
    predicates = sorted(set(PREDICATE_RE.findall(required)))
    return False, predicates or [required]


def normalize_indirect(policy: str) -> tuple[str, list[str]]:
    if policy == "TRUE":
        return "AllowsIndirect", []
    if policy == "FALSE" or not policy:
        return "Any", []
    predicates = sorted(set(PREDICATE_RE.findall(policy)))
    if "fn:MustBeDirect" in predicates:
        return "MustBeDirect", predicates
    if "fn:MustBeIndirect" in predicates:
        return "MustBeIndirect", predicates
    if "TRUE" in policy:
        return "AllowsIndirect", predicates
    return "Any", predicates or [policy]


def normalize_possible_names(value: str) -> tuple[list[str], list[str]]:
    if not value:
        return [], []
    if "fn:" in value or "@" in value:
        predicates = sorted(set(PREDICATE_RE.findall(value)))
        return [], predicates or [value]
    names: list[str] = []
    for match in NAME_VALUE_RE.findall(value):
        cleaned = match.strip().lstrip("@/")
        if cleaned and cleaned not in {"TRUE", "FALSE", "NULL"}:
            names.append(cleaned)
    return sorted(set(names), key=names.index), []


def unsupported_from_fields(row: dict[str, str], fields: Iterable[str]) -> list[str]:
    unsupported: list[str] = []
    for field in fields:
        value = row.get(field, "")
        if "fn:" in value:
            unsupported.extend(PREDICATE_RE.findall(value))
    return sorted(set(unsupported))


def find_tsv_root(arlington_root: Path) -> Path:
    candidates = [
        arlington_root / "tsv" / "latest",
        arlington_root / "tsv",
        arlington_root,
    ]
    for candidate in candidates:
        if candidate.exists() and list(candidate.glob("*.tsv")):
            return candidate
    raise SystemExit(f"no Arlington TSV files found under {arlington_root}")


def build_rules(tsv_root: Path) -> tuple[list[dict[str, object]], Stats]:
    paths = sorted(tsv_root.glob("*.tsv"))
    stats = Stats(source=str(tsv_root), commit="")
    stats.tsv_files = len(paths)
    stats.object_models = len(paths)
    rules: list[dict[str, object]] = []
    for path in paths:
        try:
            rows = read_rows(path)
        except Exception as exc:  # pragma: no cover - defensive developer tool
            stats.parse_warnings += 1
            print(f"warning: could not parse {path}: {exc}")
            continue

        for row in rows:
            key = row.get("Key", "").strip().lstrip("/")
            if not key:
                continue
            object_type = path.stem
            value_types, type_unsupported = normalize_types(row.get("Type", "any"))
            required, required_unsupported = normalize_required(row.get("Required", ""))
            indirect, indirect_unsupported = normalize_indirect(row.get("IndirectReference", ""))
            allowed_names, possible_unsupported = normalize_possible_names(
                row.get("PossibleValues", "")
            )
            link = row.get("Link", "").strip() or None
            since = row.get("SinceVersion", "").strip() or None
            deprecated = row.get("DeprecatedIn", "").strip() or None
            unsupported = sorted(
                set(
                    type_unsupported
                    + required_unsupported
                    + indirect_unsupported
                    + possible_unsupported
                    + unsupported_from_fields(
                        row,
                        [
                            "SpecialCase",
                            "DefaultValue",
                            "Inheritable",
                            "Link",
                            "Note",
                            "SinceVersion",
                            "DeprecatedIn",
                        ],
                    )
                )
            )

            rules.append(
                {
                    "object_type": object_type,
                    "key": key,
                    "required": required,
                    "value_types": value_types,
                    "allowed_names": allowed_names,
                    "since_version": since,
                    "deprecated_in": deprecated,
                    "link": link,
                    "indirect_policy": indirect,
                    "unsupported_predicates": unsupported,
                }
            )
            stats.keys += 1
            stats.required_key_rules += int(required)
            stats.type_rules += int(bool(value_types))
            stats.version_rules += int(bool(since or deprecated))
            stats.indirect_reference_rules += int(indirect != "Any")
            stats.link_rules += int(bool(link))
            stats.unsupported_predicates += len(unsupported)
            stats.parse_warnings += sum(1 for item in unsupported if item.startswith("unmapped-type:"))
    return rules, stats


def rust_array(values: list[str]) -> str:
    if not values:
        return "&[]"
    return "&[" + ", ".join(rust_str(value) for value in values) + "]"


def rust_type_array(values: list[str]) -> str:
    return (
        "&["
        + ", ".join(f"super::ArlingtonValueType::{value}" for value in values)
        + "]"
    )


def render_rust(rules: list[dict[str, object]], stats: Stats) -> str:
    lines = [
        "// @generated by scripts/generate_arlington_tables.py; do not edit by hand.",
        "",
        f"pub(super) const ARLINGTON_SOURCE: &str = {rust_str(stats.source)};",
        f"pub(super) const ARLINGTON_COMMIT: &str = {rust_str(stats.commit)};",
        f"pub(super) const ARLINGTON_TSV_FILES: usize = {stats.tsv_files};",
        f"pub(super) const ARLINGTON_OBJECT_MODELS: usize = {stats.object_models};",
        f"pub(super) const ARLINGTON_KEYS: usize = {stats.keys};",
        f"pub(super) const ARLINGTON_REQUIRED_KEY_RULES: usize = {stats.required_key_rules};",
        f"pub(super) const ARLINGTON_TYPE_RULES: usize = {stats.type_rules};",
        f"pub(super) const ARLINGTON_VERSION_RULES: usize = {stats.version_rules};",
        f"pub(super) const ARLINGTON_INDIRECT_REFERENCE_RULES: usize = {stats.indirect_reference_rules};",
        f"pub(super) const ARLINGTON_LINK_RULES: usize = {stats.link_rules};",
        f"pub(super) const ARLINGTON_UNSUPPORTED_PREDICATES: usize = {stats.unsupported_predicates};",
        f"pub(super) const ARLINGTON_PARSE_WARNINGS: usize = {stats.parse_warnings};",
        "",
        "pub(super) const ARLINGTON_RULES: &[super::ArlingtonRule] = &[",
    ]
    for rule in rules:
        lines.extend(
            [
                "    super::ArlingtonRule {",
                f"        object_type: {rust_str(str(rule['object_type']))},",
                f"        key: {rust_str(str(rule['key']))},",
                f"        required: {str(bool(rule['required'])).lower()},",
                f"        value_types: {rust_type_array(list(rule['value_types']))},",
                f"        allowed_names: {rust_array(list(rule['allowed_names']))},",
                f"        since_version: {option_str(rule['since_version'])},",
                f"        deprecated_in: {option_str(rule['deprecated_in'])},",
                f"        link: {option_str(rule['link'])},",
                f"        indirect_policy: super::ArlingtonIndirectPolicy::{rule['indirect_policy']},",
                f"        unsupported_predicates: {rust_array(list(rule['unsupported_predicates']))},",
                "    },",
            ]
        )
    lines.append("];")
    lines.append("")
    return "\n".join(lines)


def option_str(value: object) -> str:
    if not value:
        return "None"
    return f"Some({rust_str(str(value))})"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--arlington-root", type=Path, required=True)
    parser.add_argument("--commit", default="unknown")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--stats-json", type=Path)
    parser.add_argument(
        "--complete",
        action="store_true",
        help="Require a real upstream-sized Arlington TSV set, not a mock fixture.",
    )
    args = parser.parse_args()

    tsv_root = find_tsv_root(args.arlington_root)
    rules, stats = build_rules(tsv_root)
    stats.commit = args.commit
    stats.output = str(args.out)

    if args.complete and (stats.tsv_files < 100 or stats.keys < 1000):
        raise SystemExit(
            "Arlington complete mode requires real upstream TSVs; mock or seed data is insufficient"
        )

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(render_rust(rules, stats), encoding="utf-8", newline="\n")

    if args.stats_json:
        args.stats_json.parent.mkdir(parents=True, exist_ok=True)
        args.stats_json.write_text(
            json.dumps(asdict(stats), indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )

    print(
        f"generated {stats.keys} Arlington key rules from {stats.tsv_files} TSV files -> {args.out}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
