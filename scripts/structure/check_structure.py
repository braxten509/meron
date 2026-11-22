#!/usr/bin/env python3
"""The structure check: refuses a commit when any file breaks the project's STRUCTURE.md.

The git hook (.githooks/pre-commit) runs it with --hook before every commit; run it by hand with
`python3 scripts/structure/check_structure.py [paths...]`, where paths show only the problems under them.

It reads four blocks from STRUCTURE.md: the allowed paths, the settings, the style tool lines and the
long-file exceptions table. It also reads the old-files list, the files that do not follow the rules yet.
That list may only get shorter, and `--init-old-list` writes it once from today's failing files.

Main parts, in order: reading STRUCTURE.md, listing the project's files, the per-file rules (place, name,
length, opening note, doc comments and names, from doc_check.py and name_check.py), the style tool lines,
the old-files list rules, and main(), which prints each problem with how to fix it.
"""
import re
import shlex
import subprocess
import os
import sys
import tempfile
from datetime import date
from pathlib import Path

sys.dont_write_bytecode = True

from doc_check import code_lines, doc_problems  # noqa: E402  (after turning off the __pycache__ folder)
from languages import language_for  # noqa: E402
from name_check import DEFAULT_CUT_WORDS, DEFAULT_SHORT_WORDS, name_problems, name_words, word_problem  # noqa: E402

ROOT = Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True,
                           cwd=Path(__file__).resolve().parent).stdout.strip() or ".").resolve()
REPOSITORY = ROOT
DEFAULT_SETTINGS = {
    "max_code_lines": "600",
    "min_note_lines": "3",
    "paragraph_lines": "4",
    "old_list": "docs/structure_old_files.txt",
    "any_name": "",
    "dash_names": "",
    "capital_names": "**/*.md",
    "companion_endings": "",
    "skip_code_rules": "",
    "short_words": DEFAULT_SHORT_WORDS,
    "cut_words": DEFAULT_CUT_WORDS,
    "more_cut_words": "",
    "conventional_paths": "",
}
FILE_NAME = re.compile(r"^\.?[a-z0-9_]+(?:\.[a-z0-9_]+)*$")
TOOL_OUTPUT_LINES = 40


# ---- reading STRUCTURE.md ----

class Rules:
    """Everything the check reads from STRUCTURE.md."""

    def __init__(self) -> None:
        rules_file = ROOT / "STRUCTURE.md"
        if not rules_file.is_file():
            sys.exit("There is no STRUCTURE.md at the project root: the check cannot run.")
        text = rules_file.read_text()
        self.allowed = _block_lines(text, "allowed", required=True)
        self.settings = dict(DEFAULT_SETTINGS)
        for line in _block_lines(text, "settings"):
            key, _, value = line.partition(":")
            self.settings[key.strip()] = value.strip()
        self.style = [tuple(part.strip() for part in line.split("|", 1)) for line in _block_lines(text, "style")]
        self.exceptions, self.no_reason = _exception_rows(text)
        self.max_lines = int(self.settings["max_code_lines"])
        self.min_note = int(self.settings["min_note_lines"])
        self.paragraph = int(self.settings["paragraph_lines"])
        self.short_words = set(self.settings["short_words"].split())
        self.cut_words = set((self.settings["cut_words"] + " " + self.settings["more_cut_words"]).split())

    def patterns(self, key: str) -> list[str]:
        return self.settings[key].split()


def _block_lines(text: str, name: str, required: bool = False) -> list[str]:
    """The lines of a ```name block, without empty lines and # notes."""
    block = re.search(r"```" + name + r"\n(.*?)```", text, re.S)
    if block is None:
        if required:
            sys.exit(f"STRUCTURE.md has no ```{name} block: the check cannot run.")
        return []
    return [line.strip() for line in block.group(1).splitlines() if line.strip() and not line.strip().startswith("#")]


