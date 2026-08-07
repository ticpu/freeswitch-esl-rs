#!/usr/bin/env python3
"""Verify FreeSWITCH source line references against the pinned commit.

Every `file.c:NNN` reference in a doc comment or markdown file indexes a
specific FreeSWITCH tree. This checks each one against the commit pinned in
hooks/source-refs.yaml by comparing a content hash of the referenced line or
range, so bumping the pin produces a diff naming exactly the references whose
target text moved.

Local only -- it needs a FreeSWITCH clone at $FREESWITCH_SOURCE containing the
pinned commit, which CI does not have.

    hooks/check-source-refs.py            verify
    hooks/check-source-refs.py --update   regenerate the index
"""

import argparse
import hashlib
import os
import re
import subprocess
import sys
from pathlib import Path

import yaml

INDEX = "hooks/source-refs.yaml"
HASH_LEN = 12

# Only comments carry references; a C file:line inside a Rust string literal is
# test data (a synthetic FreeSWITCH log line), not a citation.
RUST_COMMENT = re.compile(r"^\s*(///|//!|//)")

REF = re.compile(
    r"`?(?P<file>[A-Za-z0-9_]+\.[ch]):(?P<start>\d+)(?:-(?P<end>\d+))?"
    r"(?P<more>(?:,\s*\d+(?:-\d+)?)+)?"
)
# A bare `:NNN` continues the last file named earlier in the same document.
CONT = re.compile(r"`:(?P<start>\d+)(?:-(?P<end>\d+))?`")
COMMIT = re.compile(r"\b[0-9a-f]{40}\b")


def fail(msg: str) -> None:
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(1)


class FsTree:
    """Reads blobs out of the FreeSWITCH repo at one pinned commit."""

    def __init__(self, repo: Path, commit: str):
        self.repo = repo
        self.commit = commit
        self._lines: dict[str, list[str]] = {}
        self._paths: dict[str, str] | None = None

    def _git(self, *args: str) -> str:
        return subprocess.run(
            ["git", "--git-dir", str(self.repo / ".git"), *args],
            check=True,
            capture_output=True,
            text=True,
        ).stdout

    def path_for(self, basename: str) -> str:
        if self._paths is None:
            self._paths = {}
            candidates: dict[str, list[str]] = {}
            for p in self._git("ls-tree", "-r", "--name-only", self.commit).splitlines():
                candidates.setdefault(p.rsplit("/", 1)[-1], []).append(p)
            for name, paths in candidates.items():
                # tests/unit/ shadows several core basenames
                real = [p for p in paths if not p.startswith("tests/")]
                if len(real) == 1:
                    self._paths[name] = real[0]
                elif real:
                    self._paths[name] = "\0".join(real)
        found = self._paths.get(basename)
        if found is None:
            fail(f"{basename} is not in FreeSWITCH {self.commit[:10]}")
        if "\0" in found:
            fail(f"{basename} is ambiguous: {found.replace(chr(0), ', ')}")
        return found

    def lines(self, path: str) -> list[str]:
        if path not in self._lines:
            self._lines[path] = self._git("show", f"{self.commit}:{path}").split("\n")
        return self._lines[path]

    def digest(self, path: str, start: int, end: int) -> str:
        lines = self.lines(path)
        if end > len(lines):
            fail(f"{path}:{start}-{end} runs past EOF ({len(lines)} lines)")
        body = "\n".join(lines[start - 1 : end])
        return hashlib.sha256(body.encode()).hexdigest()[:HASH_LEN]


