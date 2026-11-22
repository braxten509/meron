#!/usr/bin/env python3
"""The names check: every name a file makes (variables, functions, parameters, loop names) is whole words.

Used by check_structure.py for every code file, and for file and folder names. It uses languages.py to hide
strings and comments, so it reads only real declarations. Normal style tools do not check this.

Main parts, in order: the default short and cut-down words, name_words() (splits a name into its words),
word_problem() (judges one word), the declaration patterns per language family, declared_names() (finds the
names a file makes), and name_problems(), which lists the bad ones with their line numbers.
"""
import re
from pathlib import Path

from languages import Language, blank_strings

DEFAULT_SHORT_WORDS = "i j x y z w h _"
DEFAULT_CUT_WORDS = ("tmp val obj arr num cnt idx str msg btn cfg conf ctx req res resp err elem el evt ev cb fn "
                     "ptr buf prev cur curr src dst mgr util lbl img txt pos dir calc mult")
SHOWN_NAMES = 8
IDENTIFIER = r"[A-Za-z_]\w*"
NAME_FIRST = {"python", "gdscript", "rust", "js", "go", "kotlin", "swift", "lua", "shell"}

# Each pattern finds declarations; group "names" holds one name or a comma list, "params" a parameter list.
COMMON = [
    re.compile(r"\b(?:def|fn|func|fun|function)\s+(?P<names>" + IDENTIFIER
               + r")\s*(?:<[^>]*>)?\s*\((?P<params>[^)]*)"),
    re.compile(r"\bfor\s*\(?\s*(?:const\s+|let\s+|var\s+|val\s+|mut\s+)?(?P<names>\w+(?:\s*,\s*\w+)*)"
               r"\s*(?::\s*[\w.\[\]<>]+)?\s+(?:in|of)\b"),
]
FAMILY_PATTERNS = {
    "python": [
        # an assignment at the start of a line (lines inside brackets, like keyword arguments, are skipped)
        re.compile(r"^\s*(?P<names>" + IDENTIFIER + r"(?:\s*,\s*" + IDENTIFIER + r")*)\s*(?::[^=]+)?=[^=]"),
        re.compile(r"\b(?:as|except\s+\w+\s+as)\s+(?P<names>" + IDENTIFIER + r")\s*[:,)]"),
        re.compile(r"\blambda\s+(?P<params>[^:]*):"),
    ],
    "gdscript": [
        re.compile(r"\b(?:var|const)\s+(?P<names>" + IDENTIFIER + ")"),
        re.compile(r"\bfunc\s*\((?P<params>[^)]*)\)"),
    ],
    "rust": [
        # not the const in a *const pointer type, nor the static in a 'static lifetime
        # Constructor and path patterns reference existing Rust names.
        re.compile(r"(?<![*'])\b(?:let|const|static)\s+(?:mut\s+)?(?P<names>" + IDENTIFIER + r")\b(?!\s*(?:\(|::))"),
        re.compile(r"\blet\s+(?:mut\s+)?\((?P<names>[^)]*)\)"),
        re.compile(r"(?:^|[=(,\s])(?:move\s+)?\|(?P<params>[^|]*)\|"),
    ],
    "js": [
        re.compile(r"\b(?:const|let|var)\s+(?P<names>" + IDENTIFIER + ")"),
        re.compile(r"\b(?:const|let|var)\s+[\[{](?P<names>[^\]}]*)[\]}]\s*="),
        re.compile(r"\((?P<params>[^()]*)\)\s*(?::\s*[\w<>\[\]|. ]+)?\s*=>"),
        re.compile(r"(?:^|[=(,\s])(?P<names>" + IDENTIFIER + r")\s*=>"),
    ],
    "go": [
        re.compile(r"^\s*(?P<names>" + IDENTIFIER + r"(?:\s*,\s*" + IDENTIFIER + r")*)\s*:="),
        re.compile(r"\b(?:var|const)\s+(?P<names>" + IDENTIFIER + ")"),
        # a method: func (receiver Thing) name(params)
        re.compile(r"\bfunc\s*\((?P<params>[^)]*)\)\s*\w+\s*\("),
        re.compile(r"\bfunc\s*\([^)]*\)\s*(?P<names>" + IDENTIFIER + r")\s*\((?P<params>[^)]*)"),
    ],
    "kotlin": [
        re.compile(r"\b(?:val|var)\s+(?P<names>" + IDENTIFIER + ")"),
        re.compile(r"\{\s*(?P<names>" + IDENTIFIER + r"(?:\s*,\s*" + IDENTIFIER + r")*)\s*->"),
    ],
    "swift": [
        re.compile(r"\b(?:let|var|const)\s+(?P<names>" + IDENTIFIER + ")"),
        re.compile(r"\{\s*(?P<names>" + IDENTIFIER + r"(?:\s*,\s*" + IDENTIFIER + r")*)\s+in\b"),
    ],
    "lua": [re.compile(r"\blocal\s+(?P<names>" + IDENTIFIER + r"(?:\s*,\s*" + IDENTIFIER + r")*)")],
    "shell": [
        re.compile(r"^\s*(?:local\s+|declare\s+(?:-\w+\s+)?|export\s+|readonly\s+)?(?P<names>" + IDENTIFIER
                   + r")\+?="),
        re.compile(r"^\s*(?:function\s+)?(?P<names>" + IDENTIFIER + r")\s*\(\)"),
    ],
    "c_like": [
        re.compile(r"\b(?:int|float|double|bool|boolean|char|auto|var|val|let|string|String|long|short|size_t|"
                   r"usize|u8|u32|i32|i64|f32|f64)\s+(?P<names>" + IDENTIFIER + r")\s*[=;,):]"),
    ],
    "none": [],
}
SKIPPED_NAMES = {"self", "cls", "this", "super", "mut", "const", "let", "var", "val", "in", "of", "new"}