def _exception_rows(text: str) -> tuple[set[str], list[str]]:
    """The files in the Long-file exceptions table, and the rows that give no reason."""
    exceptions = set()
    no_reason = []
    table = re.search(r"## Long-file exceptions\n(.*?)(?:\n## |\Z)", text, re.S)
    for row in (table.group(1).splitlines() if table else []):
        cells = [cell.strip(" `") for cell in row.strip().strip("|").split("|")]
        if row.startswith("|") and len(cells) >= 2 and cells[0] not in ("File", "") and not cells[0].startswith("-"):
            exceptions.add(cells[0])
            if not cells[1]:
                no_reason.append(cells[0])
    return exceptions, no_reason


def pattern_matches(pattern: str, path: str) -> bool:
    """A STRUCTURE.md path pattern: `*` is any name inside one folder, `**` is any depth (`**/` may be none)."""
    regex = re.escape(pattern).replace(r"\*\*/", "\x01").replace(r"\*\*", "\x00").replace(r"\*", "[^/]*")
    regex = regex.replace("\x01", "(?:.*/)?").replace("\x00", ".*")
    return re.fullmatch(regex, path) is not None


def matches_any(patterns: list[str], path: str) -> bool:
    return any(pattern_matches(pattern, path) for pattern in patterns)


# ---- listing the project's files ----

def project_files() -> list[str]:
    """Every file git knows or would add (tracked, plus new ones not ignored) that is still on disk."""
    out = subprocess.run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
                         cwd=ROOT, capture_output=True, text=True, check=True).stdout
    return sorted({path for path in out.split("\0") if path and (ROOT / path).is_file()})


def staged_snapshot(directory: Path) -> list[str]:
    """Read the index into a temporary directory so partial commits keep their meaning."""
    entries = subprocess.check_output(["git", "ls-files", "--stage", "-z"], cwd=REPOSITORY)
    paths = []
    with subprocess.Popen(["git", "cat-file", "--batch"], cwd=REPOSITORY,
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE) as reader:
        for entry in entries.split(b"\0"):
            if not entry:
                continue
            metadata, raw_path = entry.split(b"\t", 1)
            mode, object_id, stage = metadata.split()
            if stage != b"0":
                sys.exit("Resolve unmerged paths before running the staged check.")
            if mode not in (b"100644", b"100755"):
                continue
            path = os.fsdecode(raw_path)
            target = directory / path
            reader.stdin.write(object_id + b"\n")
            reader.stdin.flush()
            _, kind, size = reader.stdout.readline().split()
            if kind != b"blob":
                raise ValueError(f"Expected file contents for {path}")
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(reader.stdout.read(int(size)))
            if reader.stdout.read(1) != b"\n":
                raise ValueError(f"Incomplete Git response for {path}")
            paths.append(path)
        reader.stdin.close()
    return paths


def read_old_list(rules: Rules, from_commit: bool, revision: str = "HEAD") -> set[str] | None:
    """The old-files list, from the working tree or from the last commit (None when it is not there)."""
    name = rules.settings["old_list"]
    if from_commit:
        shown = subprocess.run(["git", "show", revision + ":" + name], cwd=REPOSITORY, capture_output=True, text=True)
        if shown.returncode != 0:
            return None
        text = shown.stdout
    elif (ROOT / name).is_file():
        text = (ROOT / name).read_text()
    else:
        return None
    return {line.strip() for line in text.splitlines() if line.strip() and not line.startswith("#")}


# ---- the per-file rules ----

def own_file(path: str, rules: Rules) -> str:
    """A companion file (like Godot's .uid or .import) follows the rules of the file it belongs to."""
    for ending in rules.patterns("companion_endings"):
        if path.endswith(ending):
            return path[: -len(ending)]
    return path


def is_kotlin_state_extension(path: str, component: str) -> bool:
    """Meron groups extensions on the shared mobile state as Receiver+Feature.kt."""
    return path.endswith(".kt") and bool(re.fullmatch(r"MeronMobileState\+[A-Z][A-Za-z0-9]+\.kt", component))