def scanned_files(repo: Path) -> list[Path]:
    out = subprocess.run(
        ["git", "ls-files", "-z", "*.md", "*.rs"],
        cwd=repo,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return [repo / p for p in out.split("\0") if p]


def extract(repo: Path) -> tuple[dict[tuple[str, int, int], set[str]], set[str]]:
    """Map (basename, start, end) -> citing repo files, plus every pinned commit."""
    refs: dict[tuple[str, int, int], set[str]] = {}
    commits: set[str] = set()
    for path in scanned_files(repo):
        rel = path.relative_to(repo).as_posix()
        is_rust = path.suffix == ".rs"
        last_file: str | None = None
        for lineno, line in enumerate(path.read_text().splitlines(), 1):
            commits.update(COMMIT.findall(line))
            if is_rust and not RUST_COMMENT.match(line):
                continue
            for m in _line_refs(line, last_file):
                if isinstance(m, str):
                    last_file = m
                    continue
                refs.setdefault(m, set()).add(f"{rel}:{lineno}")
    return refs, commits


def _line_refs(line: str, last_file: str | None):
    """Yield refs in source order, and the basename each time one is named."""
    events: list[tuple[int, object]] = []
    for m in REF.finditer(line):
        events.append((m.start(), ("file", m)))
    for m in CONT.finditer(line):
        events.append((m.start(), ("cont", m)))
    for _, (kind, m) in sorted(events, key=lambda e: e[0]):
        if kind == "file":
            name = m.group("file")
            yield name
            last_file = name
            start = int(m.group("start"))
            end = int(m.group("end") or start)
            yield (name, start, end)
            for extra in re.findall(r"(\d+)(?:-(\d+))?", m.group("more") or ""):
                s = int(extra[0])
                yield (name, s, int(extra[1] or s))
        elif last_file:
            start = int(m.group("start"))
            yield (last_file, start, int(m.group("end") or start))


def ref_key(basename: str, start: int, end: int, tree: FsTree) -> str:
    path = tree.path_for(basename)
    return f"{path}:{start}" if start == end else f"{path}:{start}-{end}"


def load_index(repo: Path) -> tuple[str, str, dict[str, str]]:
    path = repo / INDEX
    if not path.exists():
        fail(f"{INDEX} is missing; seed it with --update --commit SHA --tag TAG")
    doc = yaml.safe_load(path.read_text())
    entries = {}
    for entry in doc["refs"]:
        ref, digest = entry.rsplit(" ", 1)
        entries[ref] = digest
    return doc["freeswitch"]["commit"], doc["freeswitch"]["tag"], entries


def write_index(repo: Path, commit: str, tag: str, entries: dict[str, str]) -> None:
    def order(ref: str) -> tuple[str, int]:
        path, lines = ref.rsplit(":", 1)
        return path, int(lines.split("-")[0])

    body = "\n".join(f"- {ref} {entries[ref]}" for ref in sorted(entries, key=order))
    (repo / INDEX).write_text(
        "# Generated by hooks/check-source-refs.py --update. Do not edit by hand,\n"
        "# except to bump the commit below and then regenerate.\n"
        f"freeswitch:\n  commit: {commit}\n  tag: {tag}\nrefs:\n{body}\n"
    )


def open_tree(commit: str) -> FsTree:
    source = os.environ.get("FREESWITCH_SOURCE")
    if not source:
        print(
            "skipped: FREESWITCH_SOURCE is unset, so the pinned commit cannot be read",
            file=sys.stderr,
        )
        sys.exit(0)
    repo = Path(source)
    if not (repo / ".git").exists():
        fail(f"FREESWITCH_SOURCE={source} is not a git repository")
    have = subprocess.run(
        ["git", "--git-dir", str(repo / ".git"), "cat-file", "-e", f"{commit}^{{commit}}"],
        capture_output=True,
    )
    if have.returncode != 0:
        fail(
            f"FREESWITCH_SOURCE={source} has no commit {commit[:10]}; "
            "run: git fetch --tags"
        )
    return FsTree(repo, commit)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--update", action="store_true", help="regenerate the index")
    ap.add_argument("--quiet", action="store_true", help="print only on failure")
    ap.add_argument("--commit", help="pin to seed a missing index with")
    ap.add_argument("--tag", help="tag naming --commit")
    ap.add_argument(
        "--list",
        action="store_true",
        help="print every citation and the reference it resolves to, for review",
    )
    args = ap.parse_args()

    repo = Path(__file__).resolve().parent.parent
    if args.commit and not (repo / INDEX).exists():
        commit, tag, indexed = args.commit, args.tag, {}
    else:
        commit, tag, indexed = load_index(repo)
    tree = open_tree(commit)

    refs, commits = extract(repo)
    seen = {ref_key(*k, tree): v for k, v in refs.items()}

    if args.list:
        for ref, cites in sorted(seen.items(), key=lambda kv: sorted(kv[1])):
            for cite in sorted(cites):
                print(f"{cite}\t{ref}")
        return 0

    digests = {ref: tree.digest(*_split(ref)) for ref in seen}

    if args.update:
        write_index(repo, commit, tag, digests)
        print(f"SourceRefs {len(digests)} refs written to {INDEX} @ {commit[:10]}")
        return 0

    problems: list[str] = []
    for other in sorted(commits - {commit}):
        problems.append(f"pin {other[:10]} disagrees with the index ({commit[:10]})")
    for ref in sorted(set(indexed) - set(digests)):
        problems.append(f"{ref} is indexed but no longer cited")
    for ref in sorted(set(digests) - set(indexed)):
        problems.append(f"{ref} is cited by {', '.join(sorted(seen[ref]))} but not indexed")
    for ref in sorted(set(digests) & set(indexed)):
        if digests[ref] != indexed[ref]:
            problems.append(
                f"{ref} changed at {commit[:10]}: indexed {indexed[ref]}, "
                f"now {digests[ref]} (cited by {', '.join(sorted(seen[ref]))})"
            )

    if problems:
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        print(
            f"❌ {len(problems)} source reference problem(s). "
            "Re-verify each against the pinned tree, then run --update.",
            file=sys.stderr,
        )
        return 1

    if not args.quiet:
        files = {c.rsplit(":", 1)[0] for cites in seen.values() for c in cites}
        print(f"SourceRefs {len(digests)} refs / {len(files)} files @ {tag} ok")
    return 0


def _split(ref: str) -> tuple[str, int, int]:
    path, lines = ref.rsplit(":", 1)
    start, _, end = lines.partition("-")
    return path, int(start), int(end or start)


if __name__ == "__main__":
    sys.exit(main())