def name_words(name: str) -> list[str]:
    """The words of a name, in lower case: snake_case, kebab-case, camelCase and CONSTANT_CASE all split."""
    spaced = re.sub(r"([a-z0-9])([A-Z])", r"\1 \2", name.replace("OAuth", "Oauth"))
    spaced = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1 \2", spaced)
    return [word.lower() for word in re.split(r"[\s_\-.]+", spaced) if word and not word.isdigit()]


def word_problem(word: str, short_words: set[str], cut_words: set[str]) -> bool:
    """Whether one word of a name is too short or a cut-down word (like msg for message)."""
    plain = word.rstrip("0123456789")
    if not plain or plain in short_words:
        return False
    if len(plain) == 1:
        return True
    return plain in cut_words or (plain.endswith("s") and plain[:-1] in cut_words)


def _split_params(params: str, name_first: bool) -> list[str]:
    """The parameter names in a parameter list, skipping types, default values and patterns."""
    names = []
    depth = 0
    piece = ""
    for character in params + ",":
        if character in "([{<":
            depth += 1
        elif character in ")]}>":
            depth -= 1
        if character == "," and depth <= 0:
            piece = re.split(r"=(?!>)", piece, maxsplit=1)[0].strip().lstrip("*&").strip()
            piece = re.sub(r"^(?:mut|ref|const|final|readonly|public|private|vararg|inout|out)\s+", "", piece)
            words = re.findall(IDENTIFIER, re.split(r":", piece, maxsplit=1)[0] if name_first else piece)
            if words and not piece.startswith(("{", "[", "(")):
                names.append(words[0] if name_first else words[-1])
            piece = ""
        else:
            piece += character
    return names


def _clean_name(piece: str) -> str:
    """One name out of a declared list piece: `mut a`, `...rest`, `key: local` and `a = 1` all give the name."""
    piece = re.split(r"=(?!>)", piece, maxsplit=1)[0]
    if ":" in piece:
        piece = piece.split(":", 1)[1]
    piece = re.sub(r"^\s*(?:mut|ref)\s+", "", piece).strip().lstrip(".&*").strip()
    found = re.match(IDENTIFIER, piece)
    return found.group(0) if found else ""


def _joined_lines(lines: list[str]) -> list[tuple[int, str, int]]:
    """Each line as (number, text, bracket depth at its start); a signature over several lines is joined."""
    joined = []
    depth = 0
    index = 0
    while index < len(lines):
        text = lines[index]
        start_depth = depth
        end = index
        opens = re.search(r"\b(?:def|fn|func|fun|function)\b[^(]*\(", text)
        while opens and text.count("(") > text.count(")") and end + 1 < len(lines) and end - index < 30:
            end += 1
            text += " " + lines[end].strip()
        for line in lines[index:end + 1]:
            depth = max(0, depth + sum(line.count(mark) for mark in "([{") - sum(line.count(mark) for mark in ")]}"))
        joined.append((index + 1, text, start_depth))
        index = end + 1
    return joined


def declared_names(path: Path, language: Language) -> list[tuple[int, str]]:
    """Every name the file makes, as (line number, name) pairs."""
    if language.family == "none":
        return []
    lines = blank_strings(path.read_text(errors="replace"), language)
    patterns = COMMON + FAMILY_PATTERNS.get(language.family, [])
    name_first = language.family in NAME_FIRST
    found = []
    for number, line, depth in _joined_lines(lines):
        for pattern in patterns:
            if depth > 0 and pattern.pattern.startswith("^"):
                continue
            for match in pattern.finditer(line):
                groups = match.groupdict()
                if groups.get("names"):
                    found += [(number, _clean_name(name)) for name in groups["names"].split(",")]
                if groups.get("params"):
                    found += [(number, name) for name in _split_params(groups["params"], name_first)]
    return [(number, name) for number, name in found if name and name not in SKIPPED_NAMES]


def name_problems(path: Path, language: Language, short_words: set[str], cut_words: set[str]) -> list[str]:
    """The names in one file that are not whole words, as one problem line (or none)."""
    bad = []
    seen = set()
    for number, name in declared_names(path, language):
        if name in seen:
            continue
        # Keep standard conversion APIs and the environment key read by Apple's SDK tools.
        if (language.family, name) in {("rust", "as_str"), ("shell", "DEVELOPER_DIR")}:
            continue
        words = name_words(name)
        if language.family == "rust":
            words = [word for word in words if not re.fullmatch(r"[iu](?:8|16|32|64|128)", word)]
        if any(word_problem(word, short_words, cut_words) for word in words):
            seen.add(name)
            bad.append(f"'{name}' line {number}")
    if not bad:
        return []
    shown = ", ".join(bad[:SHOWN_NAMES]) + (f" and {len(bad) - SHOWN_NAMES} more" if len(bad) > SHOWN_NAMES else "")
    return [f"names that are not whole words (rule: names are whole words; write player_state, not s or ps): {shown}"]