def name_problem(path: str, rules: Rules) -> str | None:
    """What is wrong with a file's name or its folders' names, if anything (rule: names are plain words)."""
    if matches_any(rules.patterns("any_name"), path):
        return None
    may_dash = matches_any(rules.patterns("dash_names"), path)
    may_capital = matches_any(rules.patterns("capital_names"), path)
    parts = path.split("/")
    for index, part in enumerate(parts):
        if part in rules.patterns("conventional_paths"):
            continue
        if is_kotlin_state_extension(path, part):
            part = part.replace("+", "_")
        checked = part.replace("-", "_") if may_dash else part
        checked = checked.lower() if may_capital else checked
        kind = "folder" if index < len(parts) - 1 else "file"
        if not FILE_NAME.match(checked):
            return (f"{kind} name '{part}' is not plain lower-case words joined by _ (rule: names are plain words; "
                    "STRUCTURE.md settings dash_names / capital_names allow - or capitals in set places)")
        pieces = part.split(".")
        pieces = pieces[:-1] if kind == "file" and len(pieces) > 1 else pieces
        words = [word for piece in pieces for word in name_words(piece)]
        # one-letter words are never allowed in file names (old group letters like a_, c1_), whatever short_words says
        bad = [word for word in words if word_problem(word, set(), rules.cut_words)]
        if bad:
            return (f"{kind} name '{part}' has a short or cut-down word ({', '.join(bad)}): name it in whole words "
                    "(rule: names are plain words)")
    return None


def file_problems(path: str, rules: Rules) -> list[str]:
    """Everything wrong with one file (its own file for companion files)."""
    base = own_file(path, rules)
    problems = []
    if not matches_any(rules.allowed, base):
        problems.append("this place is not in STRUCTURE.md's allowed paths (rule: one place for each thing): "
                        "move it, or add its pattern to the allowed block in the same commit")
    named = name_problem(base, rules)
    if named:
        problems.append(named)
    full = ROOT / path
    language = language_for(full)
    if base != path or language is None or matches_any(rules.patterns("skip_code_rules"), path):
        return problems
    length = code_lines(full.read_text(errors="replace").splitlines(), language)
    if length > rules.max_lines and path not in rules.exceptions:
        problems.append(f"{length} code lines, over {rules.max_lines} (rule: short files): split it by job, or "
                        "add a row with its reason to STRUCTURE.md's Long-file exceptions table")
    problems += doc_problems(full, language, rules.min_note, rules.paragraph)
    problems += name_problems(full, language, rules.short_words, rules.cut_words)
    return problems


# ---- the style tool lines ----

def style_problems(rules: Rules, files: list[str], file_lines_only: bool = False) -> list[str]:
    """Run each style tool line (`pattern | command`) on its matching files; one problem per failing line.

    `{files}` in the command becomes the matching files. A command without it runs once for the whole project
    when any file matches (skipped when `file_lines_only` is set, as when judging one old file).
    """
    problems = []
    for pattern, command in rules.style:
        matching = [path for path in files if pattern_matches(pattern, path)]
        if not matching or ("{files}" not in command and file_lines_only):
            continue
        line = f"{pattern} | {command}"
        if "{files}" in command:
            command = command.replace("{files}", " ".join(shlex.quote(path) for path in matching))
        result = subprocess.run(command, shell=True, cwd=ROOT, capture_output=True, text=True)
        if result.returncode == 127:
            problems.append(f"style tool line '{line}': the tool is not installed. Install it, "
                            "or take the line out of STRUCTURE.md's style block")
        elif result.returncode != 0:
            output = (result.stdout + result.stderr).strip().splitlines()
            shown = "\n      ".join(output[:TOOL_OUTPUT_LINES])
            more = f"\n      ... {len(output) - TOOL_OUTPUT_LINES} more lines" if len(output) > TOOL_OUTPUT_LINES else ""
            problems.append(f"style tool line '{pattern}' failed (rule: the style tools pass):\n      {shown}{more}")
    return problems


# ---- the old-files list and main ----

