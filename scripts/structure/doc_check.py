#!/usr/bin/env python3
"""The notes check: every code file opens with a note a newcomer can read, and doc comments are easy to read.

Used by check_structure.py for every code file. It uses languages.py to know how each language writes comments.
Normal style tools do not check these, so this check does, in every language it knows.

Main parts, in order: code_lines() (lines that count toward the length limit), opening_note() (the file's
opening note), doc_blocks() (every other doc comment), shape_problems() (the easy-to-read rules), and
doc_problems(), which puts them together for one file.
"""
import ast
import io
import re
import tokenize
from pathlib import Path

from languages import Language

LIST_ITEM = re.compile(r"^(?:[-*+]\s|\d+[.)]\s|@\w|:\w+:|\w+:\s*$)")
TRIPLE_QUOTES = ('"""', "'''")
NOT_CODE_TOKENS = {tokenize.COMMENT, tokenize.NL, tokenize.NEWLINE, tokenize.INDENT, tokenize.DEDENT,
                   tokenize.ENDMARKER}


def python_docstrings(text: str) -> list[tuple[int, int, str]] | None:
    """Every docstring in Python code as (first line, last line, raw text), lines from 0; None if it cannot parse."""
    try:
        tree = ast.parse(text)
    except (SyntaxError, ValueError):
        return None
    found = []
    for node in ast.walk(tree):
        if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)) and node.body:
            first = node.body[0]
            if isinstance(first, ast.Expr) and isinstance(first.value, ast.Constant) \
                    and isinstance(first.value.value, str):
                found.append((first.lineno - 1, first.end_lineno - 1, first.value.value))
    return sorted(found)


def _python_code_lines(text: str) -> int | None:
    """Python lines holding code, read with Python's own tokenizer; docstrings and comments are free."""
    docstrings = python_docstrings(text)
    if docstrings is None:
        return None
    free = {row for start, end, _ in docstrings for row in range(start, end + 1)}
    rows = set()
    try:
        for token in tokenize.generate_tokens(io.StringIO(text).readline):
            if token.type not in NOT_CODE_TOKENS:
                rows.update(range(token.start[0] - 1, token.end[0]))
    except (tokenize.TokenError, SyntaxError):
        return None
    return len(rows - free)


def code_lines(lines: list[str], language: Language) -> int:
    """How many lines hold code: blank lines, comment lines and docstrings are free, so notes never cost anything."""
    if language.python_docstrings:
        exact = _python_code_lines("\n".join(lines) + "\n")
        if exact is not None:
            return exact
    count = 0
    closing = None
    for line in lines:
        text = line.strip()
        if closing:
            if closing in text:
                text = text.split(closing, 1)[1].strip()
                closing = None
            else:
                continue
        if language.python_docstrings and text.startswith(TRIPLE_QUOTES):
            quote = text[:3]
            if text.count(quote) < 2:
                closing = quote
            continue
        if language.block and text.startswith(language.block[0]):
            rest = text[len(language.block[0]):]
            if language.block[1] in rest:
                text = rest.split(language.block[1], 1)[1].strip()
            else:
                closing = language.block[1]
                continue
        if text and not text.startswith(language.line_marks):
            count += 1
    return count


def _block_text(lines: list[str], start: int, closing: str, opening: str) -> tuple[list[str], int]:
    """The text inside a block comment or docstring that starts on line `start`, and the line after it ends."""
    first = lines[start].strip()[len(opening):]
    if closing in first:
        return [first.split(closing, 1)[0].strip()], start + 1
    text = [first.strip()] if first.strip() else []
    end = start + 1
    while end < len(lines) and closing not in lines[end]:
        text.append(lines[end])
        end += 1
    if end < len(lines):
        last = lines[end].split(closing, 1)[0]
        if last.strip():
            text.append(last)
    rest = text[1:] if first.strip() else text
    rest = [re.sub(r"^\s*\*(?: |$)", "", line) if opening.startswith("/*") else line for line in rest]
    return (text[:1] if first.strip() else []) + _dedent(rest), end + 1


