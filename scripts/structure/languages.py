#!/usr/bin/env python3
"""How each programming language writes comments, for the structure check.

Used by check_structure.py, doc_check.py and name_check.py. They look up a file's ending here to know how its
comments look, what may come before its opening note, and how its names are declared. It uses nothing else.

Main parts, in order: the Language record, the table of languages by file ending, language_for(), and
blank_strings(), which hides strings and comments so the name check only reads code.
"""
import re
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Language:
    """One language's comment marks and name family."""

    line_marks: tuple[str, ...]
    block: tuple[str, str] | None = None
    doc_line_marks: tuple[str, ...] = ()
    doc_block_start: str | None = None
    python_docstrings: bool = False
    head_skip: str = r"#!"
    family: str = "c_like"
    # A run of plain comment lines right above a line matching this is a doc comment too (Go's style).
    doc_before_declaration: str | None = None


HASH = ("#",)
SLASH = ("//",)
SLASH_BLOCK = ("/*", "*/")
JS_HEAD = r"#!|[\"']use \w+[\"'];?$|// @ts-|/// <reference"

LANGUAGES: dict[str, Language] = {
    ".py": Language(HASH, python_docstrings=True, head_skip=r"#!|#.*coding[:=]", family="python"),
    ".rb": Language(HASH, head_skip=r"#!|# frozen_string_literal", family="python"),
    ".sh": Language(HASH, family="shell"),
    ".bash": Language(HASH, family="shell"),
    ".zsh": Language(HASH, family="shell"),
    ".gd": Language(HASH, doc_line_marks=("##",), head_skip=r"@tool|@icon|@static_unload|class_name|extends",
                    family="gdscript"),
    ".gdshader": Language(SLASH, SLASH_BLOCK, head_skip=r"shader_type|render_mode"),
    ".gdshaderinc": Language(SLASH, SLASH_BLOCK),
    ".rs": Language(SLASH, SLASH_BLOCK, ("///", "//!"), "/**", head_skip=r"#!\[", family="rust"),
    ".ts": Language(SLASH, SLASH_BLOCK, (), "/**", head_skip=JS_HEAD, family="js"),
    ".tsx": Language(SLASH, SLASH_BLOCK, (), "/**", head_skip=JS_HEAD, family="js"),
    ".mts": Language(SLASH, SLASH_BLOCK, (), "/**", head_skip=JS_HEAD, family="js"),
    ".js": Language(SLASH, SLASH_BLOCK, (), "/**", head_skip=JS_HEAD, family="js"),
    ".jsx": Language(SLASH, SLASH_BLOCK, (), "/**", head_skip=JS_HEAD, family="js"),
    ".mjs": Language(SLASH, SLASH_BLOCK, (), "/**", head_skip=JS_HEAD, family="js"),
    ".cjs": Language(SLASH, SLASH_BLOCK, (), "/**", head_skip=JS_HEAD, family="js"),
    ".svelte": Language(SLASH, SLASH_BLOCK, (), "/**", head_skip=r"<script", family="js"),
    ".java": Language(SLASH, SLASH_BLOCK, (), "/**"),
    ".kt": Language(SLASH, SLASH_BLOCK, (), "/**", head_skip=r"@file:", family="kotlin"),
    ".kts": Language(SLASH, SLASH_BLOCK, (), "/**", family="kotlin"),
    ".scala": Language(SLASH, SLASH_BLOCK, (), "/**", family="kotlin"),
    ".swift": Language(SLASH, SLASH_BLOCK, ("///",), "/**", family="swift"),
    ".cs": Language(SLASH, SLASH_BLOCK, ("///",), "/**"),
    ".dart": Language(SLASH, SLASH_BLOCK, ("///",), "/**"),
    ".c": Language(SLASH, SLASH_BLOCK, (), "/**"),
    ".h": Language(SLASH, SLASH_BLOCK, (), "/**"),
    ".cpp": Language(SLASH, SLASH_BLOCK, ("///",), "/**"),
    ".hpp": Language(SLASH, SLASH_BLOCK, ("///",), "/**"),
    ".cc": Language(SLASH, SLASH_BLOCK, ("///",), "/**"),
    ".zig": Language(SLASH, None, ("///", "//!"), family="swift"),
    ".go": Language(SLASH, SLASH_BLOCK, head_skip=r"//go:build|// \+build", family="go",
                    doc_before_declaration=r"(?:func|type|var|const)\b"),
    ".lua": Language(("--",), ("--[[", "]]"), ("---",), family="lua"),
    ".sql": Language(("--",), SLASH_BLOCK, family="none"),
}

DOUBLE = r'"(?:[^"\\\n]|\\.)*"'
SINGLE = r"'(?:[^'\\\n]|\\.)*'"
STRING_KINDS = {
    "python": [r'"""[\s\S]*?"""', r"'''[\s\S]*?'''", DOUBLE, SINGLE],
    "gdscript": [r'"""[\s\S]*?"""', DOUBLE, SINGLE],
    # Rust and Swift use ' for lifetimes and single characters only, and Rust has raw strings: r#"..."#.
    "rust": [r'r(#*)"[\s\S]*?"\1', DOUBLE, r"'(?:\\.|[^\\'\n])'"],
    "swift": [r'"""[\s\S]*?"""', DOUBLE, r"'(?:\\.|[^\\'\n])'"],
    "js": [r"`(?:[^`\\]|\\.)*`", DOUBLE, SINGLE],
    "kotlin": [r'"""[\s\S]*?"""', DOUBLE, SINGLE],
    "go": [r"`[^`]*`", DOUBLE, SINGLE],
}


def language_for(path: Path) -> Language | None:
    """The language of a file, by its ending, or None when the check does not know it."""
    return LANGUAGES.get(path.suffix)


def blank_strings(text: str, language: Language) -> list[str]:
    """The file's lines with every string and comment turned into spaces, keeping the line breaks.

    Names inside strings and notes are not declarations, so the name check must not read them. Comments and
    strings are found in one pass from left to right, so a quote inside a comment never starts a string.
    """
    kinds = [re.escape(mark) + r"[^\n]*" for mark in language.line_marks]
    if language.block:
        kinds.insert(0, re.escape(language.block[0]) + r"[\s\S]*?" + re.escape(language.block[1]))
    kinds += STRING_KINDS.get(language.family, [DOUBLE, SINGLE])
    pattern = re.compile("|".join(f"(?:{kind})" for kind in kinds))

    def keep_breaks(found: re.Match) -> str:
        return re.sub(r"[^\n]", " ", found.group(0))

    return pattern.sub(keep_breaks, text).split("\n")