def init_old_list(rules: Rules, failing: dict[str, list[str]]) -> int:
    """Write the old-files list from today's failing files; refused once the list is in a commit."""
    name = rules.settings["old_list"]
    if read_old_list(rules, from_commit=True) is not None:
        sys.exit(f"{name} is already in a commit: it may only get shorter, never be made again.")
    failing_style = [path for path in project_files() if path not in failing and own_file(path, rules) == path
                     and style_problems(rules, [path], file_lines_only=True)]
    listed = sorted({own_file(path, rules) for path in list(failing) + failing_style})
    header = (f"# Files that do not follow STRUCTURE.md yet (made {date.today()}). This list may only get shorter:\n"
              "# take a file off when it is moved, split or fixed. Never add a line.\n")
    (ROOT / name).parent.mkdir(parents=True, exist_ok=True)
    (ROOT / name).write_text(header + "".join(path + "\n" for path in listed))
    print(f"wrote {name} with {len(listed)} files")
    return 0


def list_problems(rules: Rules, files: list[str], failing: dict[str, list[str]]) -> list[str]:
    """The old-files list rules: it only shrinks, it lists real files, and a fixed file comes off it."""
    name = rules.settings["old_list"]
    errors = []
    old_now = read_old_list(rules, from_commit=False) or set()
    old_before = read_old_list(rules, from_commit=True, revision=os.getenv("MERON_STRUCTURE_BASE") or "HEAD")
    if old_before is not None:
        for added in sorted(old_now - old_before):
            errors.append(f"{name}: '{added}' was added. The list may only get shorter; fix the file instead.")
    on_disk = set(files)
    for listed in sorted(old_now - on_disk):
        errors.append(f"{name}: '{listed}' is gone (moved or deleted). Take it off the list.")
    for listed in sorted(old_now & on_disk):
        if listed not in failing and not style_problems(rules, [listed], file_lines_only=True):
            errors.append(f"{name}: '{listed}' follows the rules now. Take it off the list.")
    return errors


def table_problems(rules: Rules, files: list[str]) -> list[str]:
    """The Long-file exceptions table only holds real, long files, each with its reason."""
    errors = [f"STRUCTURE.md: the Long-file exception for '{row}' has no reason. Write why it is allowed."
              for row in rules.no_reason]
    for row in sorted(rules.exceptions):
        if row not in files:
            errors.append(f"STRUCTURE.md: the Long-file exception '{row}' is not a file any more. Take its row off.")
        elif (language := language_for(ROOT / row)) and \
                code_lines((ROOT / row).read_text(errors="replace").splitlines(), language) <= rules.max_lines:
            errors.append(f"STRUCTURE.md: '{row}' is under {rules.max_lines} code lines now. Take its row off.")
    return errors


def check(files: list[str]) -> int:
    rules = Rules()
    shown_paths = [arg.rstrip("/") for arg in sys.argv[1:] if not arg.startswith("--")]
    failing = {}
    for path in files:
        problems = file_problems(path, rules)
        if problems:
            failing[path] = problems
    if "--init-old-list" in sys.argv:
        return init_old_list(rules, failing)
    old_now = read_old_list(rules, from_commit=False) or set()
    errors = table_problems(rules, files) + list_problems(rules, files, failing)
    for path, problems in failing.items():
        if own_file(path, rules) in old_now:
            continue
        if shown_paths and not any(path == shown or path.startswith(shown + "/") for shown in shown_paths):
            continue
        errors += [f"{path}: {problem}" for problem in problems]
    errors += style_problems(rules, [path for path in files if own_file(path, rules) not in old_now])

    if errors:
        print("STRUCTURE CHECK FAILED. Read STRUCTURE.md; to change the structure, change STRUCTURE.md first.")
        for error in errors:
            print("  " + error)
        return 1
    print(f"structure check passed ({len(files)} files, {len(old_now)} old files still to fix)")
    return 0


def main() -> int:
    global ROOT
    if "--hook" in sys.argv:
        with tempfile.TemporaryDirectory(prefix="meron-staged-") as directory:
            ROOT = Path(directory)
            return check(staged_snapshot(ROOT))
    return check(project_files())


if __name__ == "__main__":
    sys.exit(main())