def _dedent(lines: list[str]) -> list[str]:
    """Lines with their shared left margin taken off, so only extra indentation is left."""
    margins = [len(line) - len(line.lstrip()) for line in lines if line.strip()]
    cut = min(margins) if margins else 0
    return [line[cut:].rstrip() for line in lines]


def _line_run(lines: list[str], start: int, marks: tuple[str, ...]) -> tuple[list[str], int]:
    """The text of a run of comment lines starting with one of `marks`, and the line after the run."""
    longest = sorted(marks, key=len, reverse=True)
    text = []
    end = start
    while end < len(lines):
        stripped = lines[end].strip()
        mark = next((mark for mark in longest if stripped.startswith(mark)), None)
        if mark is None:
            break
        rest = stripped[len(mark):]
        if rest and not rest[0].isspace() and mark in ("#", "//", "--"):
            break
        text.append(rest[1:] if rest.startswith(" ") else rest)
        end += 1
    return [line.rstrip() for line in text], end


def opening_note(lines: list[str], language: Language) -> tuple[int, list[str]]:
    """The line number (from 0) and text of the file's opening note, or (-1, []) when it has none."""
    index = 0
    while index < len(lines) and (not lines[index].strip() or re.match(language.head_skip, lines[index].strip())):
        index += 1
    if index >= len(lines):
        return -1, []
    text = lines[index].strip()
    if language.python_docstrings and text.startswith(TRIPLE_QUOTES):
        return index, _block_text(lines, index, text[:3], text[:3])[0]
    if language.block and text.startswith(language.block[0]):
        opening = language.doc_block_start if language.doc_block_start and text.startswith(language.doc_block_start) else language.block[0]
        return index, _block_text(lines, index, language.block[1], opening)[0]
    marks = tuple(language.doc_line_marks) + tuple(language.line_marks)
    if text.startswith(marks):
        return index, _line_run(lines, index, marks)[0]
    return -1, []


def doc_blocks(lines: list[str], language: Language, skip_line: int) -> list[tuple[int, list[str]]]:
    """Every doc comment after the opening note: doc comment lines, /** blocks and Python docstrings."""
    if language.python_docstrings:
        docstrings = python_docstrings("\n".join(lines) + "\n")
        if docstrings is not None:
            return [(start, _docstring_lines(raw)) for start, _, raw in docstrings if start != skip_line]
    found = []
    index = 0
    while index < len(lines):
        text = lines[index].strip()
        start = index
        if index == skip_line:
            index = opening_end(lines, language, index)
        elif language.doc_line_marks and text.startswith(language.doc_line_marks):
            block, index = _line_run(lines, index, language.doc_line_marks)
            found.append((start, block))
        elif language.doc_block_start and text.startswith(language.doc_block_start) and not text.startswith("/**/"):
            block, index = _block_text(lines, index, "*/", language.doc_block_start)
            found.append((start, block))
        elif language.doc_before_declaration and text.startswith(language.line_marks):
            block, index = _line_run(lines, index, language.line_marks)
            if index < len(lines) and re.match(language.doc_before_declaration, lines[index].strip()):
                found.append((start, block))
        elif language.python_docstrings and text.startswith(TRIPLE_QUOTES) and _follows_definition(lines, index):
            block, index = _block_text(lines, index, text[:3], text[:3])
            found.append((start, block))
        # always move on, even when a comment line was too odd to read (like "#comment" with no space)
        index = max(index, start + 1)
    return found


def _docstring_lines(raw: str) -> list[str]:
    """A docstring's lines as written: the first as is, the rest without their shared margin."""
    lines = raw.split("\n")
    first = [lines[0].strip()] if lines[0].strip() else []
    rest = _dedent(lines[1:])
    if rest and not rest[-1].strip():
        rest = rest[:-1]
    return first + rest


def opening_end(lines: list[str], language: Language, index: int) -> int:
    """The line after the opening note that starts on line `index`."""
    text = lines[index].strip()
    if language.python_docstrings and text.startswith(TRIPLE_QUOTES):
        return _block_text(lines, index, text[:3], text[:3])[1]
    if language.block and text.startswith(language.block[0]):
        return _block_text(lines, index, language.block[1], language.block[0])[1]
    return _line_run(lines, index, tuple(language.doc_line_marks) + tuple(language.line_marks))[1]


def _follows_definition(lines: list[str], index: int) -> bool:
    """Whether the string on line `index` is a Python docstring: the first thing after a def or class line."""
    back = index - 1
    while back > 0 and not lines[back].strip():
        back -= 1
    if not lines[back].rstrip().endswith(":"):
        return False
    return any(re.match(r"\s*(?:async\s+)?(?:def|class)\b", lines[look])
               for look in range(back, max(-1, back - 8), -1))


def shape_problems(text: list[str], paragraph_lines: int) -> list[tuple[int, str]]:
    """What makes one note hard to read, as (line within the note, problem) pairs.

    A note of more than one paragraph (or longer than one paragraph's limit) starts with a one-line summary and an empty comment line. Each paragraph is
    at most `paragraph_lines` lines about one thing and ends where a sentence ends, never on "," or ";".
    List items, tags (@param), section names (Args:), indented lines and fenced code blocks are not
    paragraphs, so they have no length limit.
    """
    problems = []
    units: list[list] = []
    blank_before = False
    fenced = False
    for offset, line in enumerate(text):
        if line.strip().startswith("```"):
            fenced = not fenced
            units.append([offset, "code", [line], blank_before])
            blank_before = False
            continue
        if fenced:
            units[-1][2].append(line)
            continue
        if not line.strip():
            if offset == 0:
                problems.append((offset, "starts with an empty comment line"))
            elif blank_before:
                problems.append((offset, "has two empty comment lines in a row"))
            blank_before = True
            continue
        is_item = bool(LIST_ITEM.match(line)) or line.startswith((" ", "\t")) or (line.startswith("|") and line.endswith("|"))
        if is_item or not units or blank_before or units[-1][1] != "paragraph":
            if units and units[-1][1] == "item" and line.startswith((" ", "\t")) and not blank_before:
                units[-1][2].append(line)
            else:
                units.append([offset, "item" if is_item else "paragraph", [line], blank_before])
        else:
            units[-1][2].append(line)
        blank_before = False
    if text and not text[-1].strip() and len(text) > 1:
        problems.append((len(text) - 1, "ends with an empty comment line"))
    total = sum(1 for line in text if line.strip())
    for start, kind, lines, _ in units:
        if kind == "code":
            continue
        if kind == "paragraph" and len(lines) > paragraph_lines:
            problems.append((start, f"has a paragraph of {len(lines)} lines: split it into paragraphs of at most "
                                    f"{paragraph_lines} lines, each about one thing, with an empty comment line "
                                    "between them"))
        if kind == "paragraph" and lines[-1].rstrip().endswith((",", ";")):
            problems.append((start, 'has a paragraph that ends on "," or ";": end it where a sentence ends'))
    paragraphs = sum(1 for unit in units if unit[1] == "paragraph")
    if (paragraphs > 1 or total > paragraph_lines) and len(units) > 1 and units[0][1] == "paragraph":
        if len(units[0][2]) > 1 or not units[1][3]:
            problems.append((0, "does not start with a one-line summary followed by an empty comment line"))
    return problems


def doc_problems(path: Path, language: Language, min_note_lines: int, paragraph_lines: int) -> list[str]:
    """Everything wrong with one file's notes, one line per problem kind, with its first line numbers."""
    lines = path.read_text(errors="replace").splitlines()
    start, note = opening_note(lines, language)
    problems = []
    written = sum(1 for line in note if line.strip())
    if written < min_note_lines:
        problems.append(f"no opening note of {min_note_lines}+ lines saying what the file is for, what uses it, "
                        "what it uses and its main parts (rule: every file opens with a note)")
    by_kind: dict[str, list[int]] = {}
    blocks = ([(start, note)] if note else []) + doc_blocks(lines, language, start)
    for block_start, text in blocks:
        for offset, problem in shape_problems(text, paragraph_lines):
            by_kind.setdefault(problem, []).append(block_start + offset + 1)
    for problem, numbers in by_kind.items():
        shown = ", ".join(str(number) for number in numbers[:5]) + (" ..." if len(numbers) > 5 else "")
        problems.append(f"a doc comment {problem} (rule: notes are easy to read): {len(numbers)}x, line {shown}")
    return problems
